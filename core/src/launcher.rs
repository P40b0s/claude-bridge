//! Starts (or reuses) the background proxy and replaces this process with Claude Code.
//!
//! The proxy runs detached, so it outlives the session that started it and is shared by
//! every later session and the UI. `claude-bridge stop` ends it.

use std::process::Command;

use anyhow::{Context, bail};

use crate::providers::anthropic::PLACEHOLDER_TOKEN;
use crate::auth::claude_code;
use crate::{config, control, settings};

pub struct ContextWindow {
    pub tokens: u64,
    pub auto_compact_at: u64,
    pub disable_compact: bool,
}

pub struct Launch {
    /// `provider:model` the proxy falls back to before the first request picks one.
    pub proxy_default: String,
    /// `--model` passed to Claude Code; None keeps Claude Code's own default.
    pub claude_model: Option<String>,
    pub context: Option<ContextWindow>,
    pub skip_permissions: bool,
    pub restart: bool,
    pub claude_args: Vec<String>,
}

pub fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        let m = tokens as f64 / 1_000_000.0;
        if m.fract() == 0.0 { format!("{m:.0}M") } else { format!("{m:.1}M") }
    } else if tokens >= 1_000 {
        format!("{}k", (tokens as f64 / 1_000.0).round())
    } else {
        tokens.to_string()
    }
}

fn set_default_env(cmd: &mut Command, name: &str, value: &str) {
    if config::env(name).is_none() {
        cmd.env(name, value);
    }
}

/// Starts the proxy if needed, then execs `claude` pointed at it.
pub async fn launch(launch: Launch) -> anyhow::Result<()> {
    let port = config::port();
    let was_running = control::health(port).await.is_some();
    let health = control::ensure_proxy(&std::env::current_exe()?, port, &launch.proxy_default, launch.restart).await?;
    let model = |t: &Option<crate::routing::Target>| {
        t.as_ref().map(|t| format!("{}:{}", t.provider.as_str(), t.model)).unwrap_or_else(|| "-".into())
    };
    if was_running && !launch.restart {
        eprintln!("  Proxy already running (PID {}, {})", health.pid, model(&health.active));
    } else {
        eprintln!("  Proxy started on {} (log: {})", control::base_url(port), control::log_path().display());
    }
    if health.pinned.is_some() {
        eprintln!("  Model pinned to {} (claude-bridge model --clear to follow /model)", model(&health.pinned));
    }

    // settings.json's env applies to this claude too, so its placeholder token must match the login.
    if let Err(e) = crate::claude_settings::sync_auth(port) {
        eprintln!("  Warning: {e:#}");
    }
    let mut cmd = Command::new("claude");
    if let Some(model) = &launch.claude_model {
        cmd.args(["--model", model]);
    }
    if launch.skip_permissions {
        cmd.arg("--dangerously-skip-permissions");
    }
    cmd.args(&launch.claude_args).env("ANTHROPIC_BASE_URL", control::base_url(port));
    // Claude Code keeps its own login, so Claude models (`/model opus`) reach Claude with it.
    // Without one it gets a placeholder token, and Claude models use ANTHROPIC_API_KEY.
    match claude_code::account() {
        Some(account) => {
            // The proxy's key from .env would take over Claude Code's login; one the user exported stays.
            if settings::read().get("ANTHROPIC_API_KEY").is_some_and(|k| config::env("ANTHROPIC_API_KEY").as_ref() == Some(k)) {
                cmd.env_remove("ANTHROPIC_API_KEY");
            }
            eprintln!("  Claude: Claude Code login ({})", account.email.as_deref().unwrap_or("unknown account"));
        }
        None => set_default_env(&mut cmd, "ANTHROPIC_AUTH_TOKEN", PLACEHOLDER_TOKEN),
    }
    // `/model opus` is then an explicit pick of Claude rather than a bare claude-* name, which the
    // proxy treats as an internal call. Sonnet and haiku stay bare: Claude Code uses them internally.
    set_default_env(&mut cmd, "ANTHROPIC_DEFAULT_OPUS_MODEL", "anthropic:claude-opus-5-5");

    // Tell Claude Code the provider's real window so it compacts before the upstream rejects.
    if let Some(ctx) = &launch.context {
        set_default_env(&mut cmd, "CLAUDE_CODE_MAX_CONTEXT_TOKENS", &ctx.tokens.to_string());
        set_default_env(&mut cmd, "CLAUDE_CODE_AUTO_COMPACT_WINDOW", &ctx.auto_compact_at.to_string());
        if ctx.disable_compact {
            set_default_env(&mut cmd, "DISABLE_COMPACT", "1");
        }
        let compact = if ctx.disable_compact {
            "auto-compact disabled".to_string()
        } else {
            format!("auto-compact at {}", format_tokens(ctx.auto_compact_at))
        };
        eprintln!("  Context window: {} tokens ({compact})", format_tokens(ctx.tokens));
    }
    eprintln!();

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = cmd.exec();
        if err.kind() == std::io::ErrorKind::NotFound {
            bail!("`claude` not found on PATH. Install Claude Code first.");
        }
        Err(err).context("launching claude")
    }
    #[cfg(not(unix))]
    {
        let status = cmd.status().context("launching claude (is Claude Code installed?)")?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

#[cfg(test)]
mod tests {
    use super::format_tokens;

    #[test]
    fn token_counts() {
        assert_eq!(format_tokens(1_000_000), "1M");
        assert_eq!(format_tokens(1_500_000), "1.5M");
        assert_eq!(format_tokens(372_000), "372k");
        assert_eq!(format_tokens(353_400), "353k");
        assert_eq!(format_tokens(999), "999");
    }
}
