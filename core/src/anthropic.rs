//! Anthropic Messages API: the request subset the translating providers need, the
//! streaming events they emit, and the fold of those events into a non-streaming reply.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::error::ProxyError;

// ── Request ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct MessagesRequest {
    pub messages: Vec<Message>,
    #[serde(default)]
    pub system: Option<System>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub tools: Vec<Tool>,
    #[serde(default)]
    pub tool_choice: Option<ToolChoice>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum System {
    Text(String),
    Blocks(Vec<SystemBlock>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct SystemBlock {
    #[serde(default)]
    pub text: String,
}

impl System {
    pub fn text(&self) -> String {
        match self {
            System::Text(text) => text.clone(),
            System::Blocks(blocks) => blocks
                .iter()
                .map(|b| b.text.as_str())
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: Content,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Blocks(Vec<Block>),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Text {
        text: String,
    },
    Image {
        source: ImageSource,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        #[serde(default)]
        content: ToolResultContent,
    },
    /// thinking, redacted_thinking, documents and anything newer we do not translate.
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageSource {
    Base64 {
        media_type: String,
        data: String,
    },
    Url {
        url: String,
        #[serde(default)]
        media_type: Option<String>,
    },
    #[serde(other)]
    Other,
}

impl ImageSource {
    pub fn media_type(&self) -> &str {
        match self {
            ImageSource::Base64 { media_type, .. } => media_type,
            ImageSource::Url { media_type: Some(m), .. } => m,
            _ => "unknown",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ToolResultContent {
    Text(String),
    Blocks(Vec<Block>),
}

impl Default for ToolResultContent {
    fn default() -> Self {
        ToolResultContent::Text(String::new())
    }
}

impl ToolResultContent {
    /// Flattens structured tool output into the plain string other APIs expect.
    pub fn to_text(&self) -> String {
        match self {
            ToolResultContent::Text(text) => text.clone(),
            ToolResultContent::Blocks(blocks) => blocks
                .iter()
                .map(|b| match b {
                    Block::Text { text } => text.clone(),
                    Block::Image { source } => format!("[image: {}]", source.media_type()),
                    _ => String::new(),
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Tool {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub input_schema: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolChoice {
    Auto,
    Any,
    None,
    Tool { name: String },
    #[serde(other)]
    Other,
}

const NATIVE_SEARCH_NOTE: &str = "IMPORTANT: You have native server-side web search. Do NOT use the WebSearch or WebFetch tools — they will not work. Instead, use your built-in web search capability when you need to look something up online.";

impl MessagesRequest {
    /// Claude Code's WebSearch/WebFetch run client-side and break through the proxy;
    /// providers with built-in search get told to use that instead.
    pub fn prefer_native_search(&mut self) {
        self.tools.retain(|t| t.name != "WebSearch" && t.name != "WebFetch");
        match &mut self.system {
            Some(System::Text(text)) => {
                text.push_str("\n\n");
                text.push_str(NATIVE_SEARCH_NOTE);
            }
            Some(System::Blocks(blocks)) => blocks.push(SystemBlock { text: NATIVE_SEARCH_NOTE.into() }),
            None => self.system = Some(System::Text(NATIVE_SEARCH_NOTE.into())),
        }
    }

    pub fn system_text(&self) -> String {
        self.system.as_ref().map(System::text).unwrap_or_default()
    }
}

// ── Streaming events ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StartBlock {
    Text { text: String },
    Thinking { thinking: String },
    ToolUse { id: String, name: String, input: Value },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Delta {
    TextDelta { text: String },
    ThinkingDelta { thinking: String },
    InputJsonDelta { partial_json: String },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MessageHead {
    pub id: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    MessageStart { message: MessageHead },
    ContentBlockStart { index: usize, content_block: StartBlock },
    ContentBlockDelta { index: usize, delta: Delta },
    ContentBlockStop { index: usize },
    MessageDelta { stop_reason: String, usage: Usage },
    MessageStop,
}

impl Event {
    pub fn name(&self) -> &'static str {
        match self {
            Event::MessageStart { .. } => "message_start",
            Event::ContentBlockStart { .. } => "content_block_start",
            Event::ContentBlockDelta { .. } => "content_block_delta",
            Event::ContentBlockStop { .. } => "content_block_stop",
            Event::MessageDelta { .. } => "message_delta",
            Event::MessageStop => "message_stop",
        }
    }

    /// Wire JSON. message_start and message_delta carry more fixed fields than the enum holds.
    pub fn to_json(&self) -> Value {
        match self {
            Event::MessageStart { message } => json!({
                "type": "message_start",
                "message": {
                    "id": message.id,
                    "type": "message",
                    "role": "assistant",
                    "model": message.model,
                    "content": [],
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": Usage::default(),
                },
            }),
            Event::MessageDelta { stop_reason, usage } => json!({
                "type": "message_delta",
                "delta": { "stop_reason": stop_reason, "stop_sequence": null },
                "usage": usage,
            }),
            other => serde_json::to_value(other).expect("event serializes"),
        }
    }
}

pub type EventTx = mpsc::Sender<Result<Event, ProxyError>>;

/// How a translated reply ended, for the request log.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub stop_reason: String,
    pub usage: Usage,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum OpenKind {
    Text,
    Thinking,
    Tool,
}

/// Emits a well-formed event sequence: one message_start, at most one open content block,
/// strictly increasing block indices, and a closing message_delta/message_stop.
pub struct MessageWriter {
    tx: EventTx,
    id: String,
    model: String,
    started: bool,
    next_index: usize,
    open: Option<OpenKind>,
}

impl MessageWriter {
    pub fn new(tx: EventTx, model: impl Into<String>) -> Self {
        Self {
            tx,
            id: format!("msg_{}", uuid::Uuid::new_v4().simple()),
            model: model.into(),
            started: false,
            next_index: 0,
            open: None,
        }
    }

    pub fn started(&self) -> bool {
        self.started
    }

    pub fn thinking_open(&self) -> bool {
        self.open == Some(OpenKind::Thinking)
    }

    pub fn tool_open(&self) -> bool {
        self.open == Some(OpenKind::Tool)
    }

    async fn send(&self, event: Event) -> Result<(), ProxyError> {
        self.tx.send(Ok(event)).await.map_err(|_| ProxyError::client_gone())
    }

    pub async fn start(&mut self) -> Result<(), ProxyError> {
        if self.started {
            return Ok(());
        }
        self.started = true;
        let message = MessageHead { id: self.id.clone(), model: self.model.clone() };
        self.send(Event::MessageStart { message }).await
    }

    pub async fn close_block(&mut self) -> Result<(), ProxyError> {
        if self.open.take().is_some() {
            let index = self.next_index;
            self.next_index += 1;
            self.send(Event::ContentBlockStop { index }).await?;
        }
        Ok(())
    }

    async fn open_block(&mut self, kind: OpenKind, block: StartBlock) -> Result<(), ProxyError> {
        self.start().await?;
        self.close_block().await?;
        self.open = Some(kind);
        self.send(Event::ContentBlockStart { index: self.next_index, content_block: block }).await
    }

    async fn delta(&self, delta: Delta) -> Result<(), ProxyError> {
        self.send(Event::ContentBlockDelta { index: self.next_index, delta }).await
    }

    pub async fn text(&mut self, text: &str) -> Result<(), ProxyError> {
        if text.is_empty() {
            return Ok(());
        }
        if self.open != Some(OpenKind::Text) {
            self.open_block(OpenKind::Text, StartBlock::Text { text: String::new() }).await?;
        }
        self.delta(Delta::TextDelta { text: text.into() }).await
    }

    pub async fn thinking(&mut self, thinking: &str) -> Result<(), ProxyError> {
        if thinking.is_empty() {
            return Ok(());
        }
        if self.open != Some(OpenKind::Thinking) {
            self.open_block(OpenKind::Thinking, StartBlock::Thinking { thinking: String::new() }).await?;
        }
        self.delta(Delta::ThinkingDelta { thinking: thinking.into() }).await
    }

    pub async fn start_tool(&mut self, id: &str, name: &str) -> Result<(), ProxyError> {
        let block = StartBlock::ToolUse { id: id.into(), name: name.into(), input: json!({}) };
        self.open_block(OpenKind::Tool, block).await
    }

    /// Appends to the open tool_use block's arguments; ignored when no tool is open.
    pub async fn tool_args(&mut self, partial_json: &str) -> Result<(), ProxyError> {
        if partial_json.is_empty() || !self.tool_open() {
            return Ok(());
        }
        self.delta(Delta::InputJsonDelta { partial_json: partial_json.into() }).await
    }

    pub async fn finish(mut self, stop_reason: &str, usage: Usage) -> Result<Outcome, ProxyError> {
        self.start().await?;
        self.close_block().await?;
        self.send(Event::MessageDelta { stop_reason: stop_reason.into(), usage }).await?;
        self.send(Event::MessageStop).await?;
        Ok(Outcome { stop_reason: stop_reason.into(), usage })
    }
}

// ── Non-streaming aggregation ────────────────────────────────────────

enum AggBlock {
    Text(String),
    Thinking(String),
    ToolUse { id: String, name: String, args: String },
}

impl AggBlock {
    fn into_json(self) -> Value {
        match self {
            AggBlock::Text(text) => json!({ "type": "text", "text": text }),
            AggBlock::Thinking(thinking) => json!({ "type": "thinking", "thinking": thinking, "signature": "" }),
            AggBlock::ToolUse { id, name, args } => {
                let input = if args.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&args).unwrap_or_else(|_| json!({}))
                };
                json!({ "type": "tool_use", "id": id, "name": name, "input": input })
            }
        }
    }
}

/// Folds a stream of events into the body of a non-streaming Messages response.
#[derive(Default)]
pub struct Aggregator {
    id: String,
    model: String,
    blocks: Vec<(usize, AggBlock)>,
    stop_reason: Option<String>,
    usage: Usage,
}

impl Aggregator {
    pub fn push(&mut self, event: Event) {
        match event {
            Event::MessageStart { message } => {
                self.id = message.id;
                self.model = message.model;
            }
            Event::ContentBlockStart { index, content_block } => {
                let block = match content_block {
                    StartBlock::Text { text } => AggBlock::Text(text),
                    StartBlock::Thinking { thinking } => AggBlock::Thinking(thinking),
                    StartBlock::ToolUse { id, name, .. } => AggBlock::ToolUse { id, name, args: String::new() },
                };
                self.blocks.push((index, block));
            }
            Event::ContentBlockDelta { index, delta } => {
                let Some((_, block)) = self.blocks.iter_mut().rev().find(|(i, _)| *i == index) else {
                    return;
                };
                match (block, delta) {
                    (AggBlock::Text(acc), Delta::TextDelta { text }) => acc.push_str(&text),
                    (AggBlock::Thinking(acc), Delta::ThinkingDelta { thinking }) => acc.push_str(&thinking),
                    (AggBlock::ToolUse { args, .. }, Delta::InputJsonDelta { partial_json }) => {
                        args.push_str(&partial_json)
                    }
                    _ => {}
                }
            }
            Event::ContentBlockStop { .. } | Event::MessageStop => {}
            Event::MessageDelta { stop_reason, usage } => {
                self.stop_reason = Some(stop_reason);
                self.usage = usage;
            }
        }
    }

    pub fn finish(self) -> Value {
        let content: Vec<Value> = self.blocks.into_iter().map(|(_, b)| b.into_json()).collect();
        json!({
            "id": self.id,
            "type": "message",
            "role": "assistant",
            "model": self.model,
            "content": content,
            "stop_reason": self.stop_reason.unwrap_or_else(|| "end_turn".into()),
            "stop_sequence": null,
            "usage": self.usage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn collect(f: impl AsyncFnOnce(&mut MessageWriter)) -> Vec<Event> {
        let (tx, mut rx) = mpsc::channel(64);
        let mut w = MessageWriter::new(tx, "m");
        f(&mut w).await;
        w.finish("end_turn", Usage::default()).await.unwrap();
        let mut out = Vec::new();
        while let Ok(Ok(ev)) = rx.try_recv() {
            out.push(ev);
        }
        out
    }

    #[tokio::test]
    async fn writer_closes_previous_block_and_increments_index() {
        let events = collect(async |w| {
            w.text("a").await.unwrap();
            w.thinking("t").await.unwrap();
            w.text("b").await.unwrap();
        })
        .await;
        let starts: Vec<usize> = events
            .iter()
            .filter_map(|e| match e {
                Event::ContentBlockStart { index, .. } => Some(*index),
                _ => None,
            })
            .collect();
        let stops: Vec<usize> = events
            .iter()
            .filter_map(|e| match e {
                Event::ContentBlockStop { index } => Some(*index),
                _ => None,
            })
            .collect();
        assert_eq!(starts, vec![0, 1, 2]);
        assert_eq!(stops, vec![0, 1, 2]);
        assert!(matches!(events[0], Event::MessageStart { .. }));
        assert!(matches!(events.last(), Some(Event::MessageStop)));
    }

    #[tokio::test]
    async fn aggregates_text_thinking_and_tool_use() {
        let events = collect(async |w| {
            w.thinking("plan").await.unwrap();
            w.text("Hello ").await.unwrap();
            w.text("world").await.unwrap();
            w.start_tool("call_1", "Bash").await.unwrap();
            w.tool_args("{\"command\":").await.unwrap();
            w.tool_args("\"ls\"}").await.unwrap();
        })
        .await;
        let mut agg = Aggregator::default();
        for ev in events {
            agg.push(ev);
        }
        let msg = agg.finish();
        assert_eq!(msg["model"], "m");
        assert_eq!(msg["content"][0]["thinking"], "plan");
        assert_eq!(msg["content"][1], json!({ "type": "text", "text": "Hello world" }));
        assert_eq!(
            msg["content"][2],
            json!({ "type": "tool_use", "id": "call_1", "name": "Bash", "input": { "command": "ls" } })
        );
        assert_eq!(msg["stop_reason"], "end_turn");
    }

    #[test]
    fn deserializes_claude_code_shapes() {
        let req: MessagesRequest = serde_json::from_value(json!({
            "model": "codex",
            "system": [{ "type": "text", "text": "sys", "cache_control": { "type": "ephemeral" } }],
            "messages": [
                { "role": "user", "content": "hi" },
                { "role": "assistant", "content": [
                    { "type": "thinking", "thinking": "x", "signature": "s" },
                    { "type": "tool_use", "id": "t1", "name": "Bash", "input": { "command": "ls" } }
                ]},
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "t1", "content": [{ "type": "text", "text": "ok" }] }
                ]}
            ],
            "tools": [{ "type": "web_search_20250305", "name": "web_search" }],
            "tool_choice": { "type": "auto" },
            "thinking": { "type": "enabled", "budget_tokens": 1000 },
            "stream": true
        }))
        .unwrap();
        assert_eq!(req.system_text(), "sys");
        let Content::Blocks(blocks) = &req.messages[1].content else { panic!() };
        assert!(matches!(blocks[0], Block::Other));
        let Content::Blocks(blocks) = &req.messages[2].content else { panic!() };
        let Block::ToolResult { content, .. } = &blocks[0] else { panic!() };
        assert_eq!(content.to_text(), "ok");
    }
}
