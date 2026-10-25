//! Shared OAuth plumbing: PKCE, JWT claims, token files and the one-shot loopback callback.

pub mod claude_code;
pub mod google;
pub mod openai;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, anyhow, bail};
use axum::Router;
use axum::extract::Query;
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::sync::{Notify, oneshot};

/// Access tokens are refreshed this long before they expire.
pub const REFRESH_SKEW_MS: i64 = 5 * 60 * 1000;

// Generous: the provider may ask for 2FA or phone verification before redirecting back.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(20 * 60);

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn random_urlsafe(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Default for Pkce {
    fn default() -> Self {
        Self::new()
    }
}

impl Pkce {
    pub fn new() -> Self {
        let verifier = random_urlsafe(32);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self { verifier, challenge }
    }
}

/// Claims of a JWT, without verifying the signature (we only read our own tokens).
pub fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let data = std::fs::read(path).ok()?;
    serde_json::from_slice(&data).ok()
}

/// Writes a credentials file readable only by the owner.
pub fn write_secret_json<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let data = serde_json::to_vec_pretty(value)?;
    let tmp = path.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let mut file = opts.open(&tmp).with_context(|| format!("writing {}", tmp.display()))?;
        file.write_all(&data)?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

pub fn remove_file(path: &Path) -> bool {
    std::fs::remove_file(path).is_ok()
}

pub fn open_browser(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = std::process::Command::new(opener)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Serves `path` on every listener until the provider redirects back with `code`, then shuts down.
pub async fn wait_for_code(listeners: Vec<TcpListener>, path: &str, expected_state: String) -> anyhow::Result<String> {
    let (tx, rx) = oneshot::channel::<Result<String, String>>();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let app = Router::new().route(
        path,
        get(move |Query(q): Query<HashMap<String, String>>| {
            let tx = tx.clone();
            let expected_state = expected_state.clone();
            async move {
                let result = if let Some(err) = q.get("error") {
                    let detail = q.get("error_description").map(String::as_str).unwrap_or("");
                    Err(format!("{err} {detail}").trim().to_string())
                } else if q.get("state") != Some(&expected_state) {
                    Err("state mismatch".into())
                } else if let Some(code) = q.get("code") {
                    Ok(code.clone())
                } else {
                    Err("no authorization code in callback".into())
                };
                // Plain text on purpose: nothing from the query string is rendered as HTML.
                let body = match &result {
                    Ok(_) => "Login complete. You can close this tab.".to_string(),
                    Err(e) => format!("Login failed: {e}"),
                };
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(result);
                }
                body
            }
        }),
    );

    let shutdown = Arc::new(Notify::new());
    let mut servers = AbortOnDrop(
        listeners
            .into_iter()
            .map(|listener| {
                let app = app.clone();
                let shutdown = shutdown.clone();
                tokio::spawn(async move {
                    let _ = axum::serve(listener, app).with_graceful_shutdown(async move { shutdown.notified().await }).await;
                })
            })
            .collect(),
    );

    let outcome = tokio::time::timeout(LOGIN_TIMEOUT, rx).await;
    shutdown.notify_waiters();
    // Let the browser get its response; if the login is cancelled instead, dropping the guard frees the port.
    let _ = tokio::time::timeout(Duration::from_secs(2), futures_util::future::join_all(servers.0.iter_mut())).await;
    match outcome {
        Err(_) => bail!("timed out waiting for the browser login"),
        Ok(Err(_)) => bail!("login callback server stopped unexpectedly"),
        Ok(Ok(result)) => result.map_err(|e| anyhow!(e)),
    }
}

/// Aborts the callback servers when a login is cancelled (its future dropped).
struct AbortOnDrop(Vec<tokio::task::JoinHandle<()>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        for server in &self.0 {
            server.abort();
        }
    }
}

pub fn print_login_url(url: &str) {
    eprintln!("  Opening browser for login. If it does not open, visit:\n\n  {url}\n");
    open_browser(url);
}
