//! The async engine behind the UI.
//!
//! egui's frame loop must never wait on the network. The app sends [`Command`]s to this
//! worker; it runs them on a Tokio runtime, then sends [`Event`]s back with ready-to-draw
//! snapshots. A ticking task follows HEY's posting-changes feed so new mail appears without
//! waiting for an explicit refresh.

use std::{collections::HashMap, path::PathBuf, time::Duration};
use tokio::sync::mpsc;

use exo_auth::{CredentialStore, TokenProvider, sign_in};
use exo_store::{BoxInfo, Identity, Label, ScreenerItem, SearchHit, Store, Thread, ThreadRow};
use hey_sdk::services::BoxKind;

#[derive(Debug)]
pub enum Command {
    SignIn,
    SignOut,
    /// First load: boxes, identity, labels, screener and all core mailboxes.
    Bootstrap,
    Refresh,
    LoadBox(String),
    LoadThread(i64),
    MarkSeen(Vec<i64>, bool),
    Move(BoxKind, Vec<i64>),
    Trash(Vec<i64>),
    Spam(Vec<i64>),
    Bubble(Vec<i64>),
    LabelAdd(i64, i64),
    LabelRemove(i64, i64),
    Screen(i64, bool),
    Search(String),
    Send {
        to: Vec<String>,
        subject: String,
        body: String,
    },
    Reply {
        entry_id: i64,
        body: String,
    },
    SaveDraft {
        to: Vec<String>,
        subject: String,
        body: String,
    },
}

#[derive(Debug)]
pub enum Event {
    SignedIn {
        identity: Identity,
    },
    SignedOut,
    Bootstrapped {
        identity: Identity,
        boxes: Vec<BoxInfo>,
        labels: Vec<Label>,
        screener: Vec<ScreenerItem>,
        rows: HashMap<String, Vec<ThreadRow>>,
    },
    BoxLoaded {
        kind: String,
        rows: Vec<ThreadRow>,
    },
    BoxesUpdated {
        rows: HashMap<String, Vec<ThreadRow>>,
    },
    ThreadLoaded {
        thread: Thread,
    },
    ScreenerLoaded {
        items: Vec<ScreenerItem>,
    },
    SearchDone {
        hits: Vec<SearchHit>,
    },
    Done {
        message: String,
    },
    Error {
        message: String,
    },
}

const APP: fastframe_dirs::App = fastframe_dirs::App::new("exo", "", "", "Exo");

fn cache_path() -> PathBuf {
    APP.installed()
        .map(|dirs| dirs.state.join("hey-cache.sqlite"))
        .or_else(|| dirs::data_local_dir().map(|p| p.join("exo").join("hey-cache.sqlite")))
        .unwrap_or_else(|| PathBuf::from("hey-cache.sqlite"))
}

fn error(event: &mpsc::UnboundedSender<Event>, message: impl Into<String>) {
    let _ = event.send(Event::Error {
        message: message.into(),
    });
}

/// Runs until the UI drops its command sender. `rx` and `event` are the two ends of the
/// channel pair shared with the app.
pub async fn run(
    config: hey_sdk::Config,
    credentials: CredentialStore,
    provider: std::sync::Arc<TokenProvider>,
    mut rx: mpsc::UnboundedReceiver<Command>,
    event: mpsc::UnboundedSender<Event>,
) {
    let mut store: Option<Store> = match credentials.load() {
        Ok(Some(creds)) => {
            let _ = provider.set(creds);
            match Store::open(&cache_path(), config.clone(), provider.clone()) {
                Ok(s) => Some(s),
                Err(e) => {
                    log::error!("could not open the local cache: {e}");
                    error(&event, format!("Could not open the local cache: {e}"));
                    None
                }
            }
        }
        _ => None,
    };

    let notifier = fastframe_notifications::Client::new(fastframe_notifications::App::new(
        "exo", "Exo",
    ))
    .ok();

    let mut tick = tokio::time::interval(Duration::from_secs(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            command = rx.recv() => {
                let Some(command) = command else { break };
                match command {
                    Command::SignIn => {
                        let sign_in_config = config.clone();
                        let result = tokio::task::spawn_blocking(move || sign_in(&sign_in_config)).await;
                        match result {
                            Ok(Ok(signin)) => {
                                if let Err(e) = provider.set(signin.credentials) {
                                    error(&event, format!("Could not save credentials: {e}"));
                                    continue;
                                }
                                match Store::open(&cache_path(), config.clone(), provider.clone()) {
                                    Ok(s) => {
                                        store = Some(s.clone());
                                        bootstrap(&s, &event).await;
                                    }
                                    Err(e) => error(&event, format!("Could not open the local cache: {e}")),
                                }
                            }
                            Ok(Err(e)) => error(&event, format!("Sign-in failed: {e}")),
                            Err(e) => error(&event, format!("Sign-in task failed: {e}")),
                        }
                    }
                    Command::SignOut => {
                        if let Some(s) = &store {
                            let _ = s.sign_out().await;
                        }
                        store = None;
                        let _ = event.send(Event::SignedOut);
                    }
                    Command::Bootstrap => {
                        if let Some(s) = &store { bootstrap(s, &event).await; }
                        else { error(&event, "Sign in to HEY first"); }
                    }
                    Command::Refresh => {
                        if let Some(s) = &store {
                            match s.sync_all().await {
                                Ok(_) => {
                                    let rows = all_rows(s);
                                    let _ = event.send(Event::BoxesUpdated { rows });
                                    let _ = event.send(Event::Done { message: "Up to date".into() });
                                }
                                Err(e) => error(&event, format!("Refresh failed: {e}")),
                            }
                        }
                    }
                    Command::LoadBox(kind) => {
                        if let Some(s) = &store {
                            match s.sync_box(&kind).await {
                                Ok(_) => {
                                    let _ = event.send(Event::BoxLoaded { kind: kind.clone(), rows: s.postings(&kind) });
                                }
                                Err(e) => error(&event, format!("Could not load {kind}: {e}")),
                            }
                        }
                    }
                    Command::LoadThread(topic_id) => {
                        if let Some(s) = &store {
                            if topic_id == 0 {
                                error(&event, "This bundled row has no thread to open");
                                continue;
                            }
                            match s.load_thread(topic_id).await {
                                Ok(thread) => {
                                    let _ = event.send(Event::ThreadLoaded { thread });
                                    let ids: Vec<i64> = s.postings("imbox")
                                        .into_iter()
                                        .chain(s.postings("feedbox"))
                                        .chain(s.postings("trailbox"))
                                        .chain(s.postings("asidebox"))
                                        .chain(s.postings("laterbox"))
                                        .filter(|r| r.topic_id == topic_id)
                                        .map(|r| r.posting_id)
                                        .collect();
                                    if !ids.is_empty() {
                                        let _ = s.mark_seen(&ids, true).await;
                                    }
                                }
                                Err(e) => error(&event, format!("Could not open thread: {e}")),
                            }
                        }
                    }
                    Command::MarkSeen(ids, seen) => {
                        if let Some(s) = &store {
                            if let Err(e) = s.mark_seen(&ids, seen).await {
                                error(&event, format!("Could not update read state: {e}"));
                            }
                        }
                    }
                    Command::Move(kind, ids) => {
                        if let Some(s) = &store {
                            if let Err(e) = s.move_to(kind, &ids).await {
                                error(&event, format!("Could not move thread: {e}"));
                            } else {
                                let _ = event.send(Event::BoxLoaded { kind: kind.as_str().to_string(), rows: s.postings(kind.as_str()) });
                                let _ = event.send(Event::Done { message: "Moved".into() });
                            }
                        }
                    }
                    Command::Trash(ids) => {
                        if let Some(s) = &store {
                            match s.trash(&ids).await {
                                Ok(()) => { let _ = event.send(Event::Done { message: "Moved to Trash".into() }); }
                                Err(e) => error(&event, format!("Could not move to Trash: {e}")),
                            }
                        }
                    }
                    Command::Spam(ids) => {
                        if let Some(s) = &store {
                            match s.spam(&ids).await {
                                Ok(()) => { let _ = event.send(Event::Done { message: "Marked as spam".into() }); }
                                Err(e) => error(&event, format!("Could not mark as spam: {e}")),
                            }
                        }
                    }
                    Command::Bubble(ids) => {
                        if let Some(s) = &store {
                            match s.bubble_up(&ids).await {
                                Ok(()) => { let _ = event.send(Event::Done { message: "Bubbled up".into() }); }
                                Err(e) => error(&event, format!("Could not bubble up: {e}")),
                            }
                        }
                    }
                    Command::LabelAdd(posting_id, label_id) => {
                        if let Some(s) = &store {
                            match s.label_add(posting_id, label_id).await {
                                Ok(()) => { let _ = event.send(Event::Done { message: "Label added".into() }); }
                                Err(e) => error(&event, format!("Could not add label: {e}")),
                            }
                        }
                    }
                    Command::LabelRemove(posting_id, label_id) => {
                        if let Some(s) = &store {
                            match s.label_remove(posting_id, label_id).await {
                                Ok(()) => { let _ = event.send(Event::Done { message: "Label removed".into() }); }
                                Err(e) => error(&event, format!("Could not remove label: {e}")),
                            }
                        }
                    }
                    Command::Screen(id, approve) => {
                        if let Some(s) = &store {
                            match s.screen(id, approve).await {
                                Ok(()) => {
                                    let _ = event.send(Event::Done { message: if approve { "Sender allowed in".into() } else { "Sender turned away".into() } });
                                    if let Ok(items) = s.screener().await {
                                        let _ = event.send(Event::ScreenerLoaded { items });
                                    }
                                }
                                Err(e) => error(&event, format!("Could not screen sender: {e}")),
                            }
                        }
                    }
                    Command::Search(query) => {
                        if let Some(s) = &store {
                            match s.search(&query).await {
                                Ok(hits) => { let _ = event.send(Event::SearchDone { hits }); }
                                Err(e) => error(&event, format!("Search failed: {e}")),
                            }
                        }
                    }
                    Command::Send { to, subject, body } => {
                        if let Some(s) = &store {
                            match s.send(&to, &subject, &body).await {
                                Ok(()) => { let _ = event.send(Event::Done { message: "Message sent".into() }); }
                                Err(e) => error(&event, format!("Could not send: {e}")),
                            }
                        }
                    }
                    Command::Reply { entry_id, body } => {
                        if let Some(s) = &store {
                            match s.reply(entry_id, &body).await {
                                Ok(_) => { let _ = event.send(Event::Done { message: "Reply sent".into() }); }
                                Err(e) => error(&event, format!("Could not reply: {e}")),
                            }
                        }
                    }
                    Command::SaveDraft { to, subject, body } => {
                        if let Some(s) = &store {
                            match s.save_draft(&to, &subject, &body).await {
                                Ok(_) => { let _ = event.send(Event::Done { message: "Draft saved".into() }); }
                                Err(e) => error(&event, format!("Could not save draft: {e}")),
                            }
                        }
                    }
                }
            }
            _ = tick.tick() => {
                let Some(s) = &store else { continue };
                let before = s.postings("imbox").iter().filter(|r| !r.seen).count();
                let mut touched = false;
                for box_info in s.boxes() {
                    if s.sync_changes(&box_info).await.is_ok() { touched = true; }
                }
                if touched {
                    let rows = all_rows(s);
                    let after = rows
                        .get("imbox")
                        .map(|list| list.iter().filter(|r| !r.seen).count())
                        .unwrap_or(0);
                    if let Some(client) = &notifier
                        && after > before
                    {
                        let note = fastframe_notifications::Notification::new("New mail")
                            .body(format!("{} new message(s) in the Imbox", after - before));
                        let _ = client.show(note);
                    }
                    let _ = event.send(Event::BoxesUpdated { rows });
                }
            }
        }
    }
}

fn all_rows(s: &Store) -> HashMap<String, Vec<ThreadRow>> {
    s.boxes()
        .into_iter()
        .map(|b| {
            let kind = b.kind.clone();
            let rows = s.postings(&kind);
            (kind, rows)
        })
        .collect()
}

async fn bootstrap(s: &Store, event: &mpsc::UnboundedSender<Event>) {
    match s.sync_all().await {
        Ok(_) => {
            let identity = s.identity().await.unwrap_or_default();
            let labels = s.labels_cached();
            let screener = s.screener().await.unwrap_or_default();
            let rows = all_rows(s);
            let _ = event.send(Event::Bootstrapped {
                identity,
                boxes: s.boxes(),
                labels,
                screener,
                rows,
            });
        }
        Err(e) => error(event, format!("Could not sync with HEY: {e}")),
    }
}
