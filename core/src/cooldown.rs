//! Providers that answered "usage limit reached" with a reset time. Until then the proxy
//! answers at once instead of sending the whole conversation upstream again.

use std::collections::HashMap;
use std::sync::Mutex;

use axum::http::StatusCode;

use crate::auth::now_ms;
use crate::error::ProxyError;
use crate::routing::Provider;

#[derive(Default)]
pub struct Cooldowns(Mutex<HashMap<Provider, i64>>);

impl Cooldowns {
    /// Remembers the reset time of a limit error.
    pub fn note(&self, provider: Provider, err: &ProxyError) {
        if let Some(at) = err.retry_at {
            tracing::warn!(provider = provider.as_str(), resets_in_s = (at - now_ms()) / 1000, "usage limit reached");
            self.0.lock().unwrap().insert(provider, at);
        }
    }

    /// Err while the provider's limit has not reset yet.
    pub fn check(&self, provider: Provider) -> Result<(), ProxyError> {
        let mut limits = self.0.lock().unwrap();
        let Some(&at) = limits.get(&provider) else { return Ok(()) };
        if at <= now_ms() {
            limits.remove(&provider);
            return Ok(());
        }
        let message = format!(
            "{} usage limit until {} (resets in {}): pick another model with /model",
            label(provider),
            local_time(at),
            duration(at - now_ms()),
        );
        Err(ProxyError { retry_at: Some(at), ..ProxyError::new(StatusCode::TOO_MANY_REQUESTS, message) })
    }
}

fn label(provider: Provider) -> &'static str {
    match provider {
        Provider::CodexOauth => "ChatGPT",
        Provider::Openai => "OpenAI API",
        Provider::OpenaiCompat => "OpenAI-compatible API",
        Provider::GeminiOauth | Provider::Gemini => "Gemini",
        Provider::Anthropic => "Anthropic",
    }
}

/// `17:46`, or `26.10 09:15` when the reset is not today.
fn local_time(at_ms: i64) -> String {
    let tz = jiff::tz::TimeZone::system();
    let Ok(at) = jiff::Timestamp::from_millisecond(at_ms) else { return at_ms.to_string() };
    let at = at.to_zoned(tz.clone());
    let today = jiff::Timestamp::now().to_zoned(tz).date();
    at.strftime(if at.date() == today { "%H:%M" } else { "%d.%m %H:%M" }).to_string()
}

fn duration(ms: i64) -> String {
    let minutes = (ms + 59_999) / 60_000;
    match minutes {
        m if m >= 60 => format!("{}h {}m", m / 60, m % 60),
        m => format!("{m}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_until_reset() {
        let cooldowns = Cooldowns::default();
        let limited = ProxyError { retry_at: Some(now_ms() + 3_600_000), ..ProxyError::new(StatusCode::TOO_MANY_REQUESTS, "") };
        cooldowns.note(Provider::CodexOauth, &limited);
        let err = cooldowns.check(Provider::CodexOauth).unwrap_err();
        assert_eq!(err.status, StatusCode::TOO_MANY_REQUESTS);
        assert!(err.message.starts_with("ChatGPT usage limit until "), "{}", err.message);
        assert!(cooldowns.check(Provider::Gemini).is_ok());

        let expired = ProxyError { retry_at: Some(now_ms() - 1), ..limited };
        cooldowns.note(Provider::CodexOauth, &expired);
        assert!(cooldowns.check(Provider::CodexOauth).is_ok());
    }

    #[test]
    fn durations() {
        assert_eq!(duration(9_256_000), "2h 35m");
        assert_eq!(duration(30_000), "1m");
    }
}
