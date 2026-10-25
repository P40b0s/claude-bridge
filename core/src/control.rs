//! Client side of the background proxy: start, stop, health, model pinning and the
//! live request log. Used by both the CLI and the UI.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, bail};
use eventsource_stream::Eventsource;
use futures_util::{Stream, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config;
use crate::request_log::RequestRecord;
use crate::routing::Target;
use crate::server::SHUTDOWN_HEADER;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;

pub fn base_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

pub fn log_path() -> PathBuf {
    config::data_dir().join("proxy.log")
}

#[derive(Debug, Clone, Deserialize)]
pub struct Health {
    pub pid: u32,
    /// Unix milliseconds.
    pub started_at: i64,
    pub active: Option<Target>,
    pub pinned: Option<Target>,
}

fn client(timeout: Option<Duration>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder();
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    builder.build().unwrap_or_default()
}

pub async fn health(port: u16) -> Option<Health> {
    let resp = client(Some(Duration::from_secs(2))).get(format!("{}/healthz", base_url(port))).send().await.ok()?;
    resp.error_for_status().ok()?.json().await.ok()
}

async fn control(port: u16, method: reqwest::Method, path: &str, body: Option<Value>) -> anyhow::Result<reqwest::Response> {
    let mut rb = client(Some(Duration::from_secs(5)))
        .request(method, format!("{}{path}", base_url(port)))
        .header(SHUTDOWN_HEADER, "1");
    if let Some(body) = body {
        rb = rb.json(&body);
    }
    let resp = rb.send().await.context("proxy is not running")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or_default();
        let message = body["error"]["message"].as_str().unwrap_or("request failed");
        bail!("{message} ({status})");
    }
    Ok(resp)
}

/// Asks the proxy to exit and waits until it stops answering. Returns false if none was running.
pub async fn stop(port: u16) -> anyhow::Result<bool> {
    if health(port).await.is_none() {
        return Ok(false);
    }
    control(port, reqwest::Method::POST, "/_shutdown", None).await?;
    for _ in 0..30 {
        if health(port).await.is_none() {
            return Ok(true);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("proxy on port {port} did not stop")
}

/// Routes every request to `model` (a `/model` spec such as `terra@max`) until unpinned.
pub async fn pin_model(port: u16, model: &str) -> anyhow::Result<Target> {
    let resp = control(port, reqwest::Method::PUT, "/_model", Some(json!({ "model": model }))).await?;
    Ok(resp.json().await?)
}

/// Lets Claude Code's own `/model` choice apply again.
pub async fn unpin_model(port: u16) -> anyhow::Result<()> {
    control(port, reqwest::Method::DELETE, "/_model", None).await?;
    Ok(())
}

/// Recent request records followed by live updates; ends when the proxy goes away.
pub async fn events(port: u16) -> anyhow::Result<impl Stream<Item = RequestRecord>> {
    let resp = client(None).get(format!("{}/_events", base_url(port))).send().await?.error_for_status()?;
    Ok(resp
        .bytes_stream()
        .eventsource()
        .take_while(|e| std::future::ready(e.is_ok()))
        .filter_map(|e| async move { e.ok().and_then(|e| serde_json::from_str(&e.data).ok()) }))
}

fn open_log() -> anyhow::Result<std::fs::File> {
    let path = log_path();
    std::fs::create_dir_all(config::data_dir())?;
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }
    Ok(std::fs::OpenOptions::new().create(true).append(true).open(&path)?)
}

/// The `claude-bridge` CLI binary: `CLAUDE_BRIDGE_BIN`, next to this executable, ~/.cargo/bin, then PATH.
pub fn find_cli() -> Option<PathBuf> {
    let name = if cfg!(windows) { "claude-bridge.exe" } else { "claude-bridge" };
    let explicit = config::env("CLAUDE_BRIDGE_BIN").map(PathBuf::from);
    let sibling = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join(name)));
    let cargo = Some(config::home_dir().join(".cargo").join("bin").join(name));
    let on_path = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).map(|dir| dir.join(name)).collect::<Vec<_>>())
        .unwrap_or_default();
    [explicit, sibling, cargo].into_iter().flatten().chain(on_path).find(|p| p.is_file())
}

/// Reuses a healthy proxy, or starts `cli serve` detached and waits for it to answer.
pub async fn ensure_proxy(cli: &Path, port: u16, default: &str, restart: bool) -> anyhow::Result<Health> {
    if let Some(h) = health(port).await {
        if !restart {
            return Ok(h);
        }
        stop(port).await?;
    }

    let log = open_log()?;
    let mut cmd = Command::new(cli);
    cmd.args(["serve", "--default", default])
        .env("CLAUDE_BRIDGE_PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(unix)]
    {
        // Own process group: Ctrl-C in a terminal goes to Claude Code, not the proxy.
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    let mut child = cmd.spawn().with_context(|| format!("starting {}", cli.display()))?;

    let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        if let Some(h) = health(port).await {
            // Reap the proxy when it exits, or it lingers as a zombie of this process.
            std::thread::spawn(move || child.wait());
            return Ok(h);
        }
        if let Some(status) = child.try_wait()? {
            bail!("proxy exited with {status}; see {}", log_path().display());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let _ = child.kill();
    bail!("proxy did not become healthy in {STARTUP_TIMEOUT:?}; see {}", log_path().display())
}
