//! Subscription limits: how much of each window is used and when it resets.
//!
//! - Codex: the ChatGPT backend's usage endpoint, the one Codex CLI's `/status` reads.
//! - Gemini: Code Assist's `retrieveUserQuota`, as in Gemini CLI. The standard Gemini API has none.
//! - Claude: the `anthropic-ratelimit-unified-*` headers Anthropic puts on its replies to
//!   Claude Code. The proxy only reads them as it relays the reply and keeps the last ones in
//!   a file, so the UI and CLI can show them; it never uses Claude Code's credentials itself.

use std::path::PathBuf;

use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::auth::google::{CODE_ASSIST_ENDPOINT, GoogleAuth};
use crate::auth::{self, now_ms};
use crate::config;

const CODEX_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const CLAUDE_HEADER_PREFIX: &str = "anthropic-ratelimit-unified-";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Window {
    /// Provider's name for the window: `5h`, `7d_opus`, `primary`, a Gemini model id.
    pub id: String,
    /// Model family the window is limited to, if any.
    pub model: Option<String>,
    pub window_minutes: Option<i64>,
    pub used_percent: f64,
    /// Unix ms.
    pub resets_at: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Usage {
    /// Google account slot; None for single-account providers.
    pub account: Option<u8>,
    pub windows: Vec<Window>,
    /// The provider currently refuses requests because a limit is reached.
    pub limited: bool,
    /// Unix ms when the numbers were read.
    pub observed_at: i64,
    /// Why there are no numbers.
    pub error: Option<String>,
}

impl Usage {
    fn unavailable(account: Option<u8>, error: impl Into<String>) -> Self {
        Self { account, observed_at: now_ms(), error: Some(error.into()), ..Self::default() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Report {
    pub claude: Option<Usage>,
    pub codex: Option<Usage>,
    pub gemini: Vec<Usage>,
}

/// Reads every provider the user is logged in to. Never refreshes a ChatGPT token:
/// its refresh token rotates, and the proxy may be refreshing it at the same moment.
pub async fn collect(http: &reqwest::Client, google: &GoogleAuth) -> Report {
    let gemini = futures_util::future::join_all(
        (1..=2u8).filter(|&account| auth::google::load(account).is_some()).map(|account| gemini(http, google, account)),
    );
    let (codex, gemini) = tokio::join!(codex(http), gemini);
    Report { claude: load_claude(), codex, gemini }
}

pub async fn codex(http: &reqwest::Client) -> Option<Usage> {
    let (tokens, _) = auth::openai::load()?;
    if now_ms() > tokens.expires_at {
        return Some(Usage::unavailable(None, "token expired; it is refreshed on the next request"));
    }
    let mut rb = http.get(CODEX_USAGE_URL).bearer_auth(&tokens.access_token).header("originator", "codex_cli_rs");
    if let Some(account) = &tokens.account_id {
        rb = rb.header("chatgpt-account-id", account);
    }
    let body = match fetch_json(rb).await {
        Ok(body) => body,
        Err(e) => return Some(Usage::unavailable(None, e)),
    };
    Some(parse_codex(&body))
}

fn parse_codex(body: &Value) -> Usage {
    let limits = &body["rate_limit"];
    let windows = ["primary_window", "secondary_window"]
        .into_iter()
        .filter_map(|name| {
            let w = limits[name].as_object()?;
            Some(Window {
                id: name.trim_end_matches("_window").into(),
                model: None,
                window_minutes: w.get("limit_window_seconds").and_then(Value::as_i64).map(|s| s / 60),
                used_percent: w.get("used_percent").and_then(Value::as_f64)?,
                resets_at: w.get("reset_at").and_then(Value::as_i64).map(|s| s * 1000),
            })
        })
        .collect();
    Usage {
        windows,
        limited: limits["limit_reached"].as_bool() == Some(true) || limits["allowed"].as_bool() == Some(false),
        observed_at: now_ms(),
        ..Usage::default()
    }
}

pub async fn gemini(http: &reqwest::Client, auth: &GoogleAuth, account: u8) -> Usage {
    let tokens = match auth.credentials(http, account).await {
        Ok(tokens) => tokens,
        Err(e) => return Usage::unavailable(Some(account), e.to_string()),
    };
    let Some(project) = tokens.project_id else {
        return Usage::unavailable(Some(account), "standard Gemini API: Google reports no quota without Code Assist");
    };
    let rb = http
        .post(format!("{CODE_ASSIST_ENDPOINT}:retrieveUserQuota"))
        .bearer_auth(&tokens.access_token)
        .json(&json!({ "project": project }));
    match fetch_json(rb).await {
        Ok(body) => Usage { account: Some(account), ..parse_gemini(&body) },
        Err(e) => Usage::unavailable(Some(account), e),
    }
}

fn parse_gemini(body: &Value) -> Usage {
    let windows: Vec<Window> = body["buckets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|b| {
            let remaining = b["remainingFraction"].as_f64()?;
            let model = b["modelId"].as_str().unwrap_or("Gemini");
            let id = match b["tokenType"].as_str() {
                Some(kind) if kind != "REQUESTS" => format!("{model} {}", kind.to_ascii_lowercase()),
                _ => model.to_string(),
            };
            Some(Window {
                id,
                model: Some(model.into()),
                window_minutes: None,
                used_percent: ((1.0 - remaining) * 100.0).clamp(0.0, 100.0),
                resets_at: b["resetTime"].as_str().and_then(parse_rfc3339_ms),
            })
        })
        .collect();
    Usage { limited: windows.iter().any(|w| w.used_percent >= 100.0), windows, observed_at: now_ms(), ..Usage::default() }
}

async fn fetch_json(rb: reqwest::RequestBuilder) -> Result<Value, String> {
    let resp = rb.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("{status}: {}", crate::error::truncate(&body, 200)));
    }
    resp.json().await.map_err(|e| e.to_string())
}

/// Claude's windows from a reply's headers, e.g. `…-5h-utilization: 0.23`, `…-5h-reset: 1761400000`.
pub fn from_anthropic_headers(headers: &HeaderMap) -> Option<Usage> {
    let mut windows: Vec<Window> = Vec::new();
    let mut limited = false;
    for (name, value) in headers {
        let Some(rest) = name.as_str().strip_prefix(CLAUDE_HEADER_PREFIX) else { continue };
        let Ok(value) = value.to_str() else { continue };
        if rest == "status" {
            limited = value == "rejected";
            continue;
        }
        let (id, field) = match rest.rsplit_once('-') {
            Some((id, field @ ("utilization" | "reset"))) => (id, field),
            _ => continue,
        };
        let index = match windows.iter().position(|w| w.id == id) {
            Some(index) => index,
            None => {
                windows.push(claude_window(id));
                windows.len() - 1
            }
        };
        match field {
            "utilization" => windows[index].used_percent = (value.parse::<f64>().unwrap_or(0.0) * 1000.0).round() / 10.0,
            _ => windows[index].resets_at = value.parse::<i64>().ok().map(|s| s * 1000),
        }
    }
    windows.sort_by_key(|w| (w.window_minutes, w.model.clone()));
    (!windows.is_empty()).then(|| Usage { windows, limited, observed_at: now_ms(), ..Usage::default() })
}

/// `5h` → 300 minutes; `7d_opus` → a week, Opus only.
fn claude_window(id: &str) -> Window {
    let (span, model) = id.split_once('_').map_or((id, None), |(s, m)| (s, Some(m.to_string())));
    let minutes = span.strip_suffix('h').and_then(|h| h.parse::<i64>().ok()).map(|h| h * 60).or_else(|| {
        span.strip_suffix('d').and_then(|d| d.parse::<i64>().ok()).map(|d| d * 24 * 60)
    });
    Window { id: id.into(), model, window_minutes: minutes, used_percent: 0.0, resets_at: None }
}

fn claude_file() -> PathBuf {
    config::data_dir().join("claude-usage.json")
}

pub fn load_claude() -> Option<Usage> {
    serde_json::from_slice(&std::fs::read(claude_file()).ok()?).ok()
}

/// Keeps the latest Claude numbers; skips the write when nothing but the time changed.
pub fn save_claude(usage: &Usage) {
    let same = load_claude().is_some_and(|old| old.windows == usage.windows && old.limited == usage.limited);
    if same {
        return;
    }
    if let Ok(json) = serde_json::to_vec_pretty(usage) {
        let _ = std::fs::create_dir_all(config::data_dir());
        let _ = std::fs::write(claude_file(), json);
    }
}

/// `2026-10-25T09:00:00Z` or with a fractional part / offset → Unix ms.
pub(crate) fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    let (date, time) = s.split_once(['T', 't', ' '])?;
    let mut d = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let (clock, offset_min) = match time.find(['Z', 'z', '+', '-']) {
        Some(i) => {
            let tz = &time[i..];
            let offset = if tz.eq_ignore_ascii_case("z") {
                0
            } else {
                let sign = if tz.starts_with('-') { -1 } else { 1 };
                let (h, mm) = tz[1..].split_once(':')?;
                sign * (h.parse::<i64>().ok()? * 60 + mm.parse::<i64>().ok()?)
            };
            (&time[..i], offset)
        }
        None => (time, 0),
    };
    let mut c = clock.splitn(3, ':');
    let (h, min) = (c.next()?.parse::<i64>().ok()?, c.next()?.parse::<i64>().ok()?);
    let secs: f64 = c.next().unwrap_or("0").parse().ok()?;
    // Days from civil (Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let total = days * 86_400 + h * 3600 + min * 60 - offset_min * 60;
    Some(total * 1000 + (secs * 1000.0).round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339_ms("2026-10-25T09:00:00Z"), Some(1_792_918_800_000));
        assert_eq!(parse_rfc3339_ms("2026-10-25T12:00:00.5+03:00"), Some(1_792_918_800_500));
    }

    #[test]
    fn claude_headers() {
        let mut h = HeaderMap::new();
        h.insert("anthropic-ratelimit-unified-7d_opus-utilization", "0.5".parse().unwrap());
        h.insert("anthropic-ratelimit-unified-5h-utilization", "0.23".parse().unwrap());
        h.insert("anthropic-ratelimit-unified-5h-reset", "1793005200".parse().unwrap());
        h.insert("anthropic-ratelimit-unified-status", "allowed".parse().unwrap());
        h.insert("anthropic-ratelimit-unified-representative-claim", "five_hour".parse().unwrap());
        let u = from_anthropic_headers(&h).unwrap();
        assert!(!u.limited);
        assert_eq!(u.windows.len(), 2);
        assert_eq!((u.windows[0].id.as_str(), u.windows[0].window_minutes), ("5h", Some(300)));
        assert!((u.windows[0].used_percent - 23.0).abs() < 1e-9);
        assert_eq!(u.windows[0].resets_at, Some(1_793_005_200_000));
        assert_eq!((u.windows[1].model.as_deref(), u.windows[1].window_minutes), (Some("opus"), Some(7 * 24 * 60)));
        assert!(from_anthropic_headers(&HeaderMap::new()).is_none());
    }

    #[test]
    fn codex_body() {
        let body = json!({ "rate_limit": { "allowed": true, "limit_reached": false,
            "primary_window": { "used_percent": 12, "limit_window_seconds": 18000, "reset_at": 1795511465 },
            "secondary_window": null } });
        let u = parse_codex(&body);
        assert_eq!(u.windows.len(), 1);
        assert_eq!(u.windows[0].window_minutes, Some(300));
        assert_eq!(u.windows[0].resets_at, Some(1_795_511_465_000));
    }

    #[test]
    fn gemini_body() {
        let body = json!({ "buckets": [{ "modelId": "gemini-2.5-pro", "tokenType": "REQUESTS",
            "remainingFraction": 0.75, "resetTime": "2026-10-25T09:00:00Z" }] });
        let u = parse_gemini(&body);
        assert_eq!(u.windows[0].id, "gemini-2.5-pro");
        assert!((u.windows[0].used_percent - 25.0).abs() < 1e-9);
    }
}
