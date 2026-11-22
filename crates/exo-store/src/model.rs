//! Plain, owned view models. The UI never has to import the HEY SDK to draw a row or a
//! thread: everything here is a `String`/`i64` snapshot of what the SDK answered.

use serde::{Deserialize, Serialize};

/// A mailbox. `kind` is HEY's `imbox`, `feedbox`, `trailbox`, `asidebox`, `laterbox` or
/// `bubblebox`; the ones Exo shows in the sidebar are given friendly names.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoxInfo {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub posting_changes_url: Option<String>,
}

/// One row in a mailbox's thread list.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ThreadRow {
    /// Box-item id: use this for seen/unseen, move, trash and labels.
    pub posting_id: i64,
    /// Thread id: use this to read, reply and open.
    pub topic_id: i64,
    pub box_kind: String,
    pub subject: String,
    pub summary: String,
    pub sender_name: String,
    pub sender_email: String,
    pub sender_initials: String,
    pub avatar_color: String,
    pub created_at: Option<String>,
    pub seen: bool,
    pub is_bundle: bool,
    pub bubbled_up: bool,
    pub entry_count: i32,
    pub label_ids: Vec<i64>,
}

impl ThreadRow {
    pub fn display_name(&self) -> &str {
        if self.sender_name.is_empty() {
            if self.sender_email.is_empty() {
                "Unknown sender"
            } else {
                &self.sender_email
            }
        } else {
            &self.sender_name
        }
    }
}

/// One message in a thread, with its rendered HTML body.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MessageView {
    pub id: i64,
    pub sender_name: String,
    pub sender_email: String,
    pub sender_initials: String,
    pub subject: String,
    pub html: String,
    pub sent_at: Option<String>,
}

/// A fully loaded thread.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Thread {
    pub topic_id: i64,
    pub subject: String,
    pub messages: Vec<MessageView>,
}

/// A label (HEY folder).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Label {
    pub id: i64,
    pub name: String,
}

/// Someone waiting in The Screener.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ScreenerItem {
    pub clearance_id: i64,
    pub sender_name: String,
    pub sender_email: String,
    pub subject: String,
    pub summary: String,
    pub topic_id: i64,
}

/// A search result row.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SearchHit {
    pub topic_id: i64,
    pub posting_id: i64,
    pub subject: String,
    pub sender_name: String,
    pub summary: String,
    pub updated_at: Option<String>,
}

/// The signed-in identity.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Identity {
    pub name: String,
    pub email: String,
    pub avatar_url: Option<String>,
}
