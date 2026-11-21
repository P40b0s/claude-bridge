//! Starts (or reuses) the background proxy and replaces this process with Claude Code.
//!
//! The proxy runs detached, so it outlives the session that started it and is shared by
//! every later session and the UI. `claude-bridge stop` ends it.

use std::process::Command;

use anyhow::{Context, bail};

use crate::auth::claude_code;
use crate::providers::anthropic::PLACEHOLDER_TOKEN;
use crate::{config, control, settings};

pub struct ContextWindow {
    pub tokens: u64,
    pub auto_compact_at: u64,
    pub disable_compact: bool,
}

/// GPT-5.6 family window.
const CODEX_WINDOW: u64 = 372_000;
/// Where Claude Code compacts by default on GPT. Every tool call re-sends the whole context, cached
/// or not, so a long session past this point costs the subscription more per turn than the
/// summary loses.
pub const CODEX_COMPACT_AT: u64 = 200_000;

/// The window `claude-bridge codex` reports, from `CODEX_CONTEXT_WINDOW_TOKENS`,
/// `CODEX_AUTO_COMPACT_WINDOW_TOKENS` and `CODEX_DISABLE_COMPACT`: the .env file first (the UI
/// edits it while running and syncs settings.json from it), then the process environment.
pub fn codex_context() -> ContextWindow {
    let file = settings::read();
    let get = |name: &str| {
        file.get(name)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| config::env(name))
    };
    let num = |name: &str| {
        get(name)
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|&v| v > 0)
    };
    let tokens = num("CODEX_CONTEXT_WINDOW_TOKENS").unwrap_or(CODEX_WINDOW);
    ContextWindow {
        tokens,
        auto_compact_at: num("CODEX_AUTO_COMPACT_WINDOW_TOKENS")
            .unwrap_or(CODEX_COMPACT_AT)
            .min(tokens),
        disable_compact: get("CODEX_DISABLE_COMPACT").as_deref() == Some("1"),
    }
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
        if m.fract() == 0.0 {
            format!("{m:.0}M")
        } else {
            format!("{m:.1}M")
        }
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
    let health = control::ensure_proxy(
        &std::env::current_exe()?,
        port,
        &launch.proxy_default,
        launch.restart,
    )
    .await?;
    let model = |t: &Option<crate::routing::Target>| {
        t.as_ref()
            .map(|t| format!("{}:{}", t.provider.as_str(), t.model))
            .unwrap_or_else(|| "-".into())
    };
    if was_running && !launch.restart {
        eprintln!(
            "  Proxy already running (PID {}, {})",
            health.pid,
            model(&health.active)
        );
    } else {
        eprintln!(
            "  Proxy started on {} (log: {})",
            control::base_url(port),
            control::log_path().display()
        );
    }
    if health.pinned.is_some() {
        eprintln!(
            "  Model pinned to {} (claude-bridge model --clear to follow /model)",
            model(&health.pinned)
        );
    }

    // `CLAUDE_BRIDGE_TZ=auto`: the zone of the exit's country as it is now, not as last seen.
    if crate::claude_settings::time_zone_setting().is_some_and(|v| v.eq_ignore_ascii_case("auto")) {
        match crate::zone::locate_outside().await {
            Ok(location) => crate::zone::remember(&location),
            Err(e) => eprintln!(
                "  Warning: exit country unknown, time zone {}: {e:#}",
                crate::zone::auto()
            ),
        }
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
    cmd.args(&launch.claude_args)
        .env("ANTHROPIC_BASE_URL", control::base_url(port));
    // Claude Code keeps its own login, so Claude models (`/model opus`) reach Claude with it.
    // Without one it gets a placeholder token, and Claude models use ANTHROPIC_API_KEY.
    match claude_code::account() {
        Some(account) => {
            // The proxy's key from .env would take over Claude Code's login; one the user exported stays.
            if settings::read()
                .get("ANTHROPIC_API_KEY")
                .is_some_and(|k| config::env("ANTHROPIC_API_KEY").as_ref() == Some(k))
            {
                cmd.env_remove("ANTHROPIC_API_KEY");
            }
            eprintln!(
                "  Claude: Claude Code login ({})",
                account.email.as_deref().unwrap_or("unknown account")
            );
        }
        None => set_default_env(&mut cmd, "ANTHROPIC_AUTH_TOKEN", PLACEHOLDER_TOKEN),
    }
    // `/model opus` is then an explicit pick of Claude rather than a bare claude-* name, which the
    // proxy treats as an internal call. Sonnet and haiku stay bare: Claude Code uses them internally.
    set_default_env(
        &mut cmd,
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "anthropic:claude-opus-5-5",
    );

    // No telemetry, and Claude Code's other traffic through the egress proxy (see `claude_settings`).
    for (key, value) in crate::claude_settings::managed_env() {
        if !matches!(
            key,
            "CLAUDE_CODE_AUTO_COMPACT_WINDOW" | "CLAUDE_CODE_MAX_CONTEXT_TOKENS"
        ) {
            set_default_env(&mut cmd, key, &value);
        }
    }

    // Tell Claude Code the provider's real window so it compacts before the upstream rejects.
    if let Some(ctx) = &launch.context {
        set_default_env(
            &mut cmd,
            "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
            &ctx.tokens.to_string(),
        );
        set_default_env(
            &mut cmd,
            "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
            &ctx.auto_compact_at.to_string(),
        );
        if ctx.disable_compact {
            set_default_env(&mut cmd, "DISABLE_COMPACT", "1");
        }
        let compact = if ctx.disable_compact {
            "auto-compact disabled".to_string()
        } else {
            format!("auto-compact at {}", format_tokens(ctx.auto_compact_at))
        };
        eprintln!(
            "  Context window: {} tokens ({compact})",
            format_tokens(ctx.tokens)
        );
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
        let status = cmd
            .status()
            .context("launching claude (is Claude Code installed?)")?;
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
