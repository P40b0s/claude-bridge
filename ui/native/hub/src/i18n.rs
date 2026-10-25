//! UI language for messages Rust sends to Dart. Dart writes the choice to the .env file
//! (`CLAUDE_BRIDGE_LANG`, `ru` or `en`); until then the system locale decides.

use crate::configuration;

pub const LANG_KEY: &str = "CLAUDE_BRIDGE_LANG";

pub fn is_russian() -> bool {
    match configuration::setting(LANG_KEY) {
        Some(lang) => lang.eq_ignore_ascii_case("ru"),
        None => ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
            .is_some_and(|v| v.starts_with("ru")),
    }
}

/// The Russian or the English text, by the UI language.
pub fn tr<'a>(ru: &'a str, en: &'a str) -> &'a str {
    if is_russian() { ru } else { en }
}
