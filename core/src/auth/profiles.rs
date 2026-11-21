//! Named account profiles shared by the Claude Code and ChatGPT authentication backends.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{bail, ensure};

use crate::{config, settings};

pub const CLAUDE_ACTIVE_KEY: &str = "CLAUDE_BRIDGE_ACTIVE_CLAUDE_ACCOUNT";
pub const CHATGPT_ACTIVE_KEY: &str = "CLAUDE_BRIDGE_ACTIVE_CHATGPT_ACCOUNT";
pub const DEFAULT_PROFILE: &str = "default";

/// Profile names are also file names. Keeping the accepted alphabet small prevents traversal and
/// makes the same name safe to use for an isolated browser profile.
pub fn validate(name: &str) -> anyhow::Result<&str> {
    let name = name.trim();
    ensure!(!name.is_empty(), "account name is empty");
    ensure!(
        name.len() <= 64,
        "account name is longer than 64 characters"
    );
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        || name == "."
        || name == ".."
    {
        bail!("account name may contain only letters, digits, '.', '-' and '_'");
    }
    Ok(name)
}

pub fn active(key: &str) -> String {
    settings::read()
        .remove(key)
        .or_else(|| config::env(key))
        .filter(|v| validate(v).is_ok())
        .unwrap_or_else(|| DEFAULT_PROFILE.into())
}

pub fn set_active(key: &str, profile: Option<&str>) -> anyhow::Result<()> {
    let value = profile.map(validate).transpose()?.map(String::from);
    settings::write(&BTreeMap::from([(key.to_string(), value)]))
}

pub fn dir(provider: &str) -> PathBuf {
    config::data_dir().join("accounts").join(provider)
}

/// Rejects a profile name that differs only in case from an existing one of `provider`. On
/// case-insensitive filesystems (Windows, default macOS) both would share one file and one
/// browser profile.
pub fn ensure_distinct(provider: &str, name: &str) -> anyhow::Result<()> {
    let name = validate(name)?;
    let Ok(entries) = std::fs::read_dir(dir(provider)) else {
        return Ok(());
    };
    let existing = entries.flatten().filter_map(|entry| {
        let path = entry.path();
        (path.extension().and_then(|e| e.to_str()) == Some("json"))
            .then(|| path.file_stem()?.to_str().map(String::from))
            .flatten()
    });
    check_distinct(name, existing.chain([DEFAULT_PROFILE.to_string()]))
}

fn check_distinct(name: &str, existing: impl IntoIterator<Item = String>) -> anyhow::Result<()> {
    for other in existing {
        if other != name && other.eq_ignore_ascii_case(name) {
            bail!("account '{other}' already exists; names differing only in case are not allowed");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{check_distinct, validate};

    #[test]
    fn case_only_differences_are_rejected() {
        let existing = || ["default".to_string(), "work".to_string()];
        assert!(check_distinct("work", existing()).is_ok());
        assert!(check_distinct("other", existing()).is_ok());
        assert!(check_distinct("Work", existing()).is_err());
        assert!(check_distinct("DEFAULT", existing()).is_err());
    }

    #[test]
    fn profile_names_are_safe_file_names() {
        for valid in ["default", "work-account", "account_2", "john.doe"] {
            assert_eq!(validate(valid).unwrap(), valid);
        }
        for invalid in ["", ".", "..", "../other", "with space", "кириллица", "a/b"] {
            assert!(validate(invalid).is_err(), "{invalid}");
        }
    }
}
