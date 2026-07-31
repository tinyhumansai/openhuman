//! Durable OpenHuman transcript records.
//!
//! Runtime model calls use [`tinyagents::harness::message::Message`]. These
//! compact records preserve the stable JSONL/thread storage contract used by
//! existing installations and carry OpenHuman-only message metadata.

use serde::{Deserialize, Serialize};

use crate::openhuman::inference::provider::ToolCall;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    #[serde(default, skip_serializing)]
    pub id: Option<String>,
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing)]
    pub extra_metadata: Option<serde_json::Value>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            id: None,
            role: "system".into(),
            content: content.into(),
            extra_metadata: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            id: None,
            role: "user".into(),
            content: content.into(),
            extra_metadata: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            id: None,
            role: "assistant".into(),
            content: content.into(),
            extra_metadata: None,
        }
    }

    pub fn tool(content: impl Into<String>) -> Self {
        Self {
            id: None,
            role: "tool".into(),
            content: content.into(),
            extra_metadata: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub content: String,
    /// The result asked to reach the model unchanged — see
    /// [`ToolResult::trusted_verbatim`](crate::openhuman::tools::ToolResult).
    ///
    /// Carried on the persisted shape, not just the live one, because the
    /// serializers that batch and wrap a round run again every turn from
    /// durable history. Without it here a contract delivered verbatim in the
    /// turn that produced it comes back framed in the next one, and the model
    /// reads a schema it can no longer copy from.
    ///
    /// `#[serde(default)]` so transcripts written before the field existed load
    /// as unmarked, which is what they were.
    #[serde(default)]
    pub trusted_verbatim: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ConversationMessage {
    Chat(ChatMessage),
    AssistantToolCalls {
        text: Option<String>,
        tool_calls: Vec<ToolCall>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        extra_metadata: Option<serde_json::Value>,
    },
    ToolResults(Vec<ToolResultMessage>),
}
