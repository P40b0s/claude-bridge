use std::path::PathBuf;

pub const DEFAULT_PORT: u16 = 17870;

pub fn port() -> u16 {
    env("CLAUDE_BRIDGE_PORT")
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

/// `~/.claude-bridge`, overridable with `CLAUDE_BRIDGE_HOME`.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = env("CLAUDE_BRIDGE_HOME") {
        return PathBuf::from(dir);
    }
    home_dir().join(".claude-bridge")
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Loads `~/.claude-bridge/.env` without overriding variables already set in the environment.
pub fn load_env_file() {
    let _ = dotenvy::from_path(data_dir().join(".env"));
}

/// Environment variable that is set and non-empty.
pub fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}
