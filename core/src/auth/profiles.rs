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

#[cfg(test)]
mod tests {
    use super::validate;

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
