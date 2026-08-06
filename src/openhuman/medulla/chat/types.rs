//! Data types for the `chat_store` module.
#[allow(unused_imports)]
use super::*;
/// A conversation turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}
/// An in-memory tree node (messages included on save; loaded from disk on load).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatNode {
    pub session_id: String,
    pub name: String,
    pub fork_point: Option<i64>,
    pub messages: Vec<ChatMessage>,
    pub children: Vec<ChatNode>,
}
/// One row for the `/resume` picker — from `tree.json` alone (no md reads).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainChatSummary {
    pub session_id: String,
    pub name: String,
    pub turns: usize,
    pub thread_count: usize,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct StoredNode {
    #[serde(rename = "sessionId")]
    pub(super) session_id: String,
    pub(super) name: String,
    #[serde(rename = "forkPoint", skip_serializing_if = "Option::is_none", default)]
    pub(super) fork_point: Option<i64>,
    pub(super) turns: usize,
    pub(super) md: String,
    pub(super) children: Vec<StoredNode>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct StoredTree {
    pub(super) version: u8,
    #[serde(rename = "updatedAt")]
    pub(super) updated_at: String,
    pub(super) root: StoredNode,
}
