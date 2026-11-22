//! End-to-end check of the store against a local HEY-shaped HTTP server.
//!
//! This is the same wire contract the HEY SDK speaks: the routes, the posting JSON and the
//! way a posting names its thread in `app_url`. No real account or network is involved.

use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
};

use exo_auth::{CredentialStore, Credentials, TokenProvider};
use exo_store::Store;
use hey_sdk::Config;

const BOXES: &str = r#"[
  {"id":1,"kind":"imbox","name":"The Imbox"},
  {"id":2,"kind":"feedbox","name":"The Feed"},
  {"id":3,"kind":"trailbox","name":"Paper Trail"},
  {"id":4,"kind":"asidebox","name":"Set Aside"},
  {"id":5,"kind":"laterbox","name":"Reply Later"},
  {"id":6,"kind":"bubblebox","name":"Bubble Up"}
]"#;

const IMBOX: &str = r##"{
  "id":1,"kind":"imbox","name":"The Imbox",
  "postings":[
    {"id":10,"kind":"topic","app_url":"/topics/100","name":"Lunch on Friday","summary":"Are you free?","seen":false,
     "created_at":"2026-11-22T09:30:00Z","visible_entry_count":1,
     "creator":{"id":5,"name":"Ann Lee","email_address":"ann@example.com","initials":"AL","avatar_background_color":"#ffcc00"},
     "folders":[{"id":12,"name":"Receipts"}]}
  ]
}"##;

const NAVIGATION: &str = r#"{"items":[{"title":"Labels","menu_items":[{"title":"Receipts","app_url":"/folders/12"}]}]}"#;
const IDENTITY: &str = r#"{"id":1,"name":"Noah","primary_contact":{"id":2,"name":"Noah","email_address":"noah@hey.com"}}"#;
const TOPIC: &str = r#"{"id":100,"name":"Lunch on Friday","entries":[{"id":200,"subject":"Lunch on Friday","creator":{"id":5,"name":"Ann Lee","email_address":"ann@example.com"},"created_at":"2026-11-22T09:30:00Z"}]}"#;
const MESSAGE: &str = r#"{"id":200,"subject":"Lunch on Friday","content":"<p>Are you free Friday?</p>","topic_id":100,"active_at":"2026-11-22T09:30:00Z","sender":{"id":5,"name":"Ann Lee","email_address":"ann@example.com"}}"#;

fn response(path: &str) -> &'static str {
    match path.split('?').next().unwrap_or(path) {
        "/boxes.json" => BOXES,
        "/imbox.json" => IMBOX,
        "/my/navigation.json" => NAVIGATION,
        "/identity.json" => IDENTITY,
        "/topics/100" | "/topics/100.json" => TOPIC,
        "/messages/200" | "/messages/200.json" => MESSAGE,
        "/feedbox.json" | "/paper_trail.json" | "/set_aside.json" | "/reply_later.json"
        | "/bubble_up.json" => r#"{"id":2,"kind":"feedbox","name":"The Feed","postings":[]}"#,
        _ => "{}",
    }
}

/// A tiny HTTP server: one request per connection, which is all the SDK sends here.
fn spawn_hey() -> (String, mpsc::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = mpsc::channel();
    thread::spawn(move || {
        for stream in listener.incoming() {
            if stopped.try_recv().is_ok() {
                break;
            }
            let Ok(mut stream) = stream else { continue };
            stream
                .set_read_timeout(Some(std::time::Duration::from_millis(500)))
                .ok();
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            let read = reader.read_line(&mut line);
            let _ = read;
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            while reader.read_line(&mut String::new()).unwrap_or(0) > 2 {}
            let body = response(&path);
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.flush();
        }
    });
    (format!("http://{address}"), stop)
}

#[tokio::test]
async fn syncs_boxes_threads_and_labels() {
    let (base_url, stop) = spawn_hey();
    let config = Config {
        base_url,
        ..Config::default()
    };
    let provider = std::sync::Arc::new(TokenProvider::new(
        CredentialStore::new(),
        config.clone(),
    ));
    provider.set_in_memory(Credentials {
        access_token: "test-token".into(),
        refresh_token: None,
        install_id: "test-install".into(),
        expires_at: None,
        account_email: None,
    });

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("cache.sqlite"), config, provider).unwrap();

    let total = store.sync_all().await.unwrap();
    assert_eq!(total, 1, "only the Imbox has a posting");
    assert_eq!(store.boxes().len(), 6);

    let rows = store.postings("imbox");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.posting_id, 10);
    assert_eq!(row.topic_id, 100, "the thread comes from app_url");
    assert_eq!(row.subject, "Lunch on Friday");
    assert_eq!(row.sender_name, "Ann Lee");
    assert_eq!(row.sender_initials, "AL");
    assert!(!row.seen);
    assert_eq!(row.label_ids, vec![12]);

    let labels = store.labels_cached();
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].name, "Receipts");

    let thread = store.load_thread(100).await.unwrap();
    assert_eq!(thread.subject, "Lunch on Friday");
    assert_eq!(thread.messages.len(), 1);
    assert_eq!(thread.messages[0].html, "<p>Are you free Friday?</p>");

    // A second read is served from SQLite, without another HTTP round trip.
    stop.send(()).unwrap();
    let cached = store.load_thread(100).await.unwrap();
    assert_eq!(cached, thread);
    assert_eq!(store.postings("imbox").len(), 1);
}

#[test]
fn resolves_topic_ids_from_hey_urls() {
    assert_eq!(
        exo_store::topic_id_in_public("https://app.hey.com/topics/12345"),
        Some(12345)
    );
    assert_eq!(
        exo_store::topic_id_in_public("/topics/42?utm=1"),
        Some(42)
    );
    assert_eq!(exo_store::topic_id_in_public("/contacts/9"), None);
}
