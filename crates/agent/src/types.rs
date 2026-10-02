use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tool::ToolResult;

/// Content a user or a tool can send to the model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    Image { data: String, mime_type: String },
}

impl ContentPart {
    pub fn text(text: impl Into<String>) -> Self {
        ContentPart::Text { text: text.into() }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            ContentPart::Text { text } => Some(text),
            ContentPart::Image { .. } => None,
        }
    }
}

/// The largest image, decoded, that goes to a model inline.
pub const MAX_INLINE_IMAGE_BYTES: usize = 5 * 1024 * 1024;

/// The type of an image a model can take inline, or why it cannot: base64 data of a PNG, JPEG,
/// GIF, or WebP image of at most 5 MB. The type is what the image's first bytes say, not how it
/// was labeled. Providers refuse a whole request over any other image, and a refused image left
/// in a chat fails every later turn.
pub fn inline_image_type(data: &str) -> Result<&'static str, String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).ok().filter(|bytes| !bytes.is_empty()).ok_or("its data is not base64")?;
    if bytes.len() > MAX_INLINE_IMAGE_BYTES {
        return Err(format!("it is over {} MB", MAX_INLINE_IMAGE_BYTES / (1024 * 1024)));
    }
    match bytes.as_slice() {
        [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, ..] => Ok("image/png"),
        // JPEG-LS starts as a JPEG does, and no provider takes it.
        [0xFF, 0xD8, 0xFF, marker, ..] if !matches!(marker, 0xF7 | 0xF8) => Ok("image/jpeg"),
        [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => Ok("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Ok("image/webp"),
        _ => Err("it is not a PNG, JPEG, GIF, or WebP image".into()),
    }
}

/// A tool call the assistant asked for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// One block of an assistant message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantPart {
    Text { text: String },
    /// `signature` is the provider's seal on the thinking, sent back when the turn continues
    /// with a tool result so the provider accepts the message as its own.
    Thinking {
        thinking: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    ToolCall(ToolCall),
    /// A block the provider produced on its side and wants back verbatim when the turn
    /// continues: a server tool call, its result, redacted thinking. Never text of the reply;
    /// other providers leave it out.
    ServerBlock { block: Value },
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    #[default]
    Stop,
    Length,
    ToolUse,
    Error,
    Aborted,
}

/// The thinking level and what a response used and cost live beside the model catalog, in
/// `lorca-models`.
pub use lorca_models::{Cost, ThinkingLevel, Usage};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserMessage {
    pub content: Vec<ContentPart>,
    pub timestamp: u64,
}

impl UserMessage {
    pub fn text(text: impl Into<String>) -> Self {
        UserMessage { content: vec![ContentPart::text(text)], timestamp: crate::now_ms() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistantMessage {
    pub content: Vec<AssistantPart>,
    pub stop_reason: StopReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default)]
    pub usage: Usage,
    pub provider: String,
    pub model: String,
    pub timestamp: u64,
}

impl AssistantMessage {
    pub fn empty(provider: &str, model: &str) -> Self {
        AssistantMessage {
            content: Vec::new(),
            stop_reason: StopReason::Stop,
            error_message: None,
            usage: Usage::default(),
            provider: provider.to_string(),
            model: model.to_string(),
            timestamp: crate::now_ms(),
        }
    }

    pub fn text(&self) -> String {
        let mut out = String::new();
        for part in &self.content {
            if let AssistantPart::Text { text } = part {
                out.push_str(text);
            }
        }
        out
    }

    pub fn tool_calls(&self) -> Vec<&ToolCall> {
        self.content
            .iter()
            .filter_map(|part| match part {
                AssistantPart::ToolCall(call) => Some(call),
                _ => None,
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: Vec<ContentPart>,
    #[serde(default)]
    pub details: Value,
    pub is_error: bool,
    pub timestamp: u64,
}

impl ToolResultMessage {
    pub fn text(&self) -> String {
        self.content.iter().filter_map(ContentPart::as_text).collect::<Vec<_>>().join("\n")
    }
}

/// What the model understands. The loop converts [`AgentMessage`]s into these right before a call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum LlmMessage {
    User(UserMessage),
    Assistant(AssistantMessage),
    ToolResult(ToolResultMessage),
}

/// The transcript type. LLM messages plus app-specific `Custom` entries that `convert_to_llm`
/// filters out or rewrites.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum AgentMessage {
    User(UserMessage),
    Assistant(AssistantMessage),
    ToolResult(ToolResultMessage),
    Custom { kind: String, data: Value, timestamp: u64 },
}

impl AgentMessage {
    pub fn user(text: impl Into<String>) -> Self {
        AgentMessage::User(UserMessage::text(text))
    }

    pub fn timestamp(&self) -> u64 {
        match self {
            AgentMessage::User(m) => m.timestamp,
            AgentMessage::Assistant(m) => m.timestamp,
            AgentMessage::ToolResult(m) => m.timestamp,
            AgentMessage::Custom { timestamp, .. } => *timestamp,
        }
    }

    pub fn as_llm(&self) -> Option<LlmMessage> {
        match self {
            AgentMessage::User(m) => Some(LlmMessage::User(m.clone())),
            AgentMessage::Assistant(m) => Some(LlmMessage::Assistant(m.clone())),
            AgentMessage::ToolResult(m) => Some(LlmMessage::ToolResult(m.clone())),
            AgentMessage::Custom { .. } => None,
        }
    }

    pub fn is_assistant(&self) -> bool {
        matches!(self, AgentMessage::Assistant(_))
    }
}

impl From<LlmMessage> for AgentMessage {
    fn from(message: LlmMessage) -> Self {
        match message {
            LlmMessage::User(m) => AgentMessage::User(m),
            LlmMessage::Assistant(m) => AgentMessage::Assistant(m),
            LlmMessage::ToolResult(m) => AgentMessage::ToolResult(m),
        }
    }
}

/// Lifecycle events emitted by the loop. Same shape and order as pi-agent-core.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    AgentStart,
    AgentEnd {
        messages: Vec<AgentMessage>,
    },
    TurnStart,
    TurnEnd {
        message: AgentMessage,
        tool_results: Vec<ToolResultMessage>,
    },
    /// A model call failed in a way worth another try and nothing had streamed yet: the loop
    /// waits `delay_ms`, then asks again. The failed attempt leaves no message.
    Retry {
        attempt: u32,
        max_attempts: u32,
        delay_ms: u64,
        error: String,
    },
    MessageStart {
        message: AgentMessage,
    },
    /// Assistant messages only: a partial message plus the delta that produced it.
    MessageUpdate {
        message: AgentMessage,
        assistant_message_event: crate::provider::AssistantEvent,
    },
    MessageEnd {
        message: AgentMessage,
    },
    ToolExecutionStart {
        tool_call_id: String,
        tool_name: String,
        args: Value,
    },
    ToolExecutionUpdate {
        tool_call_id: String,
        tool_name: String,
        args: Value,
        partial_result: ToolResult,
    },
    ToolExecutionEnd {
        tool_call_id: String,
        tool_name: String,
        result: ToolResult,
        is_error: bool,
    },
}
