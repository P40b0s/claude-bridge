//! Tokens spent, by day and model: `~/.claude-bridge/stats/YYYY-MM-DD.json`, one file per local
//! day, updated after every request whose usage is known.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::anthropic::Usage;
use crate::config;

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tokens {
    pub requests: u64,
    /// Input not served from the cache.
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
}

impl Tokens {
    pub fn add(&mut self, other: &Tokens) {
        self.requests += other.requests;
        self.input += other.input;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.output += other.output;
    }

    fn of(usage: &Usage) -> Self {
        Self {
            requests: 1,
            input: usage.input_tokens,
            cache_read: usage.cache_read_input_tokens,
            cache_write: usage.cache_creation_input_tokens,
            output: usage.output_tokens,
        }
    }
}

/// One day: `provider:model` → tokens.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Day {
    #[serde(default)]
    pub models: BTreeMap<String, Tokens>,
}

impl Day {
    pub fn total(&self) -> Tokens {
        let mut t = Tokens::default();
        self.models.values().for_each(|m| t.add(m));
        t
    }
}

pub fn dir() -> PathBuf {
    config::data_dir().join("stats")
}

fn path(date: Date) -> PathBuf {
    dir().join(format!("{date}.json"))
}

pub fn today() -> Date {
    jiff::Zoned::now().date()
}

pub fn load(date: Date) -> Day {
    std::fs::read_to_string(path(date)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Adds one finished request to today's file.
pub fn record(provider: &str, model: &str, usage: &Usage) {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap();
    let date = today();
    let mut day = load(date);
    day.models.entry(format!("{provider}:{model}")).or_default().add(&Tokens::of(usage));
    if let Err(e) = write(date, &day) {
        warn!("stats: {e:#}");
    }
}

fn write(date: Date, day: &Day) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir())?;
    let file = path(date);
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(day)? + "\n")?;
    std::fs::rename(&tmp, &file)?;
    Ok(())
}

/// Every day from `from` to `to` inclusive, empty days included.
pub fn range(from: Date, to: Date) -> Vec<(Date, Day)> {
    let (from, to) = if from <= to { (from, to) } else { (to, from) };
    let mut out = Vec::new();
    let mut date = from;
    while date <= to && out.len() < 3700 {
        out.push((date, load(date)));
        match date.tomorrow() {
            Ok(next) => date = next,
            Err(_) => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_models_and_reads_old_files() {
        let day: Day = serde_json::from_str(r#"{"models":{"a:x":{"requests":2,"input":10,"output":5},"b:y":{"requests":1,"cache_read":100}}}"#).unwrap();
        let t = day.total();
        assert_eq!((t.requests, t.input, t.cache_read, t.output), (3, 10, 100, 5));
    }
}
