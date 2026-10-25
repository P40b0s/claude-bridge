//! Anthropic Messages → Gemini `streamGenerateContent`, through Code Assist (Google login
//! with a Code Assist project) or the Generative Language API (API key, or login without one).

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::StatusCode;
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde_json::{Map, Value, json};
use tracing::{info, warn};

use crate::anthropic::{Block, Content, ImageSource, Message, MessageWriter, MessagesRequest, Outcome, Tool, Usage};
use crate::auth::google::CODE_ASSIST_ENDPOINT;
use crate::config;
use crate::error::ProxyError;
use crate::routing::{Reasoning, Target};
use crate::server::AppState;

const DEFAULT_GL_ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta";
/// Gemini 3 wants thought signatures on replayed function calls; we do not keep them.
const SKIP_THOUGHT_SIGNATURE: &str = "skip_thought_signature_validator";

pub enum GeminiAuth {
    OAuth,
    ApiKey(String),
}

// ── Request translation ──────────────────────────────────────────────

fn tool_names(messages: &[Message]) -> HashMap<&str, &str> {
    messages
        .iter()
        .filter_map(|m| match &m.content {
            Content::Blocks(blocks) => Some(blocks),
            Content::Text(_) => None,
        })
        .flatten()
        .filter_map(|b| match b {
            Block::ToolUse { id, name, .. } => Some((id.as_str(), name.as_str())),
            _ => None,
        })
        .collect()
}

fn to_parts(content: &Content, tool_names: &HashMap<&str, &str>) -> Vec<Value> {
    let blocks = match content {
        Content::Text(text) if text.is_empty() => return vec![json!({ "text": " " })],
        Content::Text(text) => return vec![json!({ "text": text })],
        Content::Blocks(blocks) => blocks,
    };
    let mut parts = Vec::new();
    for block in blocks {
        match block {
            Block::Text { text } if !text.is_empty() => parts.push(json!({ "text": text })),
            Block::Image { source: ImageSource::Base64 { media_type, data } } => {
                parts.push(json!({ "inlineData": { "mimeType": media_type, "data": data } }))
            }
            Block::Image { source: ImageSource::Url { url, media_type } } => parts.push(json!({
                "fileData": { "fileUri": url, "mimeType": media_type.as_deref().unwrap_or("image/png") }
            })),
            Block::ToolUse { name, input, .. } => {
                let args = match input {
                    Value::String(s) => serde_json::from_str(s).unwrap_or_else(|_| json!({})),
                    Value::Null => json!({}),
                    other => other.clone(),
                };
                parts.push(json!({ "functionCall": { "name": name, "args": args }, "thoughtSignature": SKIP_THOUGHT_SIGNATURE }));
            }
            Block::ToolResult { tool_use_id, content } => {
                let name = tool_names.get(tool_use_id.as_str()).copied().unwrap_or("unknown_tool");
                parts.push(json!({ "functionResponse": { "name": name, "response": { "content": content.to_text() } } }));
            }
            _ => {}
        }
    }
    if parts.is_empty() {
        parts.push(json!({ "text": " " }));
    }
    parts
}

/// Gemini needs alternating user/model turns, so consecutive same-role messages merge.
pub fn to_contents(messages: &[Message]) -> Vec<Value> {
    let names = tool_names(messages);
    let mut merged: Vec<(&'static str, Vec<Value>)> = Vec::new();
    for m in messages {
        let role = if m.role == "assistant" { "model" } else { "user" };
        let parts = to_parts(&m.content, &names);
        match merged.last_mut() {
            Some((last_role, last_parts)) if *last_role == role => last_parts.extend(parts),
            _ => merged.push((role, parts)),
        }
    }
    merged.into_iter().map(|(role, parts)| json!({ "role": role, "parts": parts })).collect()
}

const SCHEMA_KEYS: [&str; 23] = [
    "type", "properties", "required", "description", "enum", "items", "format", "nullable", "title",
    "anyOf", "$ref", "$defs", "$id", "$anchor", "minimum", "maximum", "minItems", "maxItems",
    "prefixItems", "additionalProperties", "propertyOrdering", "minLength", "maxLength",
];

/// Drops JSON Schema keywords Gemini rejects (`$schema`, `exclusiveMinimum`, ...).
/// Keys directly under `properties` are property names and are kept.
pub fn sanitize_schema(value: &Value, is_property_map: bool) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(|v| sanitize_schema(v, false)).collect()),
        Value::Object(obj) => {
            let clean: Map<String, Value> = obj
                .iter()
                .filter(|(k, _)| is_property_map || SCHEMA_KEYS.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), sanitize_schema(v, !is_property_map && k == "properties")))
                .collect();
            Value::Object(clean)
        }
        other => other.clone(),
    }
}

fn to_tools(tools: &[Tool]) -> Vec<Value> {
    let mut out = Vec::new();
    if !tools.is_empty() {
        let decls: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description.clone().unwrap_or_default(),
                    "parameters": t.input_schema.as_ref()
                        .map(|s| sanitize_schema(s, false))
                        .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
                })
            })
            .collect();
        out.push(json!({ "functionDeclarations": decls }));
    }
    out.push(json!({ "google_search": {} }));
    out
}

fn is_gemini3(model: &str) -> bool {
    model
        .strip_prefix("gemini-3")
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.') || rest.starts_with('-'))
}

/// Gemini 3 takes a thinking level; 2.5 takes a token budget (-1 = dynamic).
pub fn thinking_config(model: &str, level: Option<Reasoning>) -> Value {
    if is_gemini3(model) {
        // gemini-3-pro-preview only knows LOW and HIGH.
        let limited = model == "gemini-3-pro-preview";
        let level = match level {
            Some(Reasoning::None | Reasoning::Minimal) if limited => "LOW",
            Some(Reasoning::None | Reasoning::Minimal) => "MINIMAL",
            Some(Reasoning::Low) => "LOW",
            Some(Reasoning::Medium) if !limited => "MEDIUM",
            _ => "HIGH",
        };
        json!({ "includeThoughts": true, "thinkingLevel": level })
    } else {
        let budget = match level {
            Some(Reasoning::None | Reasoning::Minimal | Reasoning::Low) => 1024,
            Some(Reasoning::Medium) => 8192,
            _ => -1,
        };
        json!({ "includeThoughts": true, "thinkingBudget": budget })
    }
}

fn build_request(req: &MessagesRequest, target: &Target, project: Option<&str>) -> Value {
    let mut contents = to_contents(&req.messages);
    let mut generation = json!({
        "temperature": req.temperature.unwrap_or(1.0),
        "topP": 0.95,
        "topK": 64,
        "thinkingConfig": thinking_config(&target.model, target.reasoning),
    });
    if let Some(max) = req.max_tokens {
        generation["maxOutputTokens"] = json!(max);
    }
    let tools = to_tools(&req.tools);
    let system = req.system_text();

    match project {
        Some(project) => {
            // Code Assist's systemInstruction has a different schema; inline the prompt instead.
            if !system.is_empty() {
                let wrapped = format!("[System Instructions]\n{system}\n[End System Instructions]\n\n");
                match contents.first_mut() {
                    Some(first) if first["role"] == "user" => {
                        first["parts"].as_array_mut().unwrap().insert(0, json!({ "text": wrapped }))
                    }
                    _ => contents.insert(0, json!({ "role": "user", "parts": [{ "text": wrapped.trim_end() }] })),
                }
            }
            json!({
                "model": target.model,
                "project": project,
                "user_prompt_id": uuid::Uuid::new_v4().to_string(),
                "request": { "contents": contents, "generationConfig": generation, "tools": tools },
            })
        }
        None => {
            let mut body = json!({ "contents": contents, "generationConfig": generation, "tools": tools });
            if !system.is_empty() {
                body["systemInstruction"] = json!({ "role": "user", "parts": [{ "text": system }] });
            }
            body
        }
    }
}

// ── Main adapter ─────────────────────────────────────────────────────

enum AttemptError {
    /// Rate limited before anything was sent to the client; another account may be tried.
    RateLimited(ProxyError),
    Fatal(ProxyError),
}

impl From<ProxyError> for AttemptError {
    fn from(e: ProxyError) -> Self {
        AttemptError::Fatal(e)
    }
}

pub async fn run(
    state: Arc<AppState>,
    req: MessagesRequest,
    target: Target,
    auth: GeminiAuth,
    mut w: MessageWriter,
) -> Result<Outcome, ProxyError> {
    let api_key = match auth {
        GeminiAuth::ApiKey(key) => Some(key),
        GeminiAuth::OAuth => None,
    };
    let result = match attempt(&state, &req, &target, api_key.as_deref(), 1, &mut w).await {
        Err(AttemptError::RateLimited(_)) if api_key.is_none() && crate::auth::google::load(2).is_some() => {
            warn!("Google account 1 rate limited, failing over to account 2");
            attempt(&state, &req, &target, None, 2, &mut w).await
        }
        other => other,
    };
    match result {
        Ok((stop_reason, usage)) => w.finish(stop_reason, usage).await,
        Err(AttemptError::RateLimited(e) | AttemptError::Fatal(e)) => Err(e),
    }
}

fn rate_limit_or_fatal(e: ProxyError) -> AttemptError {
    if e.status == StatusCode::TOO_MANY_REQUESTS { AttemptError::RateLimited(e) } else { AttemptError::Fatal(e) }
}

async fn attempt(
    state: &AppState,
    req: &MessagesRequest,
    target: &Target,
    api_key: Option<&str>,
    account: u8,
    w: &mut MessageWriter,
) -> Result<(&'static str, Usage), AttemptError> {
    let (rb, project) = match api_key {
        Some(key) => {
            let base = config::env("GEMINI_BASE_URL").unwrap_or_else(|| DEFAULT_GL_ENDPOINT.into());
            let url = format!("{base}/models/{}:streamGenerateContent?alt=sse", target.model);
            (state.http.post(url).header("x-goog-api-key", key), None)
        }
        None => {
            let tokens = state.google_auth.credentials(&state.http, account).await?;
            let url = match &tokens.project_id {
                Some(_) => format!("{CODE_ASSIST_ENDPOINT}:streamGenerateContent?alt=sse"),
                None => format!("{DEFAULT_GL_ENDPOINT}/models/{}:streamGenerateContent?alt=sse", target.model),
            };
            let rb = state
                .http
                .post(url)
                .bearer_auth(&tokens.access_token)
                .header("user-agent", "google-api-nodejs-client/9.15.1")
                .header("x-goog-api-client", "gl-node/22.17.0")
                .header("client-metadata", "ideType=IDE_UNSPECIFIED,platform=PLATFORM_UNSPECIFIED,pluginType=GEMINI");
            (rb, tokens.project_id)
        }
    };

    let body = build_request(req, target, project.as_deref());
    let thinking = thinking_config(&target.model, target.reasoning);
    info!(
        model = %target.model,
        api = if project.is_some() { "code-assist" } else { "generative-language" },
        account,
        thinking = %thinking,
        "gemini request"
    );

    let resp = rb.json(&body).send().await.map_err(ProxyError::from)?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        return Err(rate_limit_or_fatal(ProxyError::upstream("Gemini", status, &text)));
    }

    let mut stop_reason = "end_turn";
    let mut usage = Usage::default();
    let mut tool_calls = 0usize;
    let mut events = resp.bytes_stream().eventsource();
    while let Some(event) = events.next().await {
        let event = event.map_err(|e| ProxyError::new(StatusCode::BAD_GATEWAY, format!("Gemini stream: {e}")))?;
        let Ok(json) = serde_json::from_str::<Value>(&event.data) else { continue };

        // Code Assist answers 200 and puts quota errors inside the stream.
        let error = [&json["error"], &json[0]["error"], &json["response"]["error"]].into_iter().find(|e| e["code"].is_number());
        if let Some(error) = error {
            let code = error["code"].as_u64().unwrap_or(500) as u16;
            let err = ProxyError::upstream("Gemini", code, &error.to_string());
            return Err(if w.started() { AttemptError::Fatal(err) } else { rate_limit_or_fatal(err) });
        }

        let data = if json["response"].is_object() { &json["response"] } else { &json };
        let candidate = &data["candidates"][0];
        for part in candidate["content"]["parts"].as_array().into_iter().flatten() {
            if let Some(text) = part["text"].as_str() {
                if part["thought"] == true {
                    w.thinking(text).await?;
                } else {
                    w.text(text).await?;
                }
            }
            if let Some(call) = part.get("functionCall") {
                let id = format!("toolu_{}", uuid::Uuid::new_v4().simple());
                w.start_tool(&id, call["name"].as_str().unwrap_or_default()).await?;
                let args = if call["args"].is_object() { call["args"].to_string() } else { "{}".into() };
                w.tool_args(&args).await?;
                w.close_block().await?;
                tool_calls += 1;
            }
        }

        if let Some(finish) = candidate["finishReason"].as_str() {
            stop_reason = match finish {
                "MAX_TOKENS" => "max_tokens",
                _ if tool_calls > 0 => "tool_use",
                _ => "end_turn",
            };
        }
        let meta = &data["usageMetadata"];
        if meta.is_object() {
            let prompt = meta["promptTokenCount"].as_u64().unwrap_or(0);
            let cached = meta["cachedContentTokenCount"].as_u64().unwrap_or(0).min(prompt);
            usage.input_tokens = prompt - cached;
            usage.cache_read_input_tokens = cached;
            usage.output_tokens =
                meta["candidatesTokenCount"].as_u64().unwrap_or(0) + meta["thoughtsTokenCount"].as_u64().unwrap_or(0);
        }
    }
    if tool_calls > 0 && stop_reason == "end_turn" {
        stop_reason = "tool_use";
    }
    info!(tool_calls, input = usage.input_tokens, output = usage.output_tokens, stop = stop_reason, "gemini done");
    Ok((stop_reason, usage))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msgs(v: Value) -> Vec<Message> {
        let req: MessagesRequest = serde_json::from_value(json!({ "model": "x", "messages": v })).unwrap();
        req.messages
    }

    #[test]
    fn merges_roles_and_resolves_tool_names() {
        let contents = to_contents(&msgs(json!([
            { "role": "user", "content": "a" },
            { "role": "system", "content": "b" },
            { "role": "assistant", "content": [{ "type": "tool_use", "id": "t1", "name": "Bash", "input": { "command": "ls" } }] },
            { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": "out" }] }
        ])));
        assert_eq!(contents.len(), 3);
        assert_eq!(contents[0]["parts"].as_array().unwrap().len(), 2);
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[1]["parts"][0]["functionCall"]["args"]["command"], "ls");
        assert_eq!(contents[2]["parts"][0]["functionResponse"]["name"], "Bash");
    }

    #[test]
    fn schema_sanitizer_keeps_property_names() {
        let schema = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "properties": { "type": { "type": "string", "exclusiveMinimum": 0 }, "format": { "type": "string" } },
            "required": ["type"],
            "additionalProperties": false
        });
        assert_eq!(
            sanitize_schema(&schema, false),
            json!({
                "type": "object",
                "properties": { "type": { "type": "string" }, "format": { "type": "string" } },
                "required": ["type"],
                "additionalProperties": false
            })
        );
    }

    #[test]
    fn thinking_levels() {
        assert_eq!(thinking_config("gemini-3.1-pro-preview", None)["thinkingLevel"], "HIGH");
        assert_eq!(thinking_config("gemini-3-flash-preview", Some(Reasoning::Medium))["thinkingLevel"], "MEDIUM");
        assert_eq!(thinking_config("gemini-3-pro-preview", Some(Reasoning::Medium))["thinkingLevel"], "HIGH");
        assert_eq!(thinking_config("gemini-3-pro-preview", Some(Reasoning::None))["thinkingLevel"], "LOW");
        assert_eq!(thinking_config("gemini-2.5-pro", Some(Reasoning::Low))["thinkingBudget"], 1024);
        assert_eq!(thinking_config("gemini-2.5-flash", None)["thinkingBudget"], -1);
        assert!(!is_gemini3("gemini-30"));
    }

    #[test]
    fn code_assist_inlines_system_prompt() {
        let req: MessagesRequest = serde_json::from_value(json!({
            "model": "gemini", "system": "be brief", "messages": [{ "role": "user", "content": "hi" }]
        }))
        .unwrap();
        let target = Target::new(crate::routing::Provider::GeminiOauth, "gemini-3.1-pro-preview", None);
        let body = build_request(&req, &target, Some("proj"));
        assert_eq!(body["project"], "proj");
        assert!(body["request"]["contents"][0]["parts"][0]["text"].as_str().unwrap().contains("be brief"));
        let body = build_request(&req, &target, None);
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "be brief");
    }
}
