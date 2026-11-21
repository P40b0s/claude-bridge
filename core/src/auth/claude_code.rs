//! Claude Code account profiles. Claude Code still owns login and token refresh; the bridge only
//! snapshots and atomically switches its account-specific fields. Project history, settings, MCP
//! credentials and every other part of the Claude Code profile remain shared.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{profiles, read_json, remove_file, write_secret_json};
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

pub struct AccountProfile {
    pub id: String,
    pub account: ClaudeCodeAccount,
    pub active: bool,
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    claude_ai_oauth: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    organization_uuid: Option<Value>,
    oauth_account: Value,
}

fn config_dir() -> PathBuf {
    config::env("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| config::home_dir().join(".claude"))
}

fn config_file() -> PathBuf {
    match config::env("CLAUDE_CONFIG_DIR") {
        Some(dir) => PathBuf::from(dir).join(".claude.json"),
        None => config::home_dir().join(".claude.json"),
    }
}

fn credentials_file() -> PathBuf {
    config_dir().join(".credentials.json")
}

fn snapshot_file(profile: &str) -> anyhow::Result<PathBuf> {
    Ok(profiles::dir("claude").join(format!("{}.json", profiles::validate(profile)?)))
}

fn account_from(value: &Value) -> Option<ClaudeCodeAccount> {
    let account = value.as_object()?;
    let field = |name: &str| {
        account[name]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let plan = field("organizationType").and_then(|t| {
        let tier = t.strip_prefix("claude_")?;
        let mut name = tier.to_string();
        if !name.is_empty() {
            name[..1].make_ascii_uppercase();
        }
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
        subscription_since: field("subscriptionCreatedAt")
            .and_then(|t| crate::usage::parse_rfc3339_ms(&t)),
    })
}

fn current_snapshot() -> anyhow::Result<Snapshot> {
    let credentials: Value =
        read_json(&credentials_file()).context("Claude Code is not logged in")?;
    let config: Value =
        read_json(&config_file()).context("Claude Code account metadata is missing")?;
    let claude_ai_oauth = credentials
        .get("claudeAiOauth")
        .filter(|v| v.is_object())
        .cloned()
        .context("Claude Code OAuth credentials are missing")?;
    let oauth_account = config
        .get("oauthAccount")
        .filter(|v| v.is_object())
        .cloned()
        .context("Claude Code account metadata is missing")?;
    Ok(Snapshot {
        claude_ai_oauth,
        organization_uuid: credentials.get("organizationUuid").cloned(),
        oauth_account,
    })
}

fn write_object(path: &Path, value: Value) -> anyhow::Result<()> {
    ensure!(value.is_object(), "{} is not a JSON object", path.display());
    write_secret_json(path, &value)
}

fn apply(snapshot: &Snapshot) -> anyhow::Result<()> {
    let credentials_path = credentials_file();
    let mut credentials =
        read_json::<Value>(&credentials_path).unwrap_or_else(|| Value::Object(Map::new()));
    let credentials = credentials
        .as_object_mut()
        .context("Claude Code credentials are not a JSON object")?;
    credentials.insert("claudeAiOauth".into(), snapshot.claude_ai_oauth.clone());
    match &snapshot.organization_uuid {
        Some(value) => {
            credentials.insert("organizationUuid".into(), value.clone());
        }
        None => {
            credentials.remove("organizationUuid");
        }
    }
    write_object(&credentials_path, Value::Object(credentials.clone()))?;

    let config_path = config_file();
    let mut root = read_json::<Value>(&config_path).unwrap_or_else(|| Value::Object(Map::new()));
    let root = root
        .as_object_mut()
        .context("Claude Code configuration is not a JSON object")?;
    root.insert("oauthAccount".into(), snapshot.oauth_account.clone());
    write_object(&config_path, Value::Object(root.clone()))
}

/// The account currently installed in the shared Claude Code profile.
pub fn account() -> Option<ClaudeCodeAccount> {
    let config: Value = read_json(&config_file())?;
    account_from(config.get("oauthAccount")?)
}

pub fn active_profile() -> String {
    profiles::active(profiles::CLAUDE_ACTIVE_KEY)
}

/// Saves the shared profile's current login under `profile`.
pub fn save_current(profile: &str) -> anyhow::Result<()> {
    let path = snapshot_file(profile)?;
    write_secret_json(&path, &current_snapshot()?)
}

/// Makes a saved account current while leaving all non-account Claude Code state untouched.
pub fn activate(profile: &str) -> anyhow::Result<()> {
    let profile = profiles::validate(profile)?;
    let target: Snapshot = read_json(&snapshot_file(profile)?)
        .with_context(|| format!("Claude account '{profile}' is not saved"))?;
    let current = active_profile();
    if account().is_some() && current != profile {
        save_current(&current)?;
    }
    apply(&target)?;
    profiles::set_active(profiles::CLAUDE_ACTIVE_KEY, Some(profile))
}

/// Called after Claude Code completed a login in the shared profile.
pub fn finish_login(profile: &str) -> anyhow::Result<()> {
    let profile = profiles::validate(profile)?;
    save_current(profile)?;
    profiles::set_active(profiles::CLAUDE_ACTIVE_KEY, Some(profile))
}

/// Saves the current account before `claude auth login` changes it.
pub fn prepare_login() -> anyhow::Result<Option<String>> {
    let previous = active_profile();
    if account().is_some() {
        save_current(&previous)?;
        Ok(Some(previous))
    } else {
        Ok(None)
    }
}

pub fn restore_after_failed_login(previous: Option<&str>) -> anyhow::Result<()> {
    match previous {
        Some(profile) => activate(profile),
        None => Ok(()),
    }
}

pub fn remove(profile: &str) -> anyhow::Result<bool> {
    let profile = profiles::validate(profile)?;
    let removed = remove_file(&snapshot_file(profile)?);
    if active_profile() == profile {
        profiles::set_active(profiles::CLAUDE_ACTIVE_KEY, None)?;
    }
    Ok(removed)
}

pub fn profiles() -> Vec<AccountProfile> {
    let active = active_profile();
    // Adopt a pre-existing Claude Code login as the default profile on first use.
    if account().is_some() && snapshot_file(&active).is_ok_and(|path| !path.is_file()) {
        let _ = save_current(&active);
    }
    let Ok(entries) = std::fs::read_dir(profiles::dir("claude")) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else {
            continue;
        };
        let Some(snapshot) = read_json::<Snapshot>(&path) else {
            continue;
        };
        let Some(account) = account_from(&snapshot.oauth_account) else {
            continue;
        };
        result.push(AccountProfile {
            active: id == active,
            id,
            account,
        });
    }
    result.sort_by(|a, b| b.active.cmp(&a.active).then_with(|| a.id.cmp(&b.id)));
    result
}

/// Removes the account fields from the shared profile after `claude auth logout` succeeds.
pub fn clear_active(profile: &str) -> anyhow::Result<()> {
    let profile = profiles::validate(profile)?;
    if active_profile() != profile {
        bail!("Claude account '{profile}' is not active");
    }
    remove(profile)?;
    let next = saved_profile_ids().into_iter().find(|id| id != profile);
    if let Some(next) = next {
        activate(&next)?;
    }
    Ok(())
}

fn saved_profile_ids() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(profiles::dir("claude")) else {
        return Vec::new();
    };
    let mut ids: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension().and_then(|e| e.to_str()) == Some("json"))
                .then(|| path.file_stem()?.to_str().map(String::from))
                .flatten()
        })
        .collect();
    ids.sort();
    ids
}
