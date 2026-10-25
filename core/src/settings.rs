//! Reads and edits `~/.claude-bridge/.env`, keeping comments and unrelated lines intact.
//! The proxy reads it at startup, so changes apply after a restart.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Context;

use crate::config;

pub fn env_file() -> PathBuf {
    config::data_dir().join(".env")
}

/// Values set in the file (not in the process environment).
pub fn read() -> BTreeMap<String, String> {
    match dotenvy::from_path_iter(env_file()) {
        Ok(iter) => iter.filter_map(Result::ok).collect(),
        Err(_) => BTreeMap::new(),
    }
}

fn quote(value: &str) -> String {
    let plain = value.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:@+,=".contains(c));
    if plain && !value.is_empty() {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

fn line_key(line: &str) -> Option<&str> {
    let line = line.trim_start();
    let line = line.strip_prefix("export ").unwrap_or(line);
    if line.starts_with('#') {
        return None;
    }
    line.split_once('=').map(|(k, _)| k.trim())
}

/// Sets each key, or removes it when the value is None or empty.
pub fn write(updates: &BTreeMap<String, Option<String>>) -> anyhow::Result<()> {
    let path = env_file();
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut pending: BTreeMap<&str, Option<&str>> = updates
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_deref().filter(|v| !v.is_empty())))
        .collect();

    let mut out = Vec::new();
    for line in existing.lines() {
        match line_key(line).and_then(|k| pending.remove_entry(k)) {
            Some((key, Some(value))) => out.push(format!("{key}={}", quote(value))),
            Some((_, None)) => {}
            None => out.push(line.to_string()),
        }
    }
    for (key, value) in pending {
        if let Some(value) = value {
            out.push(format!("{key}={}", quote(value)));
        }
    }

    let mut text = out.join("\n");
    text.push('\n');
    std::fs::create_dir_all(config::data_dir())?;
    let tmp = path.with_extension("tmp");
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        opts.open(&tmp)?.write_all(text.as_bytes())?;
    }
    std::fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_round_trips_through_dotenvy() {
        for value in ["sk-abc_123", "with space", "quote\"and\\slash", "#hash", ""] {
            let line = format!("K={}", quote(value));
            let parsed: Vec<(String, String)> =
                dotenvy::from_read_iter(line.as_bytes()).filter_map(Result::ok).collect();
            assert_eq!(parsed, vec![("K".to_string(), value.to_string())], "{line}");
        }
    }

    #[test]
    fn keys_are_found_in_export_lines_and_comments_ignored() {
        assert_eq!(line_key("export OPENAI_API_KEY=x"), Some("OPENAI_API_KEY"));
        assert_eq!(line_key("# OPENAI_API_KEY=x"), None);
        assert_eq!(line_key("  GEMINI_API_KEY = y"), Some("GEMINI_API_KEY"));
    }
}
