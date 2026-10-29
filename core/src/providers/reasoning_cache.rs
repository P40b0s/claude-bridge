//! Per-conversation cache of the Responses API's encrypted reasoning items.
//!
//! Echoing `encrypted_content` back on the next turn gives the model its prior chain of
//! thought. Claude Code knows nothing about these blobs, so conversations are keyed by a
//! hash of the first user turn. Each entry also keeps the backend's `x-codex-turn-state`
//! sticky-routing token for the user turn in progress.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::anthropic::{Block, Content, Message};

const TTL: Duration = Duration::from_secs(30 * 60);

struct Entry {
    items: Vec<Value>,
    turn_state: Option<String>,
    last_access: Instant,
}

#[derive(Default)]
pub struct ReasoningCache {
    entries: Mutex<HashMap<String, Entry>>,
}

impl ReasoningCache {
    pub fn get(&self, key: &str) -> Vec<Value> {
        let mut entries = self.entries.lock().unwrap();
        reap(&mut entries);
        match entries.get_mut(key) {
            Some(e) => {
                e.last_access = Instant::now();
                e.items.clone()
            }
            None => Vec::new(),
        }
    }

    pub fn set(&self, key: &str, items: Vec<Value>) {
        self.update(key, |e| e.items = items);
    }

    /// The turn-state token for `key`; a new user turn drops the previous turn's token.
    pub fn turn_state(&self, key: &str, new_turn: bool) -> Option<String> {
        let mut entries = self.entries.lock().unwrap();
        reap(&mut entries);
        let e = entries.get_mut(key)?;
        if new_turn {
            e.turn_state = None;
        }
        e.turn_state.clone()
    }

    pub fn set_turn_state(&self, key: &str, state: String) {
        self.update(key, |e| e.turn_state = Some(state));
    }

    fn update(&self, key: &str, f: impl FnOnce(&mut Entry)) {
        let mut entries = self.entries.lock().unwrap();
        reap(&mut entries);
        let now = Instant::now();
        let e = entries.entry(key.to_string()).or_insert_with(|| Entry { items: Vec::new(), turn_state: None, last_access: now });
        f(e);
        e.last_access = now;
    }
}

fn reap(entries: &mut HashMap<String, Entry>) {
    entries.retain(|_, e| e.last_access.elapsed() < TTL);
}

fn user_text(m: &Message) -> String {
    match &m.content {
        Content::Text(text) => text.clone(),
        Content::Blocks(blocks) => blocks
            .iter()
            .map(|b| match b {
                Block::Text { text } => text.clone(),
                Block::ToolResult { content, .. } => content.to_text(),
                _ => String::new(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// Stable key derived from the first user turn.
pub fn conversation_key(messages: &[Message]) -> String {
    let Some(first) = messages.iter().find(|m| m.role == "user") else {
        return "anon".into();
    };
    let text = user_text(first);
    let prefix = crate::error::truncate(&text, 4096);
    let digest = Sha256::digest(prefix.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    hex[..24].to_string()
}

/// UUID-shaped `prompt_cache_key` for a conversation key.
pub fn thread_id(key: &str) -> String {
    let k = format!("{key:0<24}");
    format!("{}-{}-{}-{}-{:0<12}", &k[..8], &k[8..12], &k[12..16], &k[16..20], &k[20..24])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(text: &str) -> Message {
        Message { role: "user".into(), content: Content::Text(text.into()) }
    }

    #[test]
    fn turn_state_survives_reasoning_updates_until_next_user_turn() {
        let cache = ReasoningCache::default();
        cache.set_turn_state("k", "ts".into());
        cache.set("k", vec![Value::Null]);
        assert_eq!(cache.turn_state("k", false).as_deref(), Some("ts"));
        assert_eq!(cache.turn_state("k", true), None);
        assert_eq!(cache.get("k").len(), 1);
    }

    #[test]
    fn same_first_turn_same_key() {
        let a = conversation_key(&[user("hello"), user("later")]);
        let b = conversation_key(&[user("hello")]);
        assert_eq!(a, b);
        assert_ne!(a, conversation_key(&[user("other")]));
        assert_eq!(a.len(), 24);
    }

    #[test]
    fn thread_id_is_uuid_shaped() {
        let id = thread_id(&conversation_key(&[user("hello")]));
        let parts: Vec<usize> = id.split('-').map(str::len).collect();
        assert_eq!(parts, vec![8, 4, 4, 4, 12]);
    }
}
