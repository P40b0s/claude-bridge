//! Anthropic Messages → OpenAI Responses API, the wire format the Codex CLI uses.
//! ChatGPT login goes to the ChatGPT Codex backend, an API key to api.openai.com.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde_json::{Value, json};
use tracing::{info, warn};

use super::reasoning_cache::{conversation_key, message_step_key, step_key, thread_id};
use crate::anthropic::{Block, Content, ImageSource, Message, MessageWriter, MessagesRequest, Outcome, Tool, ToolChoice, Usage};
use crate::config;
use crate::error::ProxyError;
use crate::routing::{Reasoning, Target};
use crate::server::AppState;

const OPENAI_API_BASE: &str = "https://api.openai.com/v1";
const CHATGPT_CODEX_BASE: &str = "https://chatgpt.com/backend-api/codex";
const ORIGINATOR: &str = "codex_cli_rs";
const DEFAULT_CODEX_CLI_VERSION: &str = "0.144.1";
const CONNECT_ATTEMPTS: u32 = 3;
/// Sticky-routing token the backend hands out on a turn's first response; replaying it keeps
/// the rest of the turn on the replica that holds the prompt cache.
const TURN_STATE_HEADER: &str = "x-codex-turn-state";

pub enum CodexAuth {
    ChatGpt,
    ApiKey(String),
}

// ── Request translation ──────────────────────────────────────────────

/// Function tools plus the native web_search tool the Codex CLI always registers.
pub fn to_responses_tools(tools: &[Tool]) -> Vec<Value> {
    let mut out: Vec<Value> = tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "name": t.name,
                "description": t.description.clone().unwrap_or_default(),
                "strict": false,
                "parameters": t.input_schema.clone().unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
            })
        })
        .collect();
    out.push(json!({ "type": "web_search" }));
    out
}

pub fn to_responses_tool_choice(choice: Option<&ToolChoice>) -> Value {
    match choice {
        Some(ToolChoice::Any) => json!("required"),
        Some(ToolChoice::None) => json!("none"),
        Some(ToolChoice::Tool { name }) => json!({ "type": "function", "name": name }),
        _ => json!("auto"),
    }
}

/// The ChatGPT backend rejects role "system" inside `input`; Claude Code sends MCP and
/// skill instructions that way, so every non-user/assistant role becomes "developer".
fn input_role(role: &str) -> &'static str {
    match role {
        "assistant" => "assistant",
        "user" => "user",
        _ => "developer",
    }
}

fn image_url(source: &ImageSource) -> String {
    match source {
        ImageSource::Base64 { media_type, data } => format!("data:{media_type};base64,{data}"),
        ImageSource::Url { url, .. } => url.clone(),
        ImageSource::Other => String::new(),
    }
}

fn text_part(role: &str, text: &str) -> Value {
    let kind = if role == "assistant" { "output_text" } else { "input_text" };
    json!({ "type": kind, "text": text })
}

/// Each message in the order Claude Code sent it, never reshuffled: the prompt cache only
/// reuses an exact prefix of the previous request. Tool calls and results become top-level
/// items after the message text they belong to, and each assistant step is preceded by the
/// cached reasoning that produced it.
pub fn to_responses_input(messages: &[Message], reasoning: &HashMap<String, Vec<Value>>) -> Vec<Value> {
    let mut out = Vec::new();
    for m in messages {
        if m.role == "assistant"
            && let Some(items) = reasoning.get(&message_step_key(m))
        {
            out.extend(items.iter().cloned());
        }
        let role = input_role(&m.role);
        let blocks = match &m.content {
            Content::Text(text) => {
                out.push(json!({ "type": "message", "role": role, "content": [text_part(&m.role, text)] }));
                continue;
            }
            Content::Blocks(blocks) => blocks,
        };

        let mut parts = Vec::new();
        let mut trailing = Vec::new();
        for block in blocks {
            match block {
                Block::Text { text } => parts.push(text_part(&m.role, text)),
                Block::Image { source } => parts.push(json!({ "type": "input_image", "image_url": image_url(source) })),
                Block::ToolUse { id, name, input } => {
                    let arguments = match input {
                        Value::String(s) => s.clone(),
                        Value::Null => "{}".into(),
                        other => other.to_string(),
                    };
                    trailing.push(json!({ "type": "function_call", "call_id": id, "name": name, "arguments": arguments }));
                }
                Block::ToolResult { tool_use_id, content } => {
                    trailing.push(json!({ "type": "function_call_output", "call_id": tool_use_id, "output": content.to_text() }));
                }
                // Replayed Anthropic thinking is not reusable Codex state; only the encrypted
                // reasoning from our cache is sent back.
                Block::Other => {}
            }
        }
        if !parts.is_empty() {
            out.push(json!({ "type": "message", "role": role, "content": parts }));
        }
        out.extend(trailing);
    }
    out
}

/// A request that answers a fresh user message rather than continuing a tool loop.
fn starts_user_turn(messages: &[Message]) -> bool {
    match messages.iter().rev().find(|m| m.role == "user").map(|m| &m.content) {
        Some(Content::Blocks(blocks)) => !blocks.iter().any(|b| matches!(b, Block::ToolResult { .. })),
        _ => true,
    }
}

const COMPACTION_MARKERS: [&str; 2] =
    ["CRITICAL: Respond with TEXT ONLY. Do NOT call any tools.", "Your task is to create a detailed summary of"];

/// Claude Code's generated compaction turn (not a user asking for a summary).
pub fn is_compaction_request(req: &MessagesRequest) -> bool {
    let Some(last_user) = req.messages.iter().rev().find(|m| m.role == "user") else {
        return false;
    };
    let text = match &last_user.content {
        Content::Text(text) => text.clone(),
        Content::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                Block::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    };
    COMPACTION_MARKERS.iter().all(|m| text.contains(m))
}

/// Codex has no thinking-off, so none/minimal floor to low. Compaction is capped at medium.
pub fn effective_effort(req: &MessagesRequest, level: Option<Reasoning>) -> &'static str {
    let effort = match level {
        Some(Reasoning::None | Reasoning::Minimal) => "low",
        Some(level) => level.as_str(),
        None => match config::env("CODEX_REASONING_EFFORT").as_deref().and_then(Reasoning::parse) {
            Some(Reasoning::Low) => "low",
            Some(Reasoning::Medium) => "medium",
            Some(Reasoning::Xhigh) => "xhigh",
            Some(Reasoning::Max) => "max",
            _ => "high",
        },
    };
    if is_compaction_request(req) && !matches!(effort, "low" | "medium") { "medium" } else { effort }
}

fn reasoning_summary() -> Option<&'static str> {
    match config::env("CODEX_REASONING_SUMMARY").map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("none") => None,
        Some("concise") => Some("concise"),
        Some("detailed") => Some("detailed"),
        _ => Some("auto"),
    }
}

fn text_verbosity() -> Option<&'static str> {
    match config::env("CODEX_TEXT_VERBOSITY").map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("low") => Some("low"),
        Some("medium") => Some("medium"),
        Some("high") => Some("high"),
        _ => None,
    }
}

fn installation_id() -> Option<String> {
    let id = std::fs::read_to_string(config::home_dir().join(".codex").join("installation_id")).ok()?;
    Some(id.trim().to_string()).filter(|s| !s.is_empty())
}

fn user_agent() -> String {
    let version = config::env("CODEX_CLI_VERSION").unwrap_or_else(|| DEFAULT_CODEX_CLI_VERSION.into());
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    let os = match std::env::consts::OS {
        "linux" => "Linux",
        "macos" => "Darwin",
        other => other,
    };
    format!("{ORIGINATOR}/{version} ({os} {}; {})", release.trim(), std::env::consts::ARCH)
}

// ── Main adapter ─────────────────────────────────────────────────────

pub async fn run(
    state: Arc<AppState>,
    req: MessagesRequest,
    target: Target,
    auth: CodexAuth,
    w: MessageWriter,
) -> Result<Outcome, ProxyError> {
    let (backend, url, token, account_id) = match auth {
        CodexAuth::ApiKey(key) => ("api", format!("{OPENAI_API_BASE}/responses"), key, None),
        CodexAuth::ChatGpt => {
            let creds = state.openai_auth.credentials(&state.http).await?;
            ("chatgpt", format!("{CHATGPT_CODEX_BASE}/responses"), creds.access_token, creds.account_id)
        }
    };

    // Per model: encrypted reasoning from one model is useless to another, and Claude Code's
    // internal calls on the fast model must not overwrite the main model's entry.
    let thread = conversation_key(&req.messages);
    let convo_key = format!("{}:{thread}", target.model);
    let cache_enabled = config::env("CODEX_DISABLE_REASONING_CACHE").as_deref() != Some("1");
    let cached = if cache_enabled { state.reasoning_cache.get(&convo_key) } else { HashMap::new() };
    let cached_count = cached.len();
    let input = to_responses_input(&req.messages, &cached);
    let item_count = input.len();
    let effort = effective_effort(&req, target.reasoning);
    // ChatGPT picks the cache replica from the session-id header, not from prompt_cache_key;
    // without it requests scatter and only hit whatever prefix each replica saw last.
    let session_id = thread_id(&thread);
    let turn_state = state.reasoning_cache.turn_state(&convo_key, starts_user_turn(&req.messages));

    let mut reasoning = json!({ "effort": effort });
    if let Some(summary) = reasoning_summary() {
        reasoning["summary"] = json!(summary);
    }
    let mut body = json!({
        "model": target.model,
        "instructions": req.system_text(),
        "input": input,
        "tools": to_responses_tools(&req.tools),
        "tool_choice": to_responses_tool_choice(req.tool_choice.as_ref()),
        "parallel_tool_calls": false,
        "reasoning": reasoning,
        "store": false,
        "stream": true,
        "include": ["reasoning.encrypted_content"],
        "prompt_cache_key": session_id,
    });
    if let Some(verbosity) = text_verbosity() {
        body["text"] = json!({ "verbosity": verbosity });
    }
    let install_id = installation_id();
    if let Some(id) = &install_id {
        body["client_metadata"] = json!({ "x-codex-installation-id": id });
    }
    let body = serde_json::to_vec(&body).map_err(|e| ProxyError::internal(e.to_string()))?;

    info!(
        model = %target.model,
        backend,
        effort,
        items = item_count,
        cached_reasoning = cached_count,
        sticky = turn_state.is_some(),
        compaction = is_compaction_request(&req),
        req_kib = body.len() / 1024,
        "codex request"
    );

    let mut attempt = 0;
    let resp = loop {
        attempt += 1;
        let mut rb = state
            .http
            .post(&url)
            .bearer_auth(&token)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .header("originator", ORIGINATOR)
            .header("user-agent", user_agent())
            .header("openai-beta", "responses=experimental")
            .header("session-id", &session_id)
            .header("thread-id", &session_id)
            .header("x-client-request-id", &session_id)
            .body(body.clone());
        if backend == "chatgpt" {
            rb = rb.header("x-codex-routing-hint", format!("model={}", target.model));
        }
        if let Some(turn_state) = &turn_state {
            rb = rb.header(TURN_STATE_HEADER, turn_state);
        }
        if let Some(id) = &install_id {
            rb = rb.header("x-codex-installation-id", id);
        }
        if let Some(account) = &account_id {
            rb = rb.header("chatgpt-account-id", account);
        }
        match rb.send().await {
            Ok(resp) => break resp,
            // The request never reached the server (typically a pooled socket the backend
            // already closed), so re-sending it is safe.
            Err(e) if (e.is_connect() || e.is_request()) && attempt < CONNECT_ATTEMPTS => {
                let backoff = Duration::from_millis(250 * u64::from(attempt));
                warn!("codex connect failed (attempt {attempt}/{CONNECT_ATTEMPTS}): {}", ProxyError::from(e));
                tokio::time::sleep(backoff).await;
            }
            Err(e) => return Err(e.into()),
        }
    };

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        return Err(ProxyError::upstream("OpenAI", status, &text));
    }
    if let Some(value) = resp.headers().get(TURN_STATE_HEADER).and_then(|v| v.to_str().ok()) {
        state.reasoning_cache.set_turn_state(&convo_key, value.to_string());
    }

    let mut pump = Pump::new(w);
    let mut events = resp.bytes_stream().eventsource();
    while let Some(event) = events.next().await {
        let event = event.map_err(|e| ProxyError::new(axum::http::StatusCode::BAD_GATEWAY, format!("OpenAI stream: {e}")))?;
        if event.data.is_empty() || event.data == "[DONE]" {
            continue;
        }
        let Ok(json) = serde_json::from_str::<Value>(&event.data) else { continue };
        pump.handle(&json).await?;
    }

    info!(
        text_chars = pump.text_chars,
        tool_calls = pump.tool_calls,
        input = pump.usage.input_tokens,
        cache_read = pump.usage.cache_read_input_tokens,
        output = pump.usage.output_tokens,
        stop = pump.stop_reason,
        "codex done"
    );
    if !pump.reasoning.is_empty() {
        let step = step_key(pump.first_call_id.as_deref(), &pump.text);
        state.reasoning_cache.add_step(&convo_key, step, std::mem::take(&mut pump.reasoning));
    }
    pump.w.finish(pump.stop_reason, pump.usage).await
}

/// OpenAI counts cached tokens inside input_tokens; Anthropic reports them separately.
pub fn split_input_usage(total: i64, cached: i64) -> (u64, u64) {
    let total = total.max(0);
    let cached = cached.clamp(0, total);
    ((total - cached) as u64, cached as u64)
}

// ── Responses SSE → Anthropic events ─────────────────────────────────

struct Pump {
    w: MessageWriter,
    text_chars: usize,
    /// The answer text and first tool call id, which identify this step on later requests.
    text: String,
    first_call_id: Option<String>,
    tool_calls: usize,
    /// Item id of the function_call whose block is open, and whether any arguments streamed.
    current_tool: Option<(String, bool)>,
    stop_reason: &'static str,
    usage: Usage,
    reasoning: Vec<Value>,
}

impl Pump {
    fn new(w: MessageWriter) -> Self {
        Self { w, text_chars: 0, text: String::new(), first_call_id: None, tool_calls: 0, current_tool: None, stop_reason: "end_turn", usage: Usage::default(), reasoning: Vec::new() }
    }

    async fn text(&mut self, text: &str) -> Result<(), ProxyError> {
        self.text_chars += text.len();
        self.text.push_str(text);
        self.w.text(text).await
    }

    /// Some streams only carry the text in a final event; use it when no deltas came.
    async fn fallback_text(&mut self, text: Option<&str>) -> Result<(), ProxyError> {
        match text {
            Some(text) if self.text_chars == 0 && !text.is_empty() => self.text(text).await,
            _ => Ok(()),
        }
    }

    fn take_usage(&mut self, response: &Value) {
        let u = &response["usage"];
        if u.is_object() {
            let (input, cached) = split_input_usage(
                u["input_tokens"].as_i64().unwrap_or(0),
                u["input_tokens_details"]["cached_tokens"].as_i64().unwrap_or(0),
            );
            self.usage.input_tokens = input;
            self.usage.cache_read_input_tokens = cached;
            self.usage.output_tokens = u["output_tokens"].as_u64().unwrap_or(0);
        }
    }

    async fn handle(&mut self, ev: &Value) -> Result<(), ProxyError> {
        let Some(kind) = ev["type"].as_str() else { return Ok(()) };
        match kind {
            // Start the reply right away so headers go out before the model starts thinking.
            "response.created" => self.w.start().await,

            "response.output_text.delta" => match ev["delta"].as_str() {
                Some(delta) => self.text(delta).await,
                None => Ok(()),
            },
            "response.output_text.done" => self.fallback_text(ev["text"].as_str()).await,
            "response.content_part.done" => {
                let part = &ev["part"];
                let is_text = matches!(part["type"].as_str(), Some("output_text" | "text"));
                self.fallback_text(if is_text { part["text"].as_str() } else { None }).await
            }

            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                self.w.thinking(ev["delta"].as_str().unwrap_or_default()).await
            }
            // Paragraph break between summary parts inside one thinking block.
            "response.reasoning_summary_part.added" if self.w.thinking_open() => self.w.thinking("\n\n").await,

            "response.output_item.added" => {
                let item = &ev["item"];
                match item["type"].as_str() {
                    Some("function_call") => {
                        let call_id = item["call_id"].as_str().or(item["id"].as_str()).unwrap_or_default().to_string();
                        let item_id = item["id"].as_str().map(String::from).unwrap_or_else(|| call_id.clone());
                        self.w.start_tool(&call_id, item["name"].as_str().unwrap_or_default()).await?;
                        self.first_call_id.get_or_insert_with(|| call_id.clone());
                        self.current_tool = Some((item_id, false));
                        self.tool_calls += 1;
                        self.stop_reason = "tool_use";
                    }
                    Some("web_search_call") => info!("codex web_search invoked"),
                    _ => {}
                }
                Ok(())
            }
            "response.function_call_arguments.delta" => {
                let item_id = ev["item_id"].as_str().or(ev["id"].as_str());
                let Some((current, streamed)) = &mut self.current_tool else { return Ok(()) };
                if item_id != Some(current.as_str()) {
                    return Ok(());
                }
                *streamed = true;
                self.w.tool_args(ev["delta"].as_str().unwrap_or_default()).await
            }
            "response.output_item.done" => {
                let item = &ev["item"];
                match item["type"].as_str() {
                    Some("function_call") => {
                        let item_id = item["id"].as_str().or(item["call_id"].as_str());
                        if let Some((current, streamed)) = self.current_tool.take() {
                            if item_id != Some(current.as_str()) {
                                self.current_tool = Some((current, streamed));
                                return Ok(());
                            }
                            if !streamed {
                                self.w.tool_args(item["arguments"].as_str().unwrap_or_default()).await?;
                            }
                            self.w.close_block().await?;
                        }
                        Ok(())
                    }
                    Some("reasoning") => {
                        let summary: Vec<Value> = item["summary"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|p| json!({ "type": "summary_text", "text": p["text"].as_str().unwrap_or_default() }))
                            .collect();
                        self.reasoning.push(json!({
                            "type": "reasoning",
                            "summary": summary,
                            "encrypted_content": item["encrypted_content"],
                        }));
                        Ok(())
                    }
                    Some("message") => {
                        let text: String = item["content"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|p| matches!(p["type"].as_str(), Some("output_text" | "text")))
                            .filter_map(|p| p["text"].as_str())
                            .collect();
                        self.fallback_text(Some(&text)).await
                    }
                    _ => Ok(()),
                }
            }

            "response.completed" => {
                self.take_usage(&ev["response"]);
                Ok(())
            }
            "response.incomplete" => {
                let response = &ev["response"];
                self.take_usage(response);
                match response["incomplete_details"]["reason"].as_str() {
                    Some("max_output_tokens") => {
                        self.stop_reason = "max_tokens";
                        Ok(())
                    }
                    reason => Err(ProxyError::internal(format!("OpenAI response incomplete: {}", reason.unwrap_or("unknown")))),
                }
            }
            "response.failed" => {
                let msg = ev["response"]["error"]["message"].as_str().unwrap_or("OpenAI response failed");
                Err(ProxyError::internal(msg.to_string()))
            }
            "error" => {
                let msg = ev["message"].as_str().or(ev["error"]["message"].as_str()).unwrap_or("OpenAI stream error");
                Err(ProxyError::internal(msg.to_string()))
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anthropic::Event;
    use tokio::sync::mpsc;

    fn msgs(v: Value) -> Vec<Message> {
        let req: MessagesRequest = serde_json::from_value(json!({ "model": "x", "messages": v })).unwrap();
        req.messages
    }

    fn input(v: Value) -> Vec<Value> {
        let m = msgs(v);
        to_responses_input(&m, &HashMap::new())
    }

    #[test]
    fn tools_get_web_search_and_default_schema() {
        let tools: Vec<Tool> = serde_json::from_value(json!([
            { "name": "Bash", "description": "Run", "input_schema": { "type": "object", "properties": { "command": { "type": "string" } } } },
            { "name": "NoSchema" }
        ]))
        .unwrap();
        let out = to_responses_tools(&tools);
        assert_eq!(out[0]["type"], "function");
        assert_eq!(out[0]["strict"], false);
        assert_eq!(out[0]["parameters"]["properties"]["command"]["type"], "string");
        assert_eq!(out[1]["parameters"], json!({ "type": "object", "properties": {} }));
        assert_eq!(out[2], json!({ "type": "web_search" }));
        assert_eq!(to_responses_tools(&[]), vec![json!({ "type": "web_search" })]);
    }

    #[test]
    fn tool_choice_mapping() {
        assert_eq!(to_responses_tool_choice(None), json!("auto"));
        assert_eq!(to_responses_tool_choice(Some(&ToolChoice::Auto)), json!("auto"));
        assert_eq!(to_responses_tool_choice(Some(&ToolChoice::Any)), json!("required"));
        assert_eq!(to_responses_tool_choice(Some(&ToolChoice::None)), json!("none"));
        assert_eq!(
            to_responses_tool_choice(Some(&ToolChoice::Tool { name: "Bash".into() })),
            json!({ "type": "function", "name": "Bash" })
        );
    }

    #[test]
    fn plain_and_assistant_text() {
        assert_eq!(
            input(json!([{ "role": "user", "content": "hello" }])),
            vec![json!({ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hello" }] })]
        );
        assert_eq!(
            input(json!([{ "role": "assistant", "content": [{ "type": "text", "text": "hi" }] }]))[0]["content"][0],
            json!({ "type": "output_text", "text": "hi" })
        );
        assert_eq!(input(json!([{ "role": "system", "content": "mcp" }]))[0]["role"], "developer");
    }

    #[test]
    fn tool_use_and_result_become_top_level_items() {
        let out = input(json!([
            { "role": "assistant", "content": [
                { "type": "text", "text": "running" },
                { "type": "tool_use", "id": "call_1", "name": "Bash", "input": { "command": "ls" } }
            ]},
            { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "call_1", "content": [
                    { "type": "text", "text": "a.txt" },
                    { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "AA==" } }
                ]}
            ]}
        ]));
        assert_eq!(out.len(), 3);
        assert_eq!(out[0]["type"], "message");
        assert_eq!(out[1], json!({ "type": "function_call", "call_id": "call_1", "name": "Bash", "arguments": "{\"command\":\"ls\"}" }));
        assert_eq!(out[2], json!({ "type": "function_call_output", "call_id": "call_1", "output": "a.txt\n[image: image/png]" }));
    }

    #[test]
    fn images_and_dropped_thinking() {
        let out = input(json!([{ "role": "user", "content": [
            { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "AA==" } },
            { "type": "image", "source": { "type": "url", "url": "https://x/y.png" } }
        ]}]));
        assert_eq!(out[0]["content"][0]["image_url"], "data:image/png;base64,AA==");
        assert_eq!(out[0]["content"][1]["image_url"], "https://x/y.png");

        let out = input(json!([{ "role": "assistant", "content": [
            { "type": "thinking", "thinking": "long", "signature": "s" },
            { "type": "text", "text": "answer" }
        ]}]));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["content"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn tool_results_continue_the_user_turn() {
        assert!(starts_user_turn(&msgs(json!([{ "role": "user", "content": "q" }]))));
        assert!(!starts_user_turn(&msgs(json!([
            { "role": "user", "content": "q" },
            { "role": "assistant", "content": [{ "type": "tool_use", "id": "t1", "name": "Read", "input": {} }] },
            { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": "ok" }] }
        ]))));
    }

    #[test]
    fn each_step_gets_its_own_reasoning_back() {
        let m = msgs(json!([
            { "role": "user", "content": "q1" },
            { "role": "assistant", "content": [{ "type": "tool_use", "id": "t1", "name": "Read", "input": {} }] },
            { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": "ok" }] },
            { "role": "system", "content": "ctx" },
            { "role": "assistant", "content": [{ "type": "text", "text": "a2" }] },
            { "role": "user", "content": "q3" },
            { "role": "assistant", "content": "a3" },
            { "role": "user", "content": "q4" }
        ]));
        let r = |enc: &str| vec![json!({ "type": "reasoning", "summary": [], "encrypted_content": enc })];
        let cached = HashMap::from([(step_key(Some("t1"), ""), r("r1")), (step_key(None, "a2"), r("r2"))]);
        let out = to_responses_input(&m, &cached);
        let kinds: Vec<String> = out
            .iter()
            .map(|i| i["role"].as_str().or(i["encrypted_content"].as_str()).or(i["type"].as_str()).unwrap().to_string())
            .collect();
        assert_eq!(
            kinds,
            ["user", "r1", "function_call", "function_call_output", "developer", "r2", "assistant", "user", "assistant", "user"]
        );
    }

    #[test]
    fn usage_split() {
        assert_eq!(split_input_usage(1000, 600), (400, 600));
        assert_eq!(split_input_usage(100, 500), (0, 100));
        assert_eq!(split_input_usage(100, -5), (100, 0));
    }

    #[test]
    fn compaction_caps_effort() {
        let mut req: MessagesRequest = serde_json::from_value(json!({
            "model": "x",
            "messages": [{ "role": "user", "content": format!("{} {}", COMPACTION_MARKERS[0], COMPACTION_MARKERS[1]) }]
        }))
        .unwrap();
        assert_eq!(effective_effort(&req, Some(Reasoning::Max)), "medium");
        assert_eq!(effective_effort(&req, Some(Reasoning::Low)), "low");
        req.messages[0].content = Content::Text("Your task is to create a detailed summary of the repo".into());
        assert_eq!(effective_effort(&req, Some(Reasoning::Max)), "max");
        assert_eq!(effective_effort(&req, Some(Reasoning::None)), "low");
    }

    async fn pump(events: Vec<Value>) -> (Vec<Event>, Pump) {
        let (tx, mut rx) = mpsc::channel(256);
        let mut p = Pump::new(MessageWriter::new(tx, "gpt"));
        for ev in &events {
            p.handle(ev).await.unwrap();
        }
        let mut out = Vec::new();
        while let Ok(Ok(ev)) = rx.try_recv() {
            out.push(ev);
        }
        (out, p)
    }

    #[tokio::test]
    async fn streams_reasoning_text_and_tool_call() {
        let (events, p) = pump(vec![
            json!({ "type": "response.created" }),
            json!({ "type": "response.reasoning_summary_part.added" }),
            json!({ "type": "response.reasoning_summary_text.delta", "delta": "plan" }),
            json!({ "type": "response.output_item.done", "item": { "type": "reasoning", "summary": [{ "type": "summary_text", "text": "plan" }], "encrypted_content": "enc" } }),
            json!({ "type": "response.output_text.delta", "delta": "Hi" }),
            json!({ "type": "response.output_text.done", "text": "Hi" }),
            json!({ "type": "response.output_item.added", "item": { "type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "Bash" } }),
            json!({ "type": "response.function_call_arguments.delta", "item_id": "fc_1", "delta": "{\"command\":\"ls\"}" }),
            json!({ "type": "response.output_item.done", "item": { "type": "function_call", "id": "fc_1", "call_id": "call_1", "arguments": "{\"command\":\"ls\"}" } }),
            json!({ "type": "response.completed", "response": { "usage": { "input_tokens": 100, "output_tokens": 7, "input_tokens_details": { "cached_tokens": 40 } } } }),
        ])
        .await;
        let names: Vec<&str> = events.iter().map(Event::name).collect();
        assert_eq!(
            names,
            vec![
                "message_start",
                "content_block_start", "content_block_delta", "content_block_stop",
                "content_block_start", "content_block_delta", "content_block_stop",
                "content_block_start", "content_block_delta", "content_block_stop",
            ]
        );
        assert_eq!(p.text_chars, 2, "done event must not duplicate streamed text");
        assert_eq!(p.stop_reason, "tool_use");
        assert_eq!(p.usage.input_tokens, 60);
        assert_eq!(p.usage.cache_read_input_tokens, 40);
        assert_eq!(p.reasoning[0]["encrypted_content"], "enc");
    }

    #[tokio::test]
    async fn tool_arguments_only_in_done_event_are_kept() {
        let (events, _) = pump(vec![
            json!({ "type": "response.output_item.added", "item": { "type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "Read" } }),
            json!({ "type": "response.output_item.done", "item": { "type": "function_call", "id": "fc_1", "arguments": "{\"path\":\"a\"}" } }),
        ])
        .await;
        assert!(events.iter().any(|e| matches!(e,
            Event::ContentBlockDelta { delta: crate::anthropic::Delta::InputJsonDelta { partial_json }, .. } if partial_json == "{\"path\":\"a\"}")));
    }

    #[tokio::test]
    async fn max_output_tokens_is_a_stop_reason_not_an_error() {
        let (_, p) = pump(vec![json!({ "type": "response.incomplete", "response": { "incomplete_details": { "reason": "max_output_tokens" } } })]).await;
        assert_eq!(p.stop_reason, "max_tokens");
    }
}
