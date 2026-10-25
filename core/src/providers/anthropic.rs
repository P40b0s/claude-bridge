//! Passthrough to the Anthropic API (or any Anthropic-compatible upstream). The body is
//! forwarded untouched apart from the model, and the reply is piped back as-is, so
//! streaming, non-streaming and error responses keep their native format.
//!
//! Credentials: when Claude Code has its own login, its request is relayed with its own
//! headers, like an HTTP proxy would; the proxy never stores or reuses them. Otherwise
//! (the launcher's placeholder token) `ANTHROPIC_API_KEY` is used.

use axum::body::Body;
use axum::http::HeaderMap;
use axum::response::Response;
use futures_util::StreamExt;
use serde_json::{Value, json};

use crate::anthropic::Usage;
use crate::config;
use crate::error::ProxyError;
use crate::server::AppState;
use crate::usage;

const DEFAULT_UPSTREAM: &str = "https://api.anthropic.com";
const DEFAULT_VERSION: &str = "2023-06-01";

/// Placeholder token the launcher gives Claude Code when it has no login of its own.
pub const PLACEHOLDER_TOKEN: &str = "claude-bridge";

/// Request headers not relayed upstream: per-hop ones, and those reqwest sets itself.
const SKIPPED_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "connection",
    "keep-alive",
    "transfer-encoding",
    "te",
    "upgrade",
    "proxy-connection",
    // The reply is piped through as-is, so it must not come back compressed.
    "accept-encoding",
];

/// Whether Claude Code sent its own Anthropic credentials rather than the launcher's placeholder.
pub fn has_client_credentials(headers: &HeaderMap) -> bool {
    let real = |v: &str| !v.trim().is_empty() && v.trim() != PLACEHOLDER_TOKEN;
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(real);
    let api_key = headers.get("x-api-key").and_then(|v| v.to_str().ok()).is_some_and(real);
    bearer || api_key
}

pub async fn passthrough(
    state: &AppState,
    mut body: Value,
    model: &str,
    client: &HeaderMap,
    query: Option<&str>,
    on_usage: impl FnOnce(Usage) + Send + 'static,
) -> Result<Response, ProxyError> {
    let base = config::env("ANTHROPIC_UPSTREAM_URL").unwrap_or_else(|| DEFAULT_UPSTREAM.into());
    let mut url = format!("{}/v1/messages", base.trim_end_matches('/'));
    if let Some(query) = query {
        url = format!("{url}?{query}");
    }
    body["model"] = json!(model);
    let mut rb = state.http.post(url).json(&body);

    if has_client_credentials(client) {
        for (name, value) in client {
            if !SKIPPED_HEADERS.contains(&name.as_str()) {
                rb = rb.header(name, value);
            }
        }
    } else {
        let key = config::env("ANTHROPIC_API_KEY").ok_or_else(|| {
            ProxyError::unauthorized(
                "No Anthropic credentials: log in to Claude Code (/login) or set ANTHROPIC_API_KEY \
                 (environment or ~/.claude-bridge/.env)",
            )
        })?;
        let header = |name: &str| client.get(name).and_then(|v| v.to_str().ok()).map(String::from);
        let version = header("anthropic-version")
            .or_else(|| config::env("ANTHROPIC_VERSION"))
            .unwrap_or_else(|| DEFAULT_VERSION.into());
        rb = rb.header("x-api-key", key).header("anthropic-version", version);
        if let Some(beta) = header("anthropic-beta") {
            rb = rb.header("anthropic-beta", beta);
        }
    }

    let resp = rb.send().await?;
    if let Some(limits) = usage::from_anthropic_headers(resp.headers()) {
        usage::save_claude(&limits);
    }
    let sse = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("event-stream"));
    let success = resp.status().is_success();
    let mut out = Response::builder().status(resp.status().as_u16());
    for (name, value) in resp.headers() {
        // Claude Code shows its own limit warnings from the rate-limit headers.
        let name = name.as_str();
        if matches!(name, "content-type" | "request-id" | "retry-after") || name.starts_with("anthropic-ratelimit-") {
            out = out.header(name, value.as_bytes());
        }
    }
    let body = if success {
        let mut scanner =
            UsageScanner { model: model.into(), sse, buf: Vec::new(), usage: None, on_usage: Some(Box::new(on_usage)) };
        Body::from_stream(resp.bytes_stream().inspect(move |chunk| {
            if let Ok(chunk) = chunk {
                scanner.feed(chunk);
            }
        }))
    } else {
        Body::from_stream(resp.bytes_stream())
    };
    out.body(body).map_err(|e| ProxyError::internal(e.to_string()))
}

/// Reads token usage out of the piped reply without changing it, and logs it once the reply
/// is done (or the client went away).
struct UsageScanner {
    model: String,
    sse: bool,
    buf: Vec<u8>,
    usage: Option<Usage>,
    on_usage: Option<Box<dyn FnOnce(Usage) + Send>>,
}

impl UsageScanner {
    fn feed(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
        if !self.sse {
            return;
        }
        while let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=end).collect();
            if let Some(data) = line.strip_prefix(b"data:")
                && let Ok(event) = serde_json::from_slice::<Value>(data)
            {
                match event["type"].as_str() {
                    Some("message_start") => self.take(&event["message"]["usage"]),
                    Some("message_delta") => self.take(&event["usage"]),
                    _ => {}
                }
            }
        }
    }

    /// Later events repeat or update counts, so each field present overwrites the earlier one.
    fn take(&mut self, u: &Value) {
        if !u.is_object() {
            return;
        }
        let usage = self.usage.get_or_insert_with(Usage::default);
        let field = |name: &str| u[name].as_u64();
        if let Some(n) = field("input_tokens") {
            usage.input_tokens = n;
        }
        if let Some(n) = field("output_tokens") {
            usage.output_tokens = n;
        }
        if let Some(n) = field("cache_creation_input_tokens") {
            usage.cache_creation_input_tokens = n;
        }
        if let Some(n) = field("cache_read_input_tokens") {
            usage.cache_read_input_tokens = n;
        }
    }
}

impl Drop for UsageScanner {
    fn drop(&mut self) {
        if !self.sse
            && let Ok(reply) = serde_json::from_slice::<Value>(&self.buf)
        {
            self.take(&reply["usage"]);
        }
        if let Some(usage) = self.usage {
            usage.log("anthropic", &self.model);
            if let Some(on_usage) = self.on_usage.take() {
                on_usage(usage);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    #[test]
    fn scanner_reads_stream_usage() {
        let mut s = UsageScanner { model: "m".into(), sse: true, buf: Vec::new(), usage: None, on_usage: None };
        s.feed(b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5,\"cache_read_input_tokens\":90,\"cache_creation_input_tokens\":3,\"output_tokens\":1}}}\n\nevent: message_delta\ndata: {\"type\":\"message_de");
        s.feed(b"lta\",\"usage\":{\"output_tokens\":42}}\n\n");
        let u = s.usage.unwrap();
        assert_eq!((u.input_tokens, u.cache_read_input_tokens, u.cache_creation_input_tokens, u.output_tokens), (5, 90, 3, 42));
    }

    #[test]
    fn placeholder_is_not_a_login() {
        assert!(!has_client_credentials(&headers(&[])));
        assert!(!has_client_credentials(&headers(&[("authorization", "Bearer claude-bridge")])));
        assert!(!has_client_credentials(&headers(&[("x-api-key", "claude-bridge")])));
        assert!(has_client_credentials(&headers(&[("authorization", "Bearer sk-ant-oat01-x")])));
        assert!(has_client_credentials(&headers(&[("x-api-key", "sk-ant-api03-x")])));
    }
}
