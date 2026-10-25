//! Recent requests handled by the proxy, kept in memory and broadcast to `/_events` listeners.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::anthropic::{Outcome, Usage};
use crate::error::ProxyError;
use crate::routing::Target;

const KEEP: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequestStatus {
    Running,
    Ok,
    Error,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestRecord {
    pub id: u64,
    /// Unix milliseconds.
    pub started_at: i64,
    /// The `model` field exactly as Claude Code sent it.
    pub requested: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub stream: bool,
    pub status: RequestStatus,
    pub duration_ms: Option<u64>,
    pub http_status: Option<u16>,
    pub stop_reason: Option<String>,
    pub usage: Option<Usage>,
    pub error: Option<String>,
}

/// A request in flight. Finishing consumes it, so every request is reported exactly once.
pub struct Tracker {
    record: RequestRecord,
    started: Instant,
}

impl Tracker {
    pub fn id(&self) -> u64 {
        self.record.id
    }

    pub fn set_target(&mut self, target: &Target) {
        self.record.provider = Some(target.provider.as_str().into());
        self.record.model = Some(target.model.clone());
        self.record.reasoning = target.reasoning.map(|r| r.as_str().into());
    }

    fn done(mut self, status: RequestStatus) -> RequestRecord {
        self.record.status = status;
        self.record.duration_ms = Some(self.started.elapsed().as_millis() as u64);
        self.record
    }

    pub fn ok(self, outcome: &Outcome) -> RequestRecord {
        let mut r = self.done(RequestStatus::Ok);
        r.http_status = Some(200);
        r.stop_reason = Some(outcome.stop_reason.clone());
        r.usage = Some(outcome.usage);
        r
    }

    /// Passthrough replies are piped untouched, so only the status is known.
    pub fn passed_through(self, status: u16) -> RequestRecord {
        let ok = (200..300).contains(&status);
        let mut r = self.done(if ok { RequestStatus::Ok } else { RequestStatus::Error });
        r.http_status = Some(status);
        r
    }

    pub fn failed(self, err: &ProxyError) -> RequestRecord {
        if err.is_client_gone() {
            return self.done(RequestStatus::Cancelled);
        }
        let mut r = self.done(RequestStatus::Error);
        r.http_status = Some(err.status.as_u16());
        r.error = Some(err.message.clone());
        r
    }
}

pub struct RequestLog {
    next_id: AtomicU64,
    recent: Mutex<VecDeque<RequestRecord>>,
    tx: broadcast::Sender<RequestRecord>,
}

impl Default for RequestLog {
    fn default() -> Self {
        Self { next_id: AtomicU64::new(1), recent: Mutex::new(VecDeque::new()), tx: broadcast::channel(256).0 }
    }
}

impl RequestLog {
    pub fn begin(&self, requested: &str, stream: bool) -> Tracker {
        let record = RequestRecord {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            started_at: crate::auth::now_ms(),
            requested: requested.into(),
            provider: None,
            model: None,
            reasoning: None,
            stream,
            status: RequestStatus::Running,
            duration_ms: None,
            http_status: None,
            stop_reason: None,
            usage: None,
            error: None,
        };
        Tracker { record, started: Instant::now() }
    }

    /// Publishes a running request; the same id is published again when it finishes.
    pub fn running(&self, tracker: &Tracker) {
        self.publish(tracker.record.clone());
    }

    pub fn publish(&self, record: RequestRecord) {
        {
            let mut recent = self.recent.lock().unwrap();
            match recent.iter_mut().find(|r| r.id == record.id) {
                Some(existing) => *existing = record.clone(),
                None => {
                    recent.push_back(record.clone());
                    if recent.len() > KEEP {
                        recent.pop_front();
                    }
                }
            }
        }
        let _ = self.tx.send(record);
    }

    /// Usage that arrives after the record was published (Anthropic passthrough reads it from
    /// the piped reply once it ends).
    pub fn set_usage(&self, id: u64, usage: Usage) {
        let record = {
            let mut recent = self.recent.lock().unwrap();
            let Some(record) = recent.iter_mut().find(|r| r.id == id) else { return };
            record.usage = Some(usage);
            record.clone()
        };
        let _ = self.tx.send(record);
    }

    /// Recent records (oldest first) plus a receiver for everything after them.
    pub fn subscribe(&self) -> (Vec<RequestRecord>, broadcast::Receiver<RequestRecord>) {
        let recent = self.recent.lock().unwrap();
        (recent.iter().cloned().collect(), self.tx.subscribe())
    }
}
