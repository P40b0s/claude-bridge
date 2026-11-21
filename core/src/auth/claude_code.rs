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
        account
            .get(name)
            .and_then(Value::as_str)
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

/// An existing Claude Code JSON file. A missing file is `None`; one that cannot be read or parsed
/// is an error, so activation never replaces unrelated settings with an empty object.
/// A file's original bytes, to restore it, and its parsed object.
type Existing = (Vec<u8>, Map<String, Value>);

fn read_existing(path: &Path) -> anyhow::Result<Option<Existing>> {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let value: Value = serde_json::from_slice(&data)
        .with_context(|| format!("{} is not valid JSON; fix or remove it", path.display()))?;
    let Value::Object(map) = value else {
        bail!("{} is not a JSON object", path.display());
    };
    Ok(Some((data, map)))
}

/// Puts back what a file held before a failed activation.
fn restore(path: &Path, original: Option<Vec<u8>>) -> anyhow::Result<()> {
    match original {
        Some(data) => {
            let value: Value = serde_json::from_slice(&data)?;
            write_secret_json(path, &value)
        }
        None => {
            remove_file(path);
            Ok(())
        }
    }
}

fn apply(snapshot: &Snapshot) -> anyhow::Result<()> {
    ensure!(
        snapshot.claude_ai_oauth.is_object() && snapshot.oauth_account.is_object(),
        "the saved Claude account is damaged"
    );
    // Both documents are prepared before either is written.
    let credentials_path = credentials_file();
    let (credentials_original, mut credentials) = match read_existing(&credentials_path)? {
        Some((data, map)) => (Some(data), map),
        None => (None, Map::new()),
    };
    credentials.insert("claudeAiOauth".into(), snapshot.claude_ai_oauth.clone());
    match &snapshot.organization_uuid {
        Some(value) => {
            credentials.insert("organizationUuid".into(), value.clone());
        }
        None => {
            credentials.remove("organizationUuid");
        }
    }

    let config_path = config_file();
    let mut root = read_existing(&config_path)?
        .map(|(_, map)| map)
        .unwrap_or_default();
    root.insert("oauthAccount".into(), snapshot.oauth_account.clone());

    write_secret_json(&credentials_path, &Value::Object(credentials))?;
    if let Err(e) = write_secret_json(&config_path, &Value::Object(root)) {
        // Credentials of one account with metadata of another would be worse than either.
        if let Err(restore_error) = restore(&credentials_path, credentials_original) {
            return Err(e.context(format!(
                "restoring {} also failed: {restore_error:#}",
                credentials_path.display()
            )));
        }
        return Err(e);
    }
    Ok(())
}

/// Which account and organization a login belongs to, when Claude Code recorded it.
fn identity(oauth_account: &Value) -> Option<(Option<&str>, Option<&str>)> {
    let field = |name| {
        oauth_account
            .get(name)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    };
    let account = field("accountUuid").or_else(|| field("emailAddress"));
    let organization = field("organizationUuid");
    (account.is_some() || organization.is_some()).then_some((account, organization))
}

/// Saves the shared profile's login under `profile`, unless that profile holds a different
/// account: then the login was changed outside the bridge (`claude auth login`, `/login`), and it
/// is saved under a new name instead of overwriting the other account.
fn save_current_checked(profile: &str) -> anyhow::Result<String> {
    let current = current_snapshot()?;
    let path = snapshot_file(profile)?;
    let saved = read_json::<Snapshot>(&path);
    let same = match (&saved, identity(&current.oauth_account)) {
        (Some(saved), Some(now)) => identity(&saved.oauth_account).is_none_or(|was| was == now),
        _ => true,
    };
    if same {
        write_secret_json(&path, &current)?;
        return Ok(profile.to_string());
    }
    let base = current
        .oauth_account
        .get("emailAddress")
        .and_then(Value::as_str)
        .and_then(|email| email.split('@').next())
        .map(|name| {
            name.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                        c
                    } else {
                        '_'
                    }
                })
                .take(48)
                .collect::<String>()
        })
        .filter(|name| profiles::validate(name).is_ok())
        .unwrap_or_else(|| "account".into());
    let existing: Vec<String> = saved_profile_ids();
    let taken = |id: &str| existing.iter().any(|e| e.eq_ignore_ascii_case(id));
    let id = std::iter::once(base.clone())
        .chain((2..).map(|n| format!("{base}-{n}")))
        .find(|id| !taken(id))
        .expect("an unused profile name");
    write_secret_json(&snapshot_file(&id)?, &current)?;
    Ok(id)
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
    activate_with(profile, true)
}

fn activate_with(profile: &str, save_previous: bool) -> anyhow::Result<()> {
    let profile = profiles::validate(profile)?;
    let target: Snapshot = read_json(&snapshot_file(profile)?)
        .with_context(|| format!("Claude account '{profile}' is not saved"))?;
    let current = active_profile();
    if save_previous && account().is_some() && current != profile {
        save_current_checked(&current)?;
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

/// Saves the current account before `claude auth login` changes it. Returns the profile to
/// restore if the login fails.
pub fn prepare_login(profile: &str) -> anyhow::Result<Option<String>> {
    profiles::ensure_distinct("claude", profile)?;
    if account().is_some() {
        Ok(Some(save_current_checked(&active_profile())?))
    } else {
        Ok(None)
    }
}

pub fn restore_after_failed_login(previous: Option<&str>) -> anyhow::Result<()> {
    match previous {
        // `prepare_login` already saved it; what the failed login left is not worth keeping.
        Some(profile) => activate_with(profile, false),
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
        // Whatever `claude auth logout` left behind is not an account to save.
        activate_with(&next, false)?;
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

#[cfg(test)]
mod tests {
    use super::{account_from, identity};
    use serde_json::json;

    #[test]
    fn identity_tells_accounts_apart() {
        let a = json!({"accountUuid": "a", "organizationUuid": "o", "emailAddress": "a@x"});
        let b = json!({"accountUuid": "b", "organizationUuid": "o"});
        assert_eq!(identity(&a), Some((Some("a"), Some("o"))));
        assert_ne!(identity(&a), identity(&b));
        assert_eq!(identity(&json!({})), None);
    }

    #[test]
    fn account_allows_missing_optional_fields() {
        let account = account_from(&json!({"emailAddress": "test@example.com"})).unwrap();
        assert_eq!(account.email.as_deref(), Some("test@example.com"));
        assert!(account.organization.is_none());
        assert!(account.plan.is_none());
        assert!(account.billing.is_none());
        assert!(account.subscription_since.is_none());
    }
}
