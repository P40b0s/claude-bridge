//! Messages between Dart and Rust. Dart bindings are generated with `rinf gen`.

use std::collections::BTreeMap;

use claude_bridge_core::request_log::{RequestRecord, RequestStatus};
use claude_bridge_core::routing::Target;
use rinf::{DartSignal, RustSignal, SignalPiece};
use serde::{Deserialize, Serialize};

// ── Dart → Rust ──────────────────────────────────────────────────────

#[derive(Deserialize, Serialize, Clone, Copy, SignalPiece)]
pub enum ProxyAction {
    Start,
    Stop,
    Restart,
}

#[derive(Deserialize, DartSignal)]
pub struct ProxyCommand {
    pub action: ProxyAction,
}

/// Pins every session to `model` (a `/model` spec); None follows Claude Code's /model again.
#[derive(Deserialize, DartSignal)]
pub struct PinModel {
    pub model: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, SignalPiece)]
pub enum AuthProvider {
    ChatGpt,
    Google,
}

#[derive(Deserialize, DartSignal)]
pub struct LoginRequest {
    pub provider: AuthProvider,
    /// Google account slot, 1 or 2.
    pub account: i32,
}

#[derive(Deserialize, DartSignal)]
pub struct CancelLogin {}

#[derive(Deserialize, DartSignal)]
pub struct LogoutRequest {
    pub provider: AuthProvider,
    pub account: i32,
}

/// Adds the proxy to Claude Code's settings.json, or takes it out.
#[derive(Deserialize, DartSignal)]
pub struct ConnectClaudeCode {
    pub connect: bool,
}

/// Re-read subscription limits now instead of on the next timer tick.
#[derive(Deserialize, DartSignal)]
pub struct RefreshUsage {}

/// Values for `~/.claude-bridge/.env`; an empty value removes the key.
#[derive(Deserialize, DartSignal)]
pub struct SaveSettings {
    pub values: BTreeMap<String, String>,
}

/// Turns the country check on or off and sets the countries it allows (ISO codes).
#[derive(Deserialize, DartSignal)]
pub struct SetGeoPolicy {
    pub enabled: bool,
    pub allowed: Vec<String>,
}

/// Look up the current IP and country now instead of on the next timer tick.
#[derive(Deserialize, DartSignal)]
pub struct RefreshLocation {}

// ── Rust → Dart ──────────────────────────────────────────────────────

#[derive(Serialize, Clone, PartialEq, SignalPiece)]
pub struct ModelInfo {
    pub provider: String,
    pub model: String,
    pub reasoning: Option<String>,
}

impl From<&Target> for ModelInfo {
    fn from(t: &Target) -> Self {
        Self {
            provider: t.provider.as_str().into(),
            model: t.model.clone(),
            reasoning: t.reasoning.map(|r| r.as_str().into()),
        }
    }
}

#[derive(Serialize, Clone, PartialEq, RustSignal)]
pub struct ProxyState {
    pub running: bool,
    pub port: i32,
    pub pid: Option<i64>,
    /// Unix milliseconds.
    pub started_at: Option<i64>,
    pub active: Option<ModelInfo>,
    pub pinned: Option<ModelInfo>,
    /// The CLI that runs the proxy; None when it cannot be found.
    pub cli_path: Option<String>,
    /// A start/stop/restart is in progress.
    pub busy: bool,
    /// Every `claude` goes through this proxy (Claude Code's settings.json).
    pub claude_code_connected: bool,
    /// Claude Code's settings point at some other address.
    pub claude_code_other_url: Option<String>,
    pub claude_code_settings: String,
    /// The UI starts the proxy when it opens (CLAUDE_BRIDGE_AUTOSTART is not `0`).
    pub autostart: bool,
}

#[derive(Serialize, Clone, SignalPiece)]
pub struct ChatGptAccount {
    pub email: Option<String>,
    pub plan: Option<String>,
    /// "claude-bridge" or "codex-cli".
    pub source: String,
    pub expires_at: i64,
    /// When the paid plan ends, Unix ms; None on the free plan.
    pub subscription_until: Option<i64>,
}

#[derive(Serialize, Clone, SignalPiece)]
pub struct GoogleAccount {
    pub account: i32,
    pub email: Option<String>,
    pub project_id: Option<String>,
    pub expires_at: i64,
}

/// Claude Code's own login; only its account record is read, never its credentials.
#[derive(Serialize, Clone, SignalPiece)]
pub struct ClaudeCodeAccount {
    pub email: Option<String>,
    pub organization: Option<String>,
    /// `Pro`, `Max`, `Team`…
    pub plan: Option<String>,
    pub billing: Option<String>,
    /// Unix ms; when the subscription ends is not known.
    pub subscription_since: Option<i64>,
}

#[derive(Serialize, RustSignal)]
pub struct AccountsState {
    pub claude_code: Option<ClaudeCodeAccount>,
    pub chatgpt: Option<ChatGptAccount>,
    pub google: Vec<GoogleAccount>,
}

#[derive(Serialize, Clone, SignalPiece)]
pub struct UsageWindow {
    /// Provider's name: `5h`, `7d_opus`, `primary`, a Gemini model id.
    pub id: String,
    pub model: Option<String>,
    pub window_minutes: Option<i64>,
    pub used_percent: f64,
    /// Unix ms.
    pub resets_at: Option<i64>,
}

#[derive(Serialize, Clone, SignalPiece)]
pub struct Usage {
    /// Google account slot.
    pub account: Option<i32>,
    pub windows: Vec<UsageWindow>,
    pub limited: bool,
    /// Unix ms when the numbers were read.
    pub observed_at: i64,
    pub error: Option<String>,
}

/// Subscription limits. Claude's come from Anthropic's replies relayed by the proxy.
#[derive(Serialize, RustSignal)]
pub struct UsageState {
    pub claude: Option<Usage>,
    pub codex: Option<Usage>,
    pub gemini: Vec<Usage>,
}

#[derive(Serialize, Clone, Copy, SignalPiece)]
pub enum LoginPhase {
    /// Browser opened; `url` is there in case it did not.
    Waiting,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Serialize, RustSignal)]
pub struct LoginState {
    pub provider: AuthProvider,
    pub account: i32,
    pub phase: LoginPhase,
    pub url: Option<String>,
    pub message: Option<String>,
}

#[derive(Serialize, Clone, Copy, SignalPiece)]
pub enum RequestPhase {
    Running,
    Ok,
    Error,
    Cancelled,
}

#[derive(Serialize, Clone, SignalPiece)]
pub struct RequestEntry {
    pub id: i64,
    pub started_at: i64,
    pub requested: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub stream: bool,
    pub phase: RequestPhase,
    pub duration_ms: Option<i64>,
    pub http_status: Option<i32>,
    pub stop_reason: Option<String>,
    pub input_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub error: Option<String>,
}

impl From<RequestRecord> for RequestEntry {
    fn from(r: RequestRecord) -> Self {
        Self {
            id: r.id as i64,
            started_at: r.started_at,
            requested: r.requested,
            provider: r.provider,
            model: r.model,
            reasoning: r.reasoning,
            stream: r.stream,
            phase: match r.status {
                RequestStatus::Running => RequestPhase::Running,
                RequestStatus::Ok => RequestPhase::Ok,
                RequestStatus::Error => RequestPhase::Error,
                RequestStatus::Cancelled => RequestPhase::Cancelled,
            },
            duration_ms: r.duration_ms.map(|d| d as i64),
            http_status: r.http_status.map(i32::from),
            stop_reason: r.stop_reason,
            input_tokens: r.usage.map(|u| u.input_tokens as i64),
            cache_read_tokens: r.usage.map(|u| u.cache_read_input_tokens as i64),
            output_tokens: r.usage.map(|u| u.output_tokens as i64),
            error: r.error,
        }
    }
}

/// A new or updated request; Dart replaces any entry with the same id.
#[derive(Serialize, RustSignal)]
pub struct RequestLogged {
    pub entry: RequestEntry,
}

/// Connected to a (new) proxy process: drop the old log, its records are replayed next.
#[derive(Serialize, RustSignal)]
pub struct RequestLogReset {}

#[derive(Serialize, Clone, PartialEq, SignalPiece)]
pub struct CountryInfo {
    pub code: String,
    pub name: String,
}

/// Where the traffic leaves from, and the country allow-list that gates the proxy.
#[derive(Serialize, Clone, PartialEq, RustSignal)]
pub struct GeoState {
    pub ip: Option<String>,
    pub country_code: Option<String>,
    pub country: Option<String>,
    /// The last lookup failed (shown while an older location may still be displayed).
    pub error: Option<String>,
    /// A lookup is in progress.
    pub checking: bool,
    pub checked_at: Option<i64>,
    /// The check is on: requests are refused outside `allowed`.
    pub enabled: bool,
    pub allowed: Vec<String>,
    pub countries: Vec<CountryInfo>,
}

#[derive(Serialize, Clone, SignalPiece)]
pub struct PresetInfo {
    pub alias: String,
    pub provider: String,
    pub model: String,
    pub reasoning: Option<String>,
    pub description: String,
}

#[derive(Serialize, RustSignal)]
pub struct SettingsState {
    /// Values from the .env file.
    pub values: BTreeMap<String, String>,
    /// Keys set only in the process environment (shown as set but not editable here).
    pub from_environment: Vec<String>,
    pub file_path: String,
    pub presets: Vec<PresetInfo>,
    pub efforts: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, SignalPiece)]
pub enum ErrorSeverity {
    Info,
    Warning,
    Error,
    Success,
}

#[derive(Serialize, RustSignal)]
pub struct ErrorSignal {
    pub error: String,
    pub severity: ErrorSeverity,
}

impl ErrorSignal {
    pub fn send(severity: ErrorSeverity, text: impl Into<String>) {
        ErrorSignal { error: text.into(), severity }.send_signal_to_dart();
    }

    pub fn error(err: &anyhow::Error) {
        tracing::error!("{err:#}");
        Self::send(ErrorSeverity::Error, format!("{err:#}"));
    }
}
