use std::error::Error as _;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

/// An error that is reported to Claude Code in the Anthropic error format.
#[derive(Debug)]
pub struct ProxyError {
    pub status: StatusCode,
    pub message: String,
}

/// Non-standard status used internally when the client hung up mid-stream.
const CLIENT_CLOSED: u16 = 499;

/// Status for a provider rejecting our credentials. Claude Code treats 401/403 as its own
/// login failing and drops the session, and retries 5xx, so neither fits.
const PROVIDER_AUTH: StatusCode = StatusCode::FAILED_DEPENDENCY;

impl ProxyError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self { status, message: message.into() }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    /// Missing or rejected provider credentials.
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(PROVIDER_AUTH, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }

    /// Error status returned by an upstream provider; the status is passed through,
    /// except auth failures (see `PROVIDER_AUTH`).
    pub fn upstream(provider: &str, status: u16, body: &str) -> Self {
        let message = format!("{provider} {status}: {}", truncate(body, 1000));
        let status = match StatusCode::from_u16(status) {
            Ok(StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) => PROVIDER_AUTH,
            Ok(s) if s.is_client_error() || s.is_server_error() => s,
            _ => StatusCode::BAD_GATEWAY,
        };
        Self::new(status, message)
    }

    pub fn client_gone() -> Self {
        Self::new(StatusCode::from_u16(CLIENT_CLOSED).unwrap(), "client disconnected")
    }

    pub fn is_client_gone(&self) -> bool {
        self.status.as_u16() == CLIENT_CLOSED
    }

    fn kind(&self) -> &'static str {
        match self.status.as_u16() {
            400 => "invalid_request_error",
            401 => "authentication_error",
            403 => "permission_error",
            404 => "not_found_error",
            413 => "request_too_large",
            429 => "rate_limit_error",
            503 | 529 => "overloaded_error",
            _ => "api_error",
        }
    }

    pub fn body(&self) -> Value {
        json!({
            "type": "error",
            "error": { "type": self.kind(), "message": self.message },
        })
    }
}

impl std::fmt::Display for ProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.status.as_u16())
    }
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body())).into_response()
    }
}

impl From<reqwest::Error> for ProxyError {
    /// reqwest hides the real cause (connection reset, DNS, TLS) in the source chain.
    fn from(err: reqwest::Error) -> Self {
        let mut message = err.to_string();
        let mut source = err.source();
        while let Some(cause) = source {
            message.push_str(": ");
            message.push_str(&cause.to_string());
            source = cause.source();
        }
        let status = if err.is_timeout() { StatusCode::GATEWAY_TIMEOUT } else { StatusCode::BAD_GATEWAY };
        Self::new(status, message)
    }
}

impl From<serde_json::Error> for ProxyError {
    fn from(err: serde_json::Error) -> Self {
        Self::bad_request(format!("invalid request body: {err}"))
    }
}

pub fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}
