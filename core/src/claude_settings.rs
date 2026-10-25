//! Points every `claude` at the proxy through Claude Code's user settings
//! (`~/.claude/settings.json`, `env` block), or takes it back out. Everything else in the
//! file is kept as it is, key order included.

use std::path::PathBuf;

use anyhow::{Context, bail};
use serde_json::{Map, Value, json};

use crate::providers::anthropic::PLACEHOLDER_TOKEN;
use crate::{auth, config, control};

const BASE_URL: &str = "ANTHROPIC_BASE_URL";
const AUTH_TOKEN: &str = "ANTHROPIC_AUTH_TOKEN";
const OPUS_MODEL: &str = "ANTHROPIC_DEFAULT_OPUS_MODEL";
/// `/model opus` is an explicit pick of Claude, not a bare name the proxy treats as internal.
const OPUS_TARGET: &str = "anthropic:claude-opus-5-5";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Connection {
    /// Every `claude` goes through this proxy.
    Connected,
    /// The settings point at another address.
    Other(String),
    Disconnected,
}

pub fn settings_file() -> PathBuf {
    let dir = config::env("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| config::home_dir().join(".claude"));
    dir.join("settings.json")
}

fn read() -> anyhow::Result<Map<String, Value>> {
    let path = settings_file();
    match std::fs::read(&path) {
        Ok(bytes) if bytes.iter().all(u8::is_ascii_whitespace) => Ok(Map::new()),
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(Value::Object(map)) => Ok(map),
            Ok(_) => bail!("{} is not a JSON object", path.display()),
            Err(e) => bail!("{} is not valid JSON ({e}); fix it by hand first", path.display()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Writes through a temporary file, so Claude Code never reads a half-written one.
fn write(settings: &Map<String, Value>) -> anyhow::Result<()> {
    let path = settings_file();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut text = serde_json::to_string_pretty(settings)?;
    text.push('\n');
    let tmp = path.with_extension("json.claude-bridge.tmp");
    std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))
}

pub fn connection(port: u16) -> Connection {
    let url = read().ok().and_then(|s| s.get("env")?.get(BASE_URL)?.as_str().map(String::from));
    match url {
        Some(url) if url.trim_end_matches('/') == control::base_url(port) => Connection::Connected,
        Some(url) => Connection::Other(url),
        None => Connection::Disconnected,
    }
}

/// Without a login of its own Claude Code (the IDE extension included) asks to log in even though
/// the proxy serves other providers; the placeholder token stops that, as the launcher does. With a
/// login it has to go: a token in `env` takes precedence over the subscription. True when `env` changed.
fn sync_placeholder(env: &mut Map<String, Value>) -> bool {
    let logged_in = auth::claude_code::account().is_some();
    match env.get(AUTH_TOKEN).map(|v| v.as_str() == Some(PLACEHOLDER_TOKEN)) {
        None if !logged_in => {
            env.insert(AUTH_TOKEN.into(), json!(PLACEHOLDER_TOKEN));
            true
        }
        Some(true) if logged_in => {
            env.shift_remove(AUTH_TOKEN);
            true
        }
        // The user's own token, or already right.
        _ => false,
    }
}

pub fn connect(port: u16) -> anyhow::Result<()> {
    let mut settings = read()?;
    let env = settings.entry("env").or_insert_with(|| json!({}));
    let Some(env) = env.as_object_mut() else { bail!("\"env\" in {} is not an object", settings_file().display()) };
    env.insert(BASE_URL.into(), json!(control::base_url(port)));
    env.entry(OPUS_MODEL).or_insert_with(|| json!(OPUS_TARGET));
    sync_placeholder(env);
    write(&settings)
}

/// Keeps the placeholder token in step with Claude Code's login while connected; writes only on change.
pub fn sync_auth(port: u16) -> anyhow::Result<()> {
    if connection(port) != Connection::Connected {
        return Ok(());
    }
    let mut settings = read()?;
    let Some(env) = settings.get_mut("env").and_then(Value::as_object_mut) else { return Ok(()) };
    if sync_placeholder(env) { write(&settings) } else { Ok(()) }
}

/// Removes only what `connect` added; an empty `env` goes away too.
pub fn disconnect() -> anyhow::Result<()> {
    let mut settings = read()?;
    let Some(env) = settings.get_mut("env").and_then(Value::as_object_mut) else { return Ok(()) };
    let ours = |v: Option<&Value>, f: &dyn Fn(&str) -> bool| v.and_then(Value::as_str).is_some_and(f);
    let mut changed = false;
    if ours(env.get(BASE_URL), &|u| u.starts_with("http://127.0.0.1:") || u.starts_with("http://localhost:")) {
        env.shift_remove(BASE_URL);
        changed = true;
    }
    if ours(env.get(OPUS_MODEL), &|m| m == OPUS_TARGET) {
        env.shift_remove(OPUS_MODEL);
        changed = true;
    }
    if ours(env.get(AUTH_TOKEN), &|t| t == PLACEHOLDER_TOKEN) {
        env.shift_remove(AUTH_TOKEN);
        changed = true;
    }
    if !changed {
        return Ok(());
    }
    if env.is_empty() {
        settings.shift_remove("env");
    }
    write(&settings)
}
