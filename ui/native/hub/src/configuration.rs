//! Settings as the UI sees them: the .env file first (it may have changed since the app
//! started), then the process environment.

use std::path::PathBuf;

use claude_bridge_core::{config, control, settings};

/// Keys the settings page knows about; used to report values that come only from the environment.
pub const KNOWN_KEYS: &[&str] = &[
    "OPENAI_API_KEY",
    "OPENAI_COMPAT_BASE_URL",
    "OPENAI_COMPAT_API_KEY",
    "OPENAI_COMPAT_DEFAULT_MODEL",
    "GEMINI_API_KEY",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_UPSTREAM_URL",
    "GEMINI_BASE_URL",
    "CLAUDE_BRIDGE_PORT",
    "CLAUDE_BRIDGE_BIN",
    "CLAUDE_BRIDGE_AUTOSTART",
    "CLAUDE_BRIDGE_DISABLED_PROVIDERS",
    "CLAUDE_BRIDGE_LANG",
    "CODEX_REASONING_EFFORT",
    "CODEX_REASONING_SUMMARY",
    "CODEX_TEXT_VERBOSITY",
    "CODEX_CONTEXT_WINDOW_TOKENS",
    "CODEX_AUTO_COMPACT_WINDOW_TOKENS",
    "CODEX_DISABLE_COMPACT",
    "CODEX_DISABLE_REASONING_CACHE",
];

pub fn setting(key: &str) -> Option<String> {
    settings::read()
        .remove(key)
        .filter(|v| !v.trim().is_empty())
        .or_else(|| config::env(key))
}

pub fn port() -> u16 {
    setting("CLAUDE_BRIDGE_PORT")
        .and_then(|p| p.parse().ok())
        .unwrap_or(config::DEFAULT_PORT)
}

pub fn cli_path() -> Option<PathBuf> {
    setting("CLAUDE_BRIDGE_BIN")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .or_else(control::find_cli)
}

/// On unless CLAUDE_BRIDGE_AUTOSTART is `0`.
pub fn autostart() -> bool {
    setting("CLAUDE_BRIDGE_AUTOSTART").as_deref() != Some("0")
}
