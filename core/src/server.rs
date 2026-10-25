//! The local Anthropic-compatible HTTP server.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anyhow::Context;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use futures_util::future::BoxFuture;
use futures_util::{FutureExt, StreamExt, stream};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::{Notify, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{info, warn};

use crate::anthropic::{Aggregator, Event, MessageWriter, MessagesRequest, Outcome};
use crate::auth::google::GoogleAuth;
use crate::auth::openai::OpenAiAuth;
use crate::config;
use crate::error::ProxyError;
use crate::providers::anthropic;
use crate::providers::codex::{self, CodexAuth};
use crate::providers::gemini::{self, GeminiAuth};
use crate::providers::openai_compat;
use crate::providers::reasoning_cache::ReasoningCache;
use crate::request_log::{RequestLog, Tracker};
use crate::routing::{self, Provider, Target};
use crate::{families, geo};

/// Header the CLI and UI send to the control endpoints (/_shutdown, /_model). A custom header cannot be sent cross-origin
/// without a CORS preflight, so web pages cannot stop the proxy.
pub const SHUTDOWN_HEADER: &str = "x-claude-bridge";
const SESSION_HEADER: &str = "x-claude-code-session-id";
/// Sessions remembered at once; the map starts over when it fills up.
const MAX_SESSIONS: usize = 512;

pub struct AppState {
    pub http: reqwest::Client,
    pub openai_auth: OpenAiAuth,
    pub google_auth: GoogleAuth,
    pub reasoning_cache: ReasoningCache,
    pub log: RequestLog,
    pub geo: geo::Guard,
    /// Last model the user picked in any session; the fallback for sessions not seen yet.
    active: RwLock<Option<Target>>,
    /// Last model picked per Claude Code session (`x-claude-code-session-id`). Bare model
    /// names and the session's internal claude-* calls follow it.
    sessions: Mutex<HashMap<String, Target>>,
    /// Model chosen from the CLI or UI; while set it overrides what Claude Code asks for.
    pinned: RwLock<Option<Target>>,
    started_at: i64,
    port: u16,
    shutdown: Notify,
}

pub fn http_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        // Upstreams drop idle keep-alive sockets; do not hand out stale ones.
        .pool_idle_timeout(Duration::from_secs(30))
        .build()?)
}

pub async fn serve(port: u16, default: Option<Target>) -> anyhow::Result<()> {
    let state = Arc::new(AppState {
        http: http_client()?,
        openai_auth: OpenAiAuth::default(),
        google_auth: GoogleAuth::default(),
        reasoning_cache: ReasoningCache::default(),
        log: RequestLog::default(),
        geo: geo::Guard::default(),
        active: RwLock::new(default),
        sessions: Mutex::default(),
        pinned: RwLock::new(None),
        started_at: crate::auth::now_ms(),
        port,
        shutdown: Notify::new(),
    });

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/messages", post(messages))
        .route("/_shutdown", post(shutdown))
        .route("/_events", get(events))
        .route("/_model", axum::routing::put(pin_model).delete(unpin_model))
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), check_host))
        .with_state(state.clone());

    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .with_context(|| format!("cannot listen on 127.0.0.1:{port}"))?;
    info!(
        pid = std::process::id(),
        "listening on http://127.0.0.1:{port}"
    );
    if let Some(active) = state.active.read().unwrap().as_ref() {
        info!(
            "default model: {}:{}",
            active.provider.as_str(),
            active.model
        );
    }

    tokio::select! {
        result = axum::serve(listener, app) => result?,
        _ = state.shutdown.notified() => info!("shutdown requested"),
        _ = tokio::signal::ctrl_c() => info!("interrupted"),
        _ = terminate() => info!("terminated"),
    }
    Ok(())
}

#[cfg(unix)]
async fn terminate() {
    if let Ok(mut sig) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        sig.recv().await;
    }
}

#[cfg(not(unix))]
async fn terminate() {
    std::future::pending::<()>().await
}

/// Rejects requests whose Host is not this loopback address, which defeats DNS rebinding:
/// a web page must not be able to spend the user's subscriptions through the proxy.
async fn check_host(State(state): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    let allowed = [
        format!("127.0.0.1:{}", state.port),
        format!("localhost:{}", state.port),
    ];
    if !allowed.iter().any(|a| a == host) {
        return ProxyError::new(
            StatusCode::FORBIDDEN,
            format!("unexpected Host header: {host}"),
        )
        .into_response();
    }
    next.run(req).await
}

async fn healthz(State(state): State<Arc<AppState>>) -> Json<Value> {
    let active = state.active.read().unwrap().clone();
    let pinned = state.pinned.read().unwrap().clone();
    Json(json!({
        "ok": true,
        "pid": std::process::id(),
        "started_at": state.started_at,
        "active": active,
        "pinned": pinned,
    }))
}

/// Control endpoints need a custom header, which a web page cannot send without a CORS preflight.
fn control_allowed(headers: &HeaderMap) -> Result<(), ProxyError> {
    if headers.contains_key(SHUTDOWN_HEADER) {
        Ok(())
    } else {
        Err(ProxyError::new(
            StatusCode::FORBIDDEN,
            format!("missing {SHUTDOWN_HEADER} header"),
        ))
    }
}

async fn shutdown(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<StatusCode, ProxyError> {
    control_allowed(&headers)?;
    state.shutdown.notify_one();
    Ok(StatusCode::OK)
}

#[derive(serde::Deserialize)]
struct PinModel {
    model: String,
}

async fn pin_model(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<PinModel>,
) -> Result<Json<Target>, ProxyError> {
    control_allowed(&headers)?;
    let defaults = state.active.read().unwrap().clone();
    let target =
        routing::resolve(&body.model, defaults.as_ref()).map_err(ProxyError::bad_request)?;
    info!(
        "model pinned to {}:{}",
        target.provider.as_str(),
        target.model
    );
    *state.pinned.write().unwrap() = Some(target.clone());
    *state.active.write().unwrap() = Some(target.clone());
    Ok(Json(target))
}

async fn unpin_model(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<StatusCode, ProxyError> {
    control_allowed(&headers)?;
    info!("model unpinned; following Claude Code's /model again");
    *state.pinned.write().unwrap() = None;
    Ok(StatusCode::NO_CONTENT)
}

/// Request log as SSE: recent records first, then every update. The CLI-independent UI reads this.
async fn events(State(state): State<Arc<AppState>>) -> Response {
    let (recent, rx) = state.log.subscribe();
    let live =
        tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|r| async move { r.ok() });
    let records = stream::iter(recent).chain(live).map(|record| {
        Ok::<_, Infallible>(
            SseEvent::default()
                .event("request")
                .data(serde_json::to_string(&record).unwrap_or_default()),
        )
    });
    Sse::new(records)
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn messages(
    State(state): State<Arc<AppState>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let raw: Value = match serde_json::from_slice(&body) {
        Ok(raw) => raw,
        Err(e) => return ProxyError::from(e).into_response(),
    };
    let requested = raw["model"].as_str().unwrap_or_default().to_string();
    // Anthropic's convention: only `stream: true` streams. Claude Code's /model probe omits it.
    let stream = raw["stream"] == true;
    let tracker = state.log.begin(&requested, stream);
    if let Err(e) = state.geo.check(&state.http).await {
        warn!("{e}");
        state.log.publish(tracker.failed(&e));
        return e.into_response();
    }
    match route_message(
        state.clone(),
        headers,
        uri.query(),
        raw,
        requested,
        stream,
        tracker,
    )
    .await
    {
        Ok(resp) => resp,
        Err(failed) => {
            let (e, tracker) = *failed;
            warn!("{e}");
            state.log.publish(tracker.failed(&e));
            e.into_response()
        }
    }
}

async fn route_message(
    state: Arc<AppState>,
    headers: HeaderMap,
    query: Option<&str>,
    raw: Value,
    requested: String,
    stream: bool,
    mut tracker: Tracker,
) -> Result<Response, Box<(ProxyError, Tracker)>> {
    let pinned = state.pinned.read().unwrap().clone();
    let is_internal = requested.to_ascii_lowercase().starts_with("claude-");
    let session = headers
        .get(SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let active = || {
        let own = session
            .as_ref()
            .and_then(|s| state.sessions.lock().unwrap().get(s).cloned());
        own.or_else(|| state.active.read().unwrap().clone())
    };
    let target = match pinned {
        Some(pinned) if !is_internal => pinned,
        pinned => match routing::resolve(&requested, pinned.or_else(active).as_ref()) {
            Ok(target) => target,
            Err(e) => return Err(Box::new((ProxyError::bad_request(e), tracker))),
        },
    };
    tracker.set_target(&target);
    if families::provider_disabled(target.provider) {
        let e = ProxyError::bad_request(format!(
            "provider {} is disabled in claude-bridge: turn it on in the Providers tab or pick another model with /model",
            target.provider.as_str()
        ));
        return Err(Box::new((e, tracker)));
    }
    state.log.running(&tracker);

    info!(
        requested = %requested,
        provider = target.provider.as_str(),
        model = %target.model,
        reasoning = target.reasoning.map(|r| r.as_str()).unwrap_or("-"),
        stream,
        "request"
    );

    // Internal claude-* calls (titles, subagents) must not change the user's model.
    if !is_internal {
        *state.active.write().unwrap() = Some(target.clone());
        if let Some(session) = session {
            let mut sessions = state.sessions.lock().unwrap();
            if sessions.len() >= MAX_SESSIONS && !sessions.contains_key(&session) {
                sessions.clear();
            }
            sessions.insert(session, target.clone());
        }
    }

    if target.provider == Provider::Anthropic {
        let on_usage = {
            let state = state.clone();
            let id = tracker.id();
            move |usage| state.log.set_usage(id, usage)
        };
        return match anthropic::passthrough(&state, raw, &target.model, &headers, query, on_usage).await {
            Ok(resp) => {
                state
                    .log
                    .publish(tracker.passed_through(resp.status().as_u16()));
                Ok(resp)
            }
            Err(e) => Err(Box::new((e, tracker))),
        };
    }

    let mut req: MessagesRequest = match serde_json::from_value(raw) {
        Ok(req) => req,
        Err(e) => return Err(Box::new((e.into(), tracker))),
    };
    if target.provider.has_native_search() {
        req.prefer_native_search();
    }

    let missing_key = |name: &str| {
        ProxyError::unauthorized(format!(
            "{name} is not set (environment or ~/.claude-bridge/.env)"
        ))
    };
    let (tx, rx) = mpsc::channel(64);
    let (provider_name, model_name) = (target.provider.as_str(), target.model.clone());
    let writer = MessageWriter::new(tx.clone(), target.model.clone());
    let key = |name: &str| config::env(name).ok_or_else(|| missing_key(name));
    let provider: BoxFuture<'static, Result<Outcome, ProxyError>> = match target.provider {
        Provider::CodexOauth => {
            codex::run(state.clone(), req, target, CodexAuth::ChatGpt, writer).boxed()
        }
        Provider::Openai => match key("OPENAI_API_KEY") {
            Ok(key) => {
                codex::run(state.clone(), req, target, CodexAuth::ApiKey(key), writer).boxed()
            }
            Err(e) => return Err(Box::new((e, tracker))),
        },
        Provider::OpenaiCompat => match config::env("OPENAI_COMPAT_BASE_URL") {
            Some(base) => openai_compat::run(
                state.clone(),
                req,
                target,
                base,
                config::env("OPENAI_COMPAT_API_KEY"),
                writer,
            )
            .boxed(),
            None => return Err(Box::new((missing_key("OPENAI_COMPAT_BASE_URL"), tracker))),
        },
        Provider::GeminiOauth => {
            gemini::run(state.clone(), req, target, GeminiAuth::OAuth, writer).boxed()
        }
        Provider::Gemini => match key("GEMINI_API_KEY") {
            Ok(key) => {
                gemini::run(state.clone(), req, target, GeminiAuth::ApiKey(key), writer).boxed()
            }
            Err(e) => return Err(Box::new((e, tracker))),
        },
        Provider::Anthropic => unreachable!("handled above"),
    };
    tokio::spawn(async move {
        match provider.await {
            Ok(outcome) => {
                outcome.usage.log(provider_name, &model_name);
                state.log.publish(tracker.ok(&outcome));
            }
            Err(e) => {
                state.log.publish(tracker.failed(&e));
                if !e.is_client_gone() {
                    let _ = tx.send(Err(e)).await;
                }
            }
        }
    });

    Ok(respond(rx, stream).await)
}

/// Waits for the provider's first event so that failures before any output (auth,
/// rate limits, bad requests) become real HTTP errors Claude Code can retry or show.
async fn respond(mut rx: mpsc::Receiver<Result<Event, ProxyError>>, stream: bool) -> Response {
    let first = match rx.recv().await {
        Some(Ok(event)) => event,
        Some(Err(e)) => {
            warn!("{e}");
            return e.into_response();
        }
        None => return ProxyError::internal("provider ended without a response").into_response(),
    };

    if stream {
        let events = stream::once(async move { Ok(first) })
            .chain(ReceiverStream::new(rx))
            .map(|item| {
                let sse = match item {
                    Ok(event) => SseEvent::default()
                        .event(event.name())
                        .data(event.to_json().to_string()),
                    Err(e) => {
                        warn!("{e}");
                        SseEvent::default()
                            .event("error")
                            .data(e.body().to_string())
                    }
                };
                Ok::<_, Infallible>(sse)
            });
        // Keep-alive comments hold the connection open through long reasoning pauses.
        return Sse::new(events)
            .keep_alive(KeepAlive::default())
            .into_response();
    }

    let mut agg = Aggregator::default();
    agg.push(first);
    while let Some(item) = rx.recv().await {
        match item {
            Ok(event) => agg.push(event),
            Err(e) => {
                warn!("{e}");
                return e.into_response();
            }
        }
    }
    Json(agg.finish()).into_response()
}
