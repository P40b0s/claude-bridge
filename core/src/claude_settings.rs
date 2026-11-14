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
/// Keys of `managed_env` the bridge wrote, comma-separated, so it can update and remove exactly
/// those and leave the user's own values alone.
const MANAGED_MARKER: &str = "CLAUDE_BRIDGE_MANAGED";

/// Values the bridge keeps in Claude Code's environment besides the address and login.
pub fn managed_env() -> Vec<(&'static str, String)> {
    let mut env = Vec::new();
    // Without it a connected `claude` compacts by Claude's own window, long after GPT sessions
    // get expensive.
    let ctx = crate::launcher::codex_context();
    if !ctx.disable_compact {
        env.push(("CLAUDE_CODE_AUTO_COMPACT_WINDOW", ctx.auto_compact_at.to_string()));
    }
    // Telemetry, error reports, update checks: straight to Anthropic and others from the real address.
    env.push(("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1".into()));
    // Everything else Claude Code sends (OAuth refresh, MCP over HTTP, WebFetch, the tools it
    // runs) leaves through the egress proxy; the bridge itself is reached directly.
    if let Some(url) = crate::egress::url() {
        env.push(("HTTPS_PROXY", url.clone()));
        env.push(("HTTP_PROXY", url));
        env.push(("NO_PROXY", "127.0.0.1,localhost,::1".into()));
    }
    // Commands Claude Code runs (git commits, `date`) and its own clock show the local time zone.
    if let Some(tz) = time_zone() {
        env.push(("TZ", tz));
    }
    // Their output (dates, compiler errors, git messages) goes to the model in the system language.
    if let Some(lang) = language() {
        env.push(("LANG", lang.clone()));
        env.push(("LC_ALL", lang));
    }
    env
}

pub const LOCALE_KEY: &str = "CLAUDE_BRIDGE_LOCALE";

/// `CLAUDE_BRIDGE_LOCALE`: the locale Claude Code and the commands it runs get as `LANG` and
/// `LC_ALL` (`en_US.UTF-8`, or `C.UTF-8`, which every system has; `en` means `en_US.UTF-8`).
/// Unset, or a locale the system lacks: theirs stays as is, since a missing one makes every shell
/// complain.
pub fn language() -> Option<String> {
    let value = crate::settings::read()
        .remove(LOCALE_KEY)
        .or_else(|| crate::config::env(LOCALE_KEY))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())?;
    let locale = full_locale(&value);
    if !locale_exists(&locale) {
        tracing::warn!("{LOCALE_KEY}={value}: the system has no locale {locale}; left as is");
        return None;
    }
    Some(locale)
}

/// `en` → `en_US.UTF-8`, `de` → `de_DE.UTF-8`; full names stay.
fn full_locale(value: &str) -> String {
    if value.contains(['_', '.']) || value.eq_ignore_ascii_case("C") || value.eq_ignore_ascii_case("POSIX") {
        return value.to_string();
    }
    let lang = value.to_ascii_lowercase();
    let upper = lang.to_ascii_uppercase();
    let country = match lang.as_str() {
        "en" => "US",
        "uk" => "UA",
        "sv" => "SE",
        "da" => "DK",
        "et" => "EE",
        "ja" => "JP",
        "ko" => "KR",
        "zh" => "CN",
        "cs" => "CZ",
        "el" => "GR",
        _ => &upper,
    };
    format!("{lang}_{country}.UTF-8")
}

/// Linux: whether `locale -a` lists it. Elsewhere `LANG` only tells programs the language, so
/// any value goes.
fn locale_exists(locale: &str) -> bool {
    if !cfg!(target_os = "linux") {
        return true;
    }
    let norm = |s: &str| s.to_ascii_lowercase().replace(['-', '_', '.'], "");
    let Ok(out) = std::process::Command::new("locale").arg("-a").output() else { return true };
    let want = norm(locale);
    String::from_utf8_lossy(&out.stdout).lines().any(|l| norm(l.trim()) == want)
}

pub const TZ_KEY: &str = "CLAUDE_BRIDGE_TZ";

/// `CLAUDE_BRIDGE_TZ` as written.
pub fn time_zone_setting() -> Option<String> {
    crate::settings::read()
        .remove(TZ_KEY)
        .or_else(|| crate::config::env(TZ_KEY))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// The zone Claude Code and the commands it runs get as `TZ`: a name (`UTC`, `Europe/Berlin`), or
/// with `auto` the zone of the Anthropic exit's country (see `zone`). Unset: theirs stays as is.
pub fn time_zone() -> Option<String> {
    time_zone_setting().map(|v| if v.eq_ignore_ascii_case("auto") { crate::zone::auto() } else { v })
}

fn marked(env: &Map<String, Value>) -> Vec<String> {
    env.get(MANAGED_MARKER).and_then(Value::as_str).unwrap_or_default().split(',').filter(|k| !k.is_empty()).map(String::from).collect()
}

/// Brings the managed values in line with the settings: sets missing ones, updates its own, drops
/// its own that are no longer wanted (egress turned off, say). True when `env` changed.
fn sync_managed(env: &mut Map<String, Value>) -> bool {
    let before = env.clone();
    let previous = marked(env);
    let want = managed_env();
    let mut ours = Vec::new();
    for (key, value) in &want {
        if !env.contains_key(*key) || previous.iter().any(|k| k == key) {
            env.insert((*key).into(), json!(value));
            ours.push(*key);
        }
    }
    for key in previous.iter().filter(|k| !want.iter().any(|(w, _)| w == k)) {
        env.shift_remove(key.as_str());
    }
    if ours.is_empty() {
        env.shift_remove(MANAGED_MARKER);
    } else {
        env.insert(MANAGED_MARKER.into(), json!(ours.join(",")));
    }
    *env != before
}

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
    sync_managed(env);
    sync_placeholder(env);
    write(&settings)
}

/// Keeps the placeholder token in step with Claude Code's login while connected, and the managed
/// values in place; writes only on change.
pub fn sync_auth(port: u16) -> anyhow::Result<()> {
    if connection(port) != Connection::Connected {
        return Ok(());
    }
    let mut settings = read()?;
    let Some(env) = settings.get_mut("env").and_then(Value::as_object_mut) else { return Ok(()) };
    let changed = sync_placeholder(env) | sync_managed(env);
    if changed { write(&settings) } else { Ok(()) }
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
    for key in marked(env) {
        env.shift_remove(key.as_str());
        changed = true;
    }
    if env.shift_remove(MANAGED_MARKER).is_some() {
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

#[cfg(test)]
mod locale_tests {
    use super::full_locale;

    #[test]
    fn short_codes_become_full_locales() {
        assert_eq!(full_locale("ru"), "ru_RU.UTF-8");
        assert_eq!(full_locale("en"), "en_US.UTF-8");
        assert_eq!(full_locale("C.UTF-8"), "C.UTF-8");
        assert_eq!(full_locale("de_AT.UTF-8"), "de_AT.UTF-8");
    }
}
