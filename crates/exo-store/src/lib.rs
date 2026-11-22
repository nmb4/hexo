//! The offline-first HEY data layer.
//!
//! [`Store`] owns one SQLite cache per account and a [`hey_sdk::Client`] for everything the
//! cache cannot answer. Reads the UI makes are served from SQLite and refreshed in the
//! background; writes go straight to HEY and update the cache on success. This keeps the
//! three-pane view instant even on a slow connection.

pub mod model;

use std::{path::Path, sync::Mutex};

use exo_auth::TokenProvider;
use hey_sdk::{
    Client, Config,
    models::{
        Contact, Entry, Mailbox, MessageDraft, Posting, Topic,
    },
    services::{
        BoxKind, ClearanceStatus, MessageContent, ReplyContent, ScreenOptions, SearchParams,
    },
};
use rusqlite::{Connection, OptionalExtension, params};

pub use model::{
    BoxInfo, Identity, Label, MessageView, ScreenerItem, SearchHit, Thread, ThreadRow,
};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("HEY error: {0}")]
    Hey(#[from] hey_sdk::Error),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

/// Resolve the thread a posting opens. HEY's posting JSON has no `topic_id` field: the
/// thread lives in `app_url` (or `app_bundle_url` for a single-thread bundle). This is the
/// same resolution `hey-cli` performs.
pub fn topic_id_from_posting(posting: &Posting) -> i64 {
    topic_id_in(posting.app_url.as_deref().unwrap_or_default())
        .or_else(|| topic_id_in(posting.app_bundle_url.as_deref().unwrap_or_default()))
        .unwrap_or(0)
}

/// The thread id in a HEY app URL, if the URL names a topic.
pub fn topic_id_in_public(url: &str) -> Option<i64> {
    topic_id_in(url)
}

fn topic_id_in(url: &str) -> Option<i64> {
    let marker = url.rfind("/topics/")?;
    let segment = &url[marker + "/topics/".len()..];
    let segment = segment
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    segment.parse().ok()
}

fn contact_email(contact: &Contact) -> String {
    contact
        .email_address
        .as_ref()
        .map(|e| e.expose().to_string())
        .unwrap_or_default()
}

fn contact_initials(contact: &Contact) -> String {
    if let Some(initials) = &contact.initials {
        if !initials.is_empty() {
            return initials.clone();
        }
    }
    let name = contact.name.as_deref().unwrap_or_default().trim();
    if name.is_empty() {
        return "?".into();
    }
    name.split_whitespace()
        .take(2)
        .filter_map(|part| part.chars().next())
        .collect::<String>()
        .to_uppercase()
}

fn iso(dt: Option<&hey_sdk::DateTime>) -> Option<String> {
    dt.map(|d| d.to_string())
}

/// HEY's message bodies come back as a Trix attachment whose `content` is HTML-escaped a
/// second time and wrapped in `<shadow-content><template>`. Unwrap it so Blitz lays out the
/// actual message instead of an empty template element.
pub fn unwrap_hey_html(raw: &str) -> String {
    let mut html = raw.to_string();
    if let Some(marker) = raw.find("data-trix-attachment=\"") {
        let rest = &raw[marker + "data-trix-attachment=\"".len()..];
        if let Some(end) = rest.find('"') {
            let json = unescape_html(&rest[..end]);
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json)
                && let Some(content) = value.get("content").and_then(|v| v.as_str())
            {
                html = unescape_html(content);
            }
        }
    }
    for wrapper in [
        ("<shadow-content><template>", "</template></shadow-content>"),
        ("<template>", "</template>"),
    ] {
        if html.trim_start().starts_with(wrapper.0) {
            if let Some(end) = html.rfind(wrapper.1) {
                let start = html.find(wrapper.0).unwrap() + wrapper.0.len();
                return html[start..end].trim().to_string();
            }
        }
    }
    html
}

fn unescape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '&' {
            out.push(ch);
            continue;
        }
        let mut entity = String::new();
        while let Some(&next) = chars.peek() {
            entity.push(next);
            chars.next();
            if next == ';' {
                break;
            }
            if entity.len() > 12 {
                break;
            }
        }
        let decoded = match entity.as_str() {
            "lt;" => Some('<'),
            "gt;" => Some('>'),
            "quot;" => Some('"'),
            "amp;" => Some('&'),
            "apos;" | "#39;" => Some('\''),
            "nbsp;" => Some('\u{a0}'),
            _ => {
                if let Some(hex) = entity.strip_prefix("#x").and_then(|s| s.strip_suffix(';')) {
                    u32::from_str_radix(hex, 16)
                        .ok()
                        .and_then(char::from_u32)
                } else if let Some(dec) = entity.strip_prefix('#').and_then(|s| s.strip_suffix(';')) {
                    dec.parse::<u32>().ok().and_then(char::from_u32)
                } else {
                    None
                }
            }
        };
        match decoded {
            Some(c) => out.push(c),
            None => {
                out.push('&');
                out.push_str(&entity);
            }
        }
    }
    out
}

/// The local cache and the HEY client, in one handle. `Clone` shares both.
#[derive(Clone)]
pub struct Store {
    db: std::sync::Arc<Mutex<Connection>>,
    pub client: Client,
    pub config: Config,
    pub provider: std::sync::Arc<TokenProvider>,
}

impl Store {
    pub fn open(
        path: &Path,
        config: Config,
        provider: std::sync::Arc<TokenProvider>,
    ) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Connection::open(path)?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS boxes (key TEXT PRIMARY KEY, json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS postings (key TEXT PRIMARY KEY, json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS threads (topic_id INTEGER PRIMARY KEY, json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        let client = Client::new(config.clone(), provider.clone())?;
        Ok(Self {
            db: std::sync::Arc::new(Mutex::new(db)),
            client,
            config,
            provider,
        })
    }

    fn put<T: serde::Serialize>(&self, table: &str, key: &dyn std::fmt::Display, value: &T) -> Result<(), StoreError> {
        let json = serde_json::to_string(value)?;
        let sql = format!("INSERT OR REPLACE INTO {table} (key, json) VALUES (?1, ?2)");
        self.db.lock().unwrap().execute(&sql, params![key.to_string(), json])?;
        Ok(())
    }

    fn put_thread(&self, topic_id: i64, thread: &Thread) -> Result<(), StoreError> {
        let json = serde_json::to_string(thread)?;
        self.db
            .lock()
            .unwrap()
            .execute(
                "INSERT OR REPLACE INTO threads (topic_id, json) VALUES (?1, ?2)",
                params![topic_id, json],
            )?;
        Ok(())
    }

    fn get_thread(&self, topic_id: i64) -> Option<Thread> {
        self.db
            .lock()
            .unwrap()
            .query_row(
                "SELECT json FROM threads WHERE topic_id = ?1",
                params![topic_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok())
    }

    pub fn meta(&self, key: &str) -> Option<String> {
        self.db
            .lock()
            .unwrap()
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |row| row.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.db.lock().unwrap().execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    // ---------------------------------------------------------------- reading

    /// The cached box list. Empty until `sync_all` has run.
    pub fn boxes(&self) -> Vec<BoxInfo> {
        self.db
            .lock()
            .unwrap()
            .prepare("SELECT json FROM boxes ORDER BY key")
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get::<_, String>(0))
                    .ok()
                    .map(|rows| rows.filter_map(|r| r.ok()).filter_map(|j| serde_json::from_str(&j).ok()).collect())
            })
            .unwrap_or_default()
    }

    /// Cached rows for a box, newest first.
    pub fn postings(&self, kind: &str) -> Vec<ThreadRow> {
        let prefix = format!("{kind}:");
        self.db
            .lock()
            .unwrap()
            .prepare("SELECT json FROM postings WHERE key LIKE ?1")
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map(params![format!("{prefix}%")], |row| row.get::<_, String>(0))
                    .ok()
                    .map(|rows| {
                        let mut list: Vec<ThreadRow> = rows
                            .filter_map(|r| r.ok())
                            .filter_map(|j| serde_json::from_str(&j).ok())
                            .collect();
                        list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                        list
                    })
            })
            .unwrap_or_default()
    }

    pub fn thread_cached(&self, topic_id: i64) -> Option<Thread> {
        self.get_thread(topic_id)
    }

    pub fn labels_cached(&self) -> Vec<Label> {
        self.meta("labels.json")
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    // ---------------------------------------------------------------- syncing

    fn posting_to_row(&self, posting: &Posting, box_kind: &str) -> ThreadRow {
        let sender = posting
            .creator
            .as_ref()
            .or_else(|| posting.contacts.as_ref().and_then(|c| c.first()));
        let (sender_name, sender_email, sender_initials, avatar_color) = match sender {
            Some(c) => (
                c.name.clone().unwrap_or_default(),
                contact_email(c),
                contact_initials(c),
                c.avatar_background_color.clone().unwrap_or_default(),
            ),
            None => ("?".into(), String::new(), "?".into(), String::new()),
        };
        let labels = posting
            .folders
            .as_ref()
            .map(|f| f.iter().map(|x| x.id).collect())
            .unwrap_or_default();
        ThreadRow {
            posting_id: posting.id,
            topic_id: topic_id_from_posting(posting),
            box_kind: box_kind.to_string(),
            subject: posting.name.clone().unwrap_or_default(),
            summary: posting.summary.clone().unwrap_or_default(),
            sender_name,
            sender_email,
            sender_initials,
            avatar_color,
            created_at: iso(posting.created_at.as_ref()),
            seen: posting.seen.unwrap_or(false),
            is_bundle: posting.kind == "bundle",
            bubbled_up: posting.bubbled_up.unwrap_or(false),
            entry_count: posting.visible_entry_count.unwrap_or(0),
            label_ids: labels,
        }
    }

    fn cache_box(&self, kind: &str, postings: Vec<Posting>) -> Result<(), StoreError> {
        for posting in &postings {
            let row = self.posting_to_row(posting, kind);
            self.put("postings", &format!("{}:{}", kind, posting.id), &row)?;
        }
        Ok(())
    }

    /// Refreshes the box index and every core mailbox. Returns the number of cached rows.
    pub async fn sync_all(&self) -> Result<usize, StoreError> {
        let boxes: Vec<Mailbox> = self.client.boxes().list().await?.into_inner();
        let mut infos = Vec::new();
        for b in &boxes {
            let info = BoxInfo {
                id: b.id,
                kind: b.kind.clone(),
                name: b.name.clone(),
                posting_changes_url: b.posting_changes_url.clone(),
            };
            infos.push(info.clone());
            self.put("boxes", &b.kind, &info)?;
        }
        let mut total = 0;
        for info in &infos {
            if matches!(
                info.kind.as_str(),
                "imbox" | "feedbox" | "trailbox" | "asidebox" | "laterbox" | "bubblebox"
            ) {
                total += self.sync_box(&info.kind).await?;
            }
        }
        self.sync_labels().await?;
        Ok(total)
    }

    /// Refreshes a single mailbox and returns how many rows it holds now.
    pub async fn sync_box(&self, kind: &str) -> Result<usize, StoreError> {
        let postings = match kind {
            "imbox" => self.client.boxes().get_imbox(&Default::default()).await?.postings,
            "feedbox" => self.client.boxes().get_feedbox(&Default::default()).await?.postings,
            "trailbox" => self.client.boxes().get_trailbox(&Default::default()).await?.postings,
            "asidebox" => self.client.boxes().get_asidebox(&Default::default()).await?.postings,
            "laterbox" => self.client.boxes().get_laterbox(&Default::default()).await?.postings,
            "bubblebox" => self.client.boxes().get_bubblebox(&Default::default()).await?.postings,
            _ => None,
        }
        .unwrap_or_default();
        let count = postings.len();
        self.cache_box(kind, postings)?;
        Ok(count)
    }

    /// Uses HEY's incremental changes feed for a box. If the cursor is too old HEY asks for
    /// a full sync, which we do by re-reading the box.
    pub async fn sync_changes(&self, info: &BoxInfo) -> Result<(), StoreError> {
        let Some(url) = &info.posting_changes_url else {
            self.sync_box(&info.kind).await?;
            return Ok(());
        };
        let cursor = match hey_sdk::services::PostingChangesCursor::from_url(url) {
            Ok(c) => c,
            Err(_) => {
                self.sync_box(&info.kind).await?;
                return Ok(());
            }
        };
        let changes = self.client.postings().changes(info.id, &cursor).await?;
        if changes.full_sync_required {
            self.sync_box(&info.kind).await?;
            return Ok(());
        }
        for posting in changes.added.iter().chain(changes.updated.iter()) {
            let row = self.posting_to_row(posting, &info.kind);
            self.put("postings", &format!("{}:{}", info.kind, posting.id), &row)?;
        }
        for deleted in changes.deleted {
            self.db
                .lock()
                .unwrap()
                .execute(
                    "DELETE FROM postings WHERE key = ?1",
                    params![format!("{}:{}", info.kind, deleted.id)],
                )?;
        }
        Ok(())
    }

    async fn sync_labels(&self) -> Result<(), StoreError> {
        let nav = self.client.identity().get_navigation().await?;
        let mut labels = Vec::new();
        for item in nav.items.unwrap_or_default() {
            for child in item.menu_items.unwrap_or_default() {
                if let Some(url) = &child.app_url
                    && let Some(id) = url
                        .rsplit('/')
                        .next()
                        .and_then(|s| s.trim_end_matches(".json").parse::<i64>().ok())
                    && url.contains("/folders/")
                {
                    labels.push(Label {
                        id,
                        name: child.title.unwrap_or_default(),
                    });
                }
            }
        }
        self.set_meta("labels.json", &serde_json::to_string(&labels)?)?;
        Ok(())
    }

    /// Loads a full thread, fetching message bodies that the index only summarizes.
    pub async fn load_thread(&self, topic_id: i64) -> Result<Thread, StoreError> {
        if let Some(cached) = self.thread_cached(topic_id) {
            return Ok(cached);
        }
        let topic: Topic = self.client.topics().get(topic_id).await?;
        let mut messages = Vec::new();
        let entries: Vec<Entry> = topic.entries.clone().unwrap_or_default();
        for entry in entries.iter().take(80) {
            // Only messages have a body; notes already carry theirs.
            let detail = self.client.messages().get(entry.id).await.ok();
            let (html, subject, sent_at) = match &detail {
                Some(message) => (
                    unwrap_hey_html(&message.content.clone().unwrap_or_default()),
                    message.subject.clone().unwrap_or_default(),
                    iso(message.created_at.as_ref()),
                ),
                None => (
                    unwrap_hey_html(&entry.content.clone().unwrap_or_default()),
                    entry.subject.clone().unwrap_or_default(),
                    iso(entry.created_at.as_ref()),
                ),
            };
            let sender = detail
                .as_ref()
                .and_then(|m| m.sender.as_ref())
                .or(entry.creator.as_ref());
            if html.is_empty() && subject.is_empty() {
                continue;
            }
            messages.push(MessageView {
                id: entry.id,
                sender_name: sender.and_then(|c| c.name.clone()).unwrap_or_default(),
                sender_email: sender.map(contact_email).unwrap_or_default(),
                sender_initials: sender.map(contact_initials).unwrap_or_default(),
                subject,
                html,
                sent_at: sent_at.or_else(|| iso(entry.created_at.as_ref())),
            });
        }
        let thread = Thread {
            topic_id,
            subject: topic
                .name
                .clone()
                .or_else(|| messages.first().map(|m| m.subject.clone()))
                .unwrap_or_default(),
            messages,
        };
        self.put_thread(topic_id, &thread)?;
        Ok(thread)
    }

    /// The signed-in identity, used for the account chip and for sign-out.
    pub async fn identity(&self) -> Result<Identity, StoreError> {
        let identity = self.client.identity().get().await?;
        let primary = identity.primary_contact.as_ref();
        let email = primary.map(contact_email).unwrap_or_default();
        if !email.is_empty() {
            self.set_meta("account_email", &email)?;
        }
        Ok(Identity {
            name: primary.and_then(|c| c.name.clone()).unwrap_or_default(),
            email,
            avatar_url: primary.and_then(|c| c.avatar_url.clone()),
        })
    }

    // ---------------------------------------------------------------- actions

    pub async fn mark_seen(&self, posting_ids: &[i64], seen: bool) -> Result<(), StoreError> {
        if seen {
            self.client.postings().mark_postings_seen(posting_ids).await?;
        } else {
            self.client.postings().mark_postings_unseen(posting_ids).await?;
        }
        for id in posting_ids {
            self.patch_row(*id, |row| row.seen = seen)?;
        }
        Ok(())
    }

    pub async fn move_to(&self, kind: BoxKind, posting_ids: &[i64]) -> Result<(), StoreError> {
        self.client.postings().move_to_kind(kind, posting_ids).await?;
        for id in posting_ids {
            self.remove_row(*id)?;
        }
        self.sync_box(kind.as_str()).await?;
        Ok(())
    }

    pub async fn trash(&self, posting_ids: &[i64]) -> Result<(), StoreError> {
        self.client.postings().move_to_trash(posting_ids).await?;
        for id in posting_ids {
            self.remove_row(*id)?;
        }
        Ok(())
    }

    pub async fn spam(&self, posting_ids: &[i64]) -> Result<(), StoreError> {
        self.client.postings().mark_postings_spam(posting_ids).await?;
        for id in posting_ids {
            self.remove_row(*id)?;
        }
        Ok(())
    }

    pub async fn bubble_up(&self, posting_ids: &[i64]) -> Result<(), StoreError> {
        self.client
            .postings()
            .bubble_up_postings_now(posting_ids)
            .await?;
        Ok(())
    }

    pub async fn label_add(&self, posting_id: i64, label_id: i64) -> Result<(), StoreError> {
        self.client
            .postings()
            .file(&hey_sdk::models::FilePostingsRequestContent {
                posting_ids: vec![posting_id],
                folder_id: label_id,
            })
            .await?;
        self.patch_row(posting_id, |row| {
            if !row.label_ids.contains(&label_id) {
                row.label_ids.push(label_id);
            }
        })
    }

    pub async fn label_remove(&self, posting_id: i64, label_id: i64) -> Result<(), StoreError> {
        self.client
            .postings()
            .unfile(
                &posting_id.to_string(),
                &hey_sdk::services::postings::UnfilePostingsParams {
                    folder_id: Some(label_id),
                },
            )
            .await?;
        self.patch_row(posting_id, |row| row.label_ids.retain(|id| *id != label_id))
    }

    pub async fn screener(&self) -> Result<Vec<ScreenerItem>, StoreError> {
        let summary = self.client.clearances().pending(None).await?;
        let mut out = Vec::new();
        for clearance in summary.clearances.unwrap_or_default() {
            let sender = clearance.petitioner.as_ref();
            let entry = clearance.most_recent_entry.as_ref();
            out.push(ScreenerItem {
                clearance_id: clearance.id,
                sender_name: sender.and_then(|c| c.name.clone()).unwrap_or_default(),
                sender_email: sender.map(contact_email).unwrap_or_default(),
                subject: entry.and_then(|e| e.subject.clone()).unwrap_or_default(),
                summary: entry.and_then(|e| e.summary.clone()).unwrap_or_default(),
                topic_id: entry.and_then(|e| e.topic_id).unwrap_or(0),
            });
        }
        Ok(out)
    }

    pub async fn screen(&self, clearance_id: i64, approve: bool) -> Result<(), StoreError> {
        let status = if approve {
            ClearanceStatus::Approved
        } else {
            ClearanceStatus::Denied
        };
        self.client
            .clearances()
            .screen(clearance_id, status, &ScreenOptions::default())
            .await?;
        Ok(())
    }

    pub async fn search(&self, query: &str) -> Result<Vec<SearchHit>, StoreError> {
        let result = self
            .client
            .search()
            .search(&SearchParams {
                query: query.to_string(),
                ..SearchParams::default()
            })
            .await?;
        Ok(result
            .matches
            .into_iter()
            .map(|m| {
                let topic = m.topic;
                SearchHit {
                    topic_id: topic.id,
                    posting_id: m.posting_id.unwrap_or(0),
                    subject: topic.name.unwrap_or_default(),
                    sender_name: topic
                        .creator
                        .and_then(|c| c.name)
                        .unwrap_or_default(),
                    summary: topic
                        .latest_entry
                        .and_then(|e| e.summary)
                        .unwrap_or_default(),
                    updated_at: iso(topic.updated_at.as_ref()),
                }
            })
            .collect())
    }

    pub async fn send(
        &self,
        to: &[String],
        bcc: &[String],
        subject: &str,
        html: &str,
    ) -> Result<(), StoreError> {
        self.client
            .messages()
            .send(&MessageContent {
                subject: subject.to_string(),
                content: html.to_string(),
                to: to.to_vec(),
                bcc: bcc.to_vec(),
                ..MessageContent::default()
            })
            .await?;
        Ok(())
    }

    pub async fn reply(
        &self,
        entry_id: i64,
        content: &str,
    ) -> Result<hey_sdk::models::SentMessage, StoreError> {
        let prefill: MessageDraft = self.client.entries().new_reply(entry_id).await?;
        let to = prefill
            .addressed
            .as_ref()
            .and_then(|a| a.directly.as_ref())
            .map(|contacts| {
                contacts
                    .iter()
                    .filter_map(|c| c.email_address.as_ref())
                    .map(|e| e.expose().to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let sender_id = prefill
            .sender
            .as_ref()
            .map(|s| s.id)
            .unwrap_or_default();
        Ok(self
            .client
            .entries()
            .reply(
                entry_id,
                &ReplyContent {
                    acting_sender_id: sender_id,
                    subject: prefill.subject.clone().unwrap_or_default(),
                    content: content.to_string(),
                    to,
                    ..ReplyContent::default()
                },
            )
            .await?)
    }

    pub async fn save_draft(
        &self,
        to: &[String],
        bcc: &[String],
        subject: &str,
        html: &str,
    ) -> Result<i64, StoreError> {
        let id = self
            .client
            .messages()
            .create_draft(&hey_sdk::services::DraftContent {
                subject: subject.to_string(),
                content: html.to_string(),
                to: to.to_vec(),
                bcc: bcc.to_vec(),
                ..hey_sdk::services::DraftContent::default()
            })
            .await?;
        Ok(id)
    }

    pub async fn send_draft(
        &self,
        draft_id: i64,
        draft: &hey_sdk::services::DraftContent,
    ) -> Result<(), StoreError> {
        self.client.messages().send_draft(draft_id, draft).await?;
        Ok(())
    }

    pub async fn sign_out(&self) -> Result<(), StoreError> {
        self.provider.clear().map_err(|e| StoreError::Other(e.to_string()))
    }

    // ---------------------------------------------------------------- helpers

    fn all_rows_with_id(&self, posting_id: i64) -> Vec<(String, ThreadRow)> {
        self.db
            .lock()
            .unwrap()
            .prepare("SELECT key, json FROM postings")
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
                    .ok()
                    .map(|rows| {
                        rows.filter_map(|r| r.ok())
                            .filter_map(|(key, json)| {
                                serde_json::from_str::<ThreadRow>(&json)
                                    .ok()
                                    .map(|row| (key, row))
                            })
                            .filter(|(_, row)| row.posting_id == posting_id)
                            .collect()
                    })
            })
            .unwrap_or_default()
    }

    fn patch_row(&self, posting_id: i64, mut f: impl FnMut(&mut ThreadRow)) -> Result<(), StoreError> {
        for (key, mut row) in self.all_rows_with_id(posting_id) {
            f(&mut row);
            self.put("postings", &key, &row)?;
        }
        Ok(())
    }

    fn remove_row(&self, posting_id: i64) -> Result<(), StoreError> {
        let keys: Vec<String> = self
            .all_rows_with_id(posting_id)
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        let db = self.db.lock().unwrap();
        for key in keys {
            db.execute("DELETE FROM postings WHERE key = ?1", params![key])?;
        }
        Ok(())
    }
}
