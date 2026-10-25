//! Anthropic Messages → OpenAI-compatible Chat Completions API.

use std::collections::BTreeMap;
use std::sync::Arc;

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde_json::{Map, Value, json};
use tracing::info;

use crate::anthropic::{
    Block, Content, ImageSource, Message, MessageWriter, MessagesRequest, Outcome, ToolChoice,
    Usage,
};
use crate::error::ProxyError;
use crate::routing::Target;
use crate::server::AppState;

fn endpoint(base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.into()
    } else {
        format!("{base}/chat/completions")
    }
}

fn image_url(source: &ImageSource) -> String {
    match source {
        ImageSource::Base64 { media_type, data } => format!("data:{media_type};base64,{data}"),
        ImageSource::Url { url, .. } => url.clone(),
        ImageSource::Other => String::new(),
    }
}

fn content_parts(blocks: &[Block]) -> Vec<Value> {
    blocks
        .iter()
        .filter_map(|block| match block {
            Block::Text { text } => Some(json!({ "type": "text", "text": text })),
            Block::Image { source } => {
                Some(json!({ "type": "image_url", "image_url": { "url": image_url(source) } }))
            }
            _ => None,
        })
        .collect()
}

fn text_content(content: &Content) -> Value {
    match content {
        Content::Text(text) => json!(text),
        Content::Blocks(blocks) => {
            let parts = content_parts(blocks);
            if parts.len() == 1 && parts[0]["type"] == "text" {
                parts[0]["text"].clone()
            } else {
                json!(parts)
            }
        }
    }
}

pub fn to_messages(req: &MessagesRequest) -> Vec<Value> {
    let mut out = Vec::new();
    let system = req.system_text();
    if !system.is_empty() {
        out.push(json!({ "role": "system", "content": system }));
    }
    for message in &req.messages {
        match &message.content {
            Content::Text(_) => {
                out.push(json!({ "role": message.role, "content": text_content(&message.content) }))
            }
            Content::Blocks(blocks) => append_blocks(&mut out, message, blocks),
        }
    }
    out
}

fn append_blocks(out: &mut Vec<Value>, message: &Message, blocks: &[Block]) {
    if message.role == "assistant" {
        let parts = content_parts(blocks);
        let calls: Vec<Value> = blocks
            .iter()
            .filter_map(|block| match block {
                Block::ToolUse { id, name, input } => Some(json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": if input.is_string() { input.as_str().unwrap_or_default().to_string() } else { input.to_string() },
                    }
                })),
                _ => None,
            })
            .collect();
        if !parts.is_empty() || !calls.is_empty() {
            let mut msg = Map::from_iter([("role".into(), json!("assistant"))]);
            msg.insert(
                "content".into(),
                if parts.is_empty() {
                    Value::Null
                } else if parts.len() == 1 {
                    parts[0]["text"].clone()
                } else {
                    json!(parts)
                },
            );
            if !calls.is_empty() {
                msg.insert("tool_calls".into(), json!(calls));
            }
            out.push(Value::Object(msg));
        }
    } else {
        // Chat Completions requires tool results immediately after the assistant tool calls.
        for block in blocks {
            if let Block::ToolResult {
                tool_use_id,
                content,
            } = block
            {
                out.push(json!({ "role": "tool", "tool_call_id": tool_use_id, "content": content.to_text() }));
            }
        }
        let parts = content_parts(blocks);
        if !parts.is_empty() {
            out.push(json!({
                "role": if message.role == "user" { "user" } else { "developer" },
                "content": if parts.len() == 1 && parts[0]["type"] == "text" { parts[0]["text"].clone() } else { json!(parts) },
            }));
        }
    }
}

fn tools(req: &MessagesRequest) -> Vec<Value> {
    req.tools
        .iter()
        .map(|tool| {
            let mut function = Map::from_iter([
                ("name".into(), json!(tool.name)),
                (
                    "parameters".into(),
                    tool.input_schema
                        .clone()
                        .unwrap_or_else(|| json!({ "type": "object" })),
                ),
            ]);
            if let Some(description) = &tool.description {
                function.insert("description".into(), json!(description));
            }
            json!({ "type": "function", "function": function })
        })
        .collect()
}

fn tool_choice(choice: Option<&ToolChoice>) -> Value {
    match choice {
        Some(ToolChoice::Any) => json!("required"),
        Some(ToolChoice::None) => json!("none"),
        Some(ToolChoice::Tool { name }) => {
            json!({ "type": "function", "function": { "name": name } })
        }
        _ => json!("auto"),
    }
}

pub async fn run(
    state: Arc<AppState>,
    req: MessagesRequest,
    target: Target,
    base_url: String,
    api_key: Option<String>,
    w: MessageWriter,
) -> Result<Outcome, ProxyError> {
    let url = endpoint(&base_url);
    let mut body = json!({
        "model": target.model,
        "messages": to_messages(&req),
        "stream": true,
    });
    if let Some(max_tokens) = req.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if let Some(temperature) = req.temperature {
        body["temperature"] = json!(temperature);
    }
    if !req.tools.is_empty() {
        body["tools"] = json!(tools(&req));
        body["tool_choice"] = tool_choice(req.tool_choice.as_ref());
    }
    if let Some(reasoning) = target.reasoning {
        body["reasoning_effort"] = json!(reasoning.as_str());
    }

    info!(model = %target.model, url, messages = req.messages.len(), tools = req.tools.len(), "openai-compatible request");
    let mut request = state
        .http
        .post(&url)
        .header("accept", "text/event-stream")
        .json(&body);
    if let Some(key) = api_key {
        request = request.bearer_auth(key);
    }
    let resp = request.send().await?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        return Err(ProxyError::upstream("OpenAI-compatible API", status, &text));
    }

    let mut pump = Pump::new(w);
    let mut events = resp.bytes_stream().eventsource();
    while let Some(event) = events.next().await {
        let event = event.map_err(|e| {
            ProxyError::new(
                axum::http::StatusCode::BAD_GATEWAY,
                format!("OpenAI-compatible stream: {e}"),
            )
        })?;
        if event.data.is_empty() || event.data == "[DONE]" {
            continue;
        }
        let value: Value = serde_json::from_str(&event.data).map_err(|e| {
            ProxyError::new(
                axum::http::StatusCode::BAD_GATEWAY,
                format!("invalid OpenAI-compatible event: {e}"),
            )
        })?;
        pump.handle(&value).await?;
    }
    pump.finish().await
}

#[derive(Default)]
struct ToolCall {
    id: String,
    name: String,
    arguments: String,
}

struct Pump {
    w: MessageWriter,
    tools: BTreeMap<u64, ToolCall>,
    stop_reason: &'static str,
    usage: Usage,
}

impl Pump {
    fn new(w: MessageWriter) -> Self {
        Self {
            w,
            tools: BTreeMap::new(),
            stop_reason: "end_turn",
            usage: Usage::default(),
        }
    }

    async fn handle(&mut self, chunk: &Value) -> Result<(), ProxyError> {
        self.w.start().await?;
        if chunk["error"].is_object() {
            let message = chunk["error"]["message"]
                .as_str()
                .unwrap_or("OpenAI-compatible stream error");
            return Err(ProxyError::new(
                axum::http::StatusCode::BAD_GATEWAY,
                message,
            ));
        }
        let choice = &chunk["choices"][0];
        let delta = &choice["delta"];
        if let Some(thinking) = delta["reasoning_content"]
            .as_str()
            .or(delta["reasoning"].as_str())
        {
            self.w.thinking(thinking).await?;
        }
        match &delta["content"] {
            Value::String(text) => self.w.text(text).await?,
            Value::Array(parts) => {
                for part in parts {
                    if let Some(text) = part["text"].as_str() {
                        self.w.text(text).await?;
                    }
                }
            }
            _ => {}
        }
        if let Some(calls) = delta["tool_calls"].as_array() {
            for call in calls {
                let index = call["index"].as_u64().unwrap_or(0);
                let buffered = self.tools.entry(index).or_default();
                if buffered.id.is_empty()
                    && let Some(id) = call["id"].as_str()
                {
                    buffered.id = id.into();
                }
                if buffered.name.is_empty()
                    && let Some(name) = call["function"]["name"].as_str()
                {
                    buffered.name = name.into();
                }
                if let Some(arguments) = call["function"]["arguments"].as_str() {
                    buffered.arguments.push_str(arguments);
                }
            }
        }
        if let Some(reason) = choice["finish_reason"].as_str() {
            self.stop_reason = match reason {
                "tool_calls" | "function_call" => "tool_use",
                "length" => "max_tokens",
                _ => "end_turn",
            };
        }
        let usage = &chunk["usage"];
        if usage.is_object() {
            let total = usage["prompt_tokens"].as_u64().unwrap_or(0);
            let cached = usage["prompt_tokens_details"]["cached_tokens"]
                .as_u64()
                .unwrap_or(0)
                .min(total);
            self.usage.input_tokens = total - cached;
            self.usage.cache_read_input_tokens = cached;
            self.usage.output_tokens = usage["completion_tokens"].as_u64().unwrap_or(0);
        }
        Ok(())
    }

    async fn finish(mut self) -> Result<Outcome, ProxyError> {
        for (_, tool) in self.tools {
            let id = if tool.id.is_empty() {
                format!("call_{}", uuid::Uuid::new_v4().simple())
            } else {
                tool.id
            };
            self.w.start_tool(&id, &tool.name).await?;
            self.w
                .tool_args(if tool.arguments.is_empty() {
                    "{}"
                } else {
                    &tool.arguments
                })
                .await?;
            self.w.close_block().await?;
            self.stop_reason = "tool_use";
        }
        self.w.finish(self.stop_reason, self.usage).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anthropic::{Event, System, ToolResultContent};
    use tokio::sync::mpsc;

    #[test]
    fn normalizes_endpoint() {
        assert_eq!(
            endpoint("http://localhost:11434/v1"),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(
            endpoint("https://example.test/v1/chat/completions/"),
            "https://example.test/v1/chat/completions"
        );
    }

    #[test]
    fn translates_system_and_tool_calls() {
        let req = MessagesRequest {
            system: Some(System::Text("system".into())),
            max_tokens: None,
            temperature: None,
            tools: Vec::new(),
            tool_choice: None,
            messages: vec![Message {
                role: "assistant".into(),
                content: Content::Blocks(vec![Block::ToolUse {
                    id: "call_1".into(),
                    name: "shell".into(),
                    input: json!({ "command": "pwd" }),
                }]),
            }],
        };
        let messages = to_messages(&req);
        assert_eq!(
            messages[0],
            json!({ "role": "system", "content": "system" })
        );
        assert_eq!(messages[1]["tool_calls"][0]["function"]["name"], "shell");
    }

    #[test]
    fn tool_results_precede_the_next_user_text() {
        let req = MessagesRequest {
            system: None,
            max_tokens: None,
            temperature: None,
            tools: Vec::new(),
            tool_choice: None,
            messages: vec![Message {
                role: "user".into(),
                content: Content::Blocks(vec![
                    Block::ToolResult {
                        tool_use_id: "call_1".into(),
                        content: ToolResultContent::Text("ok".into()),
                    },
                    Block::Text {
                        text: "continue".into(),
                    },
                ]),
            }],
        };
        let messages = to_messages(&req);
        assert_eq!(messages[0]["role"], "tool");
        assert_eq!(
            messages[1],
            json!({ "role": "user", "content": "continue" })
        );
    }

    #[tokio::test]
    async fn streams_reasoning_text_and_buffered_tool_calls() {
        let (tx, mut rx) = mpsc::channel(64);
        let mut pump = Pump::new(MessageWriter::new(tx, "model"));
        pump.handle(&json!({ "choices": [{ "delta": { "reasoning_content": "plan" } }] }))
            .await
            .unwrap();
        pump.handle(&json!({ "choices": [{ "delta": { "content": "answer" } }] }))
            .await
            .unwrap();
        pump.handle(&json!({ "choices": [{ "delta": { "tool_calls": [
            { "index": 0, "id": "call_1", "function": { "name": "shell", "arguments": "{\"command\":" } },
            { "index": 1, "id": "call_2", "function": { "name": "read", "arguments": "{\"path\":" } }
        ] } }] }))
            .await
            .unwrap();
        pump.handle(&json!({ "choices": [{ "delta": { "tool_calls": [
            { "index": 1, "id": "call_2", "function": { "name": "read", "arguments": "\"a\"}" } },
            { "index": 0, "id": "call_1", "function": { "name": "shell", "arguments": "\"pwd\"}" } }
        ] }, "finish_reason": "tool_calls" }], "usage": { "prompt_tokens": 10, "completion_tokens": 4 } }))
            .await
            .unwrap();
        let outcome = pump.finish().await.unwrap();
        assert_eq!(outcome.stop_reason, "tool_use");
        assert_eq!(outcome.usage.input_tokens, 10);

        let mut events = Vec::new();
        while let Some(Ok(event)) = rx.recv().await {
            let done = matches!(event, Event::MessageStop);
            events.push(event);
            if done {
                break;
            }
        }
        let output = serde_json::to_value(events).unwrap().to_string();
        assert!(output.contains("plan"));
        assert!(output.contains("answer"));
        assert!(output.contains("call_1"));
        assert!(output.contains("call_2"));
        assert!(output.contains("shell"));
        assert!(output.contains("read"));
        assert!(!output.contains("call_1call_1"));
        assert!(!output.contains("shellshell"));
    }
}
