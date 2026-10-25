//! Claude Code's own login, read-only. Only the account record in its config file is read,
//! never its credentials: the proxy just relays Claude Code's requests.

use std::path::PathBuf;

use serde_json::Value;

use crate::config;

pub struct ClaudeCodeAccount {
    pub email: Option<String>,
    pub organization: Option<String>,
    /// `Pro`, `Max`, `Team`…; None for an API-key (Console) account.
    pub plan: Option<String>,
    /// How the subscription is paid: `App Store`, `Google Play`, `card`.
    pub billing: Option<String>,
    /// Unix ms. Claude Code does not record when the subscription ends.
    pub subscription_since: Option<i64>,
}

fn config_file() -> PathBuf {
    match config::env("CLAUDE_CONFIG_DIR") {
        Some(dir) => PathBuf::from(dir).join(".claude.json"),
        None => config::home_dir().join(".claude.json"),
    }
}

/// The account Claude Code is logged in with (`/login`), if any.
pub fn account() -> Option<ClaudeCodeAccount> {
    let config: Value = serde_json::from_slice(&std::fs::read(config_file()).ok()?).ok()?;
    let account = config.get("oauthAccount").filter(|a| a.is_object())?;
    let field = |name: &str| account[name].as_str().filter(|s| !s.is_empty()).map(String::from);
    let plan = field("organizationType").and_then(|t| {
        let tier = t.strip_prefix("claude_")?;
        let mut name = tier.to_string();
        name[..1].make_ascii_uppercase();
        Some(name)
    });
    let billing = field("billingType").map(|b| match b.as_str() {
        "apple_subscription" => "App Store".into(),
        "google_play_subscription" => "Google Play".into(),
        "stripe_subscription" => "card".into(),
        other => other.trim_end_matches("_subscription").replace('_', " "),
    });
    Some(ClaudeCodeAccount {
        email: field("emailAddress"),
        organization: field("organizationName"),
        plan,
        billing,
        subscription_since: field("subscriptionCreatedAt").and_then(|t| crate::usage::parse_rfc3339_ms(&t)),
    })
}
