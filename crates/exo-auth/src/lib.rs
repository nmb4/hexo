//! HEY authentication for Exo.
//!
//! Sign-in is OAuth 2.0 with PKCE: a loopback server waits on `127.0.0.1`, the browser opens
//! HEY's authorization page, and the code it redirects back is exchanged for tokens. Tokens
//! live in the platform credential store (Keychain on macOS, Credential Manager on Windows,
//! Secret Service on Linux), with a private file as a fallback. The [`TokenProvider`] handed to
//! the HEY SDK refreshes an expired access token transparently.

use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use hey_sdk::{
    SensitiveString,
    oauth::{self, ExchangeRequest, OAuthClient, RefreshRequest, ServerMetadata},
    {Config, Error as HeyError},
};
use serde::{Deserialize, Serialize};

const SERVICE: &str = "exo.hey.auth";
const ACCOUNT: &str = "default";
const REDIRECT_PATH: &str = "/callback";

/// Credentials persisted between runs. Nothing here is logged or shown to the user.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Credentials {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub install_id: String,
    /// Unix seconds at which `access_token` expires, when HEY told us.
    pub expires_at: Option<i64>,
    pub account_email: Option<String>,
}

impl Credentials {
    fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(at) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                now >= at - 60
            }
            None => false,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("HEY API error: {0}")]
    Hey(#[from] HeyError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the sign-in redirect was missing its authorization code")]
    MissingCode,
    #[error("the sign-in redirect had a state that did not match the request")]
    StateMismatch,
    #[error("the credential store rejected a write: {0}")]
    Store(String),
    #[error("no credentials are stored; sign in first")]
    NoCredentials,
    #[error("the browser could not be opened: {0}")]
    Browser(String),
}

/// Where credentials are kept. It tries the OS credential store first and falls back to a
/// private JSON file so Exo still works on machines without a keyring daemon.
#[derive(Clone, Debug)]
pub struct CredentialStore {
    entry: Option<Arc<keyring::Entry>>,
    fallback: PathBuf,
}

impl CredentialStore {
    pub fn new() -> Self {
        let entry = keyring::Entry::new(SERVICE, ACCOUNT)
            .ok()
            .map(Arc::new);
        let fallback = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("exo")
            .join("credentials.json");
        Self { entry, fallback }
    }

    pub fn load(&self) -> Result<Option<Credentials>, AuthError> {
        if let Some(entry) = &self.entry {
            if let Ok(value) = entry.get_password() {
                if let Ok(creds) = serde_json::from_str(&value) {
                    return Ok(Some(creds));
                }
            }
        }
        match std::fs::read_to_string(&self.fallback) {
            Ok(value) => Ok(serde_json::from_str(&value).ok()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, creds: &Credentials) -> Result<(), AuthError> {
        let value = serde_json::to_string(creds)?;
        if let Some(entry) = &self.entry {
            if entry.set_password(&value).is_ok() {
                return Ok(());
            }
        }
        if let Some(parent) = self.fallback.parent() {
            std::fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&self.fallback)?;
            file.write_all(value.as_bytes())?;
        }
        #[cfg(not(unix))]
        std::fs::write(&self.fallback, value.as_bytes())?;
        Ok(())
    }

    pub fn clear(&self) -> Result<(), AuthError> {
        if let Some(entry) = &self.entry {
            let _ = entry.delete_credential();
        }
        match std::fs::remove_file(&self.fallback) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

impl Default for CredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

/// A random, stable-per-install identifier. HEY requires it on every OAuth request.
pub fn install_id() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("system randomness");
    bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
}

/// Opens `url` in the user's browser on macOS, Linux and Windows.
pub fn open_browser(url: &str) -> Result<(), AuthError> {
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(target_os = "windows") {
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    match std::process::Command::new(program).args(args).spawn() {
        Ok(_) => Ok(()),
        Err(e) => Err(AuthError::Browser(e.to_string())),
    }
}

/// The outcome of an interactive sign-in.
#[derive(Debug)]
pub struct SignIn {
    pub credentials: Credentials,
    pub identity_email: Option<String>,
}

/// Runs the interactive PKCE flow. `base_url` lets tests point at a local HEY; production
/// callers pass `None` for HEY's canonical origin.
pub fn sign_in(config: &Config) -> Result<SignIn, AuthError> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(false)?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}{REDIRECT_PATH}");

    let pkce = oauth::generate_pkce();
    let state = oauth::generate_state();
    let install = install_id();
    let metadata = ServerMetadata::for_hey(&config.base_url);
    let url = oauth::authorization_url(
        &metadata,
        &config.oauth_client_id,
        &redirect_uri,
        None,
        &state,
        &pkce,
        &install,
    )?;

    open_browser(url.as_str())?;

    // The browser follows the redirect with `?code=…&state=…`. Serve a small confirmation
    // page so the person knows they can come back to the app.
    let (mut stream, _) = listener.accept()?;
    let (code, returned_state) = {
        let mut reader = BufReader::new(&stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line)?;
        let target = request_line.split_whitespace().nth(1).unwrap_or_default();
        let pairs: Vec<(String, String)> = url::Url::parse(&format!("http://localhost{target}"))
            .ok()
            .map(|u| {
                u.query_pairs()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect()
            })
            .unwrap_or_default();
        let code = pairs
            .iter()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.clone());
        let state = pairs
            .iter()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.clone());
        (code, state)
    };
    let Some(code) = code else {
        return Err(AuthError::MissingCode);
    };
    if returned_state.as_deref() != Some(state.as_str()) {
        return Err(AuthError::StateMismatch);
    }
    let body = "<html><body style=\"font-family:system-ui;text-align:center;padding:3rem\"><h1>You're signed in</h1><p>Return to Exo to start reading.</p></body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();

    // Exchange the code on a blocking runtime; the UI calls this function off its frame loop.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AuthError::Browser(e.to_string()))?;
    let client = OAuthClient::default();
    let token = runtime.block_on(client.exchange(&ExchangeRequest {
        token_endpoint: metadata.token_endpoint.clone(),
        code: SensitiveString::new(code),
        redirect_uri,
        client_id: config.oauth_client_id.clone(),
        client_secret: None,
        code_verifier: pkce.verifier,
        install_id: install.clone(),
    }))?;

    let expires_at = token.expires_at.map(|t| t.as_second() as i64);
    Ok(SignIn {
        credentials: Credentials {
            access_token: token.access_token.expose().to_string(),
            refresh_token: token.refresh_token.map(|t| t.expose().to_string()),
            install_id: install,
            expires_at,
            account_email: None,
        },
        identity_email: None,
    })
}

/// An [`hey_sdk::TokenProvider`] backed by a [`CredentialStore`]. It refreshes with the
/// refresh token when the access token is expired, and answers the SDK's 401 retry with the
/// same fresh token.
#[derive(Clone, Debug)]
pub struct TokenProvider {
    store: CredentialStore,
    config: Config,
    current: Arc<std::sync::RwLock<Option<Credentials>>>,
    refreshing: Arc<tokio::sync::Mutex<()>>,
}

impl TokenProvider {
    pub fn new(store: CredentialStore, config: Config) -> Self {
        let current = store.load().ok().flatten();
        Self {
            store,
            config,
            current: Arc::new(std::sync::RwLock::new(current)),
            refreshing: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Sets credentials in memory without writing them to disk. Useful for tests and for
    /// callers that manage persistence themselves.
    pub fn set_in_memory(&self, credentials: Credentials) {
        *self.current.write().unwrap() = Some(credentials);
    }

    pub fn set(&self, credentials: Credentials) -> Result<(), AuthError> {
        self.store.save(&credentials)?;
        *self.current.write().unwrap() = Some(credentials);
        Ok(())
    }

    pub fn clear(&self) -> Result<(), AuthError> {
        self.store.clear()?;
        *self.current.write().unwrap() = None;
        Ok(())
    }

    pub fn credentials(&self) -> Option<Credentials> {
        self.current.read().unwrap().clone()
    }

    async fn refresh_locked(&self) -> bool {
        let Some(creds) = self.credentials() else {
            return false;
        };
        let Some(refresh_token) = creds.refresh_token.clone() else {
            return false;
        };
        let metadata = ServerMetadata::for_hey(&self.config.base_url);
        let client = OAuthClient::default();
        let result = client
            .refresh(&RefreshRequest {
                token_endpoint: metadata.token_endpoint,
                refresh_token: SensitiveString::new(refresh_token),
                client_id: self.config.oauth_client_id.clone(),
                client_secret: None,
                install_id: creds.install_id.clone(),
            })
            .await;
        match result {
            Ok(token) => {
                let next = Credentials {
                    access_token: token.access_token.expose().to_string(),
                    refresh_token: token
                        .refresh_token
                        .map(|t| t.expose().to_string())
                        .or(creds.refresh_token),
                    install_id: creds.install_id,
                    expires_at: token.expires_at.map(|t| t.as_second() as i64),
                    account_email: creds.account_email,
                };
                let _ = self.set(next);
                true
            }
            Err(e) => {
                log::warn!("HEY token refresh failed: {e}");
                false
            }
        }
    }
}

#[async_trait::async_trait]
impl hey_sdk::TokenProvider for TokenProvider {
    async fn access_token(&self) -> Result<String, HeyError> {
        let expired = self
            .current
            .read()
            .unwrap()
            .as_ref()
            .is_some_and(|c| c.is_expired());
        if expired {
            let _guard = self.refreshing.lock().await;
            self.refresh_locked().await;
        }
        self.current
            .read()
            .unwrap()
            .as_ref()
            .map(|c| c.access_token.clone())
            .ok_or_else(|| HeyError::auth("no HEY credentials; sign in first"))
    }

    async fn refresh(&self) -> bool {
        let _guard = self.refreshing.lock().await;
        self.refresh_locked().await
    }
}

/// How long to wait for the browser redirect before giving up.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(180);
