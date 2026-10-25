//! Local Anthropic-compatible proxy that runs Claude Code on Codex (GPT), Gemini,
//! the OpenAI API, or the Anthropic API, plus the logins and launcher around it.

pub mod anthropic;
pub mod auth;
pub mod claude_settings;
pub mod config;
pub mod control;
pub mod error;
pub mod families;
pub mod geo;
pub mod launcher;
pub mod providers;
pub mod request_log;
pub mod routing;
pub mod server;
pub mod settings;
pub mod usage;
