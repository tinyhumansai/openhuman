//! JSON-RPC request/response models for the `threads` namespace.
//!
//! Moved verbatim from the v1 `memory::rpc_models` when memory v2 replaced
//! that module: the serde shapes, field names and defaults are unchanged, so
//! nothing on the wire moved. These types describe chat threads and their
//! messages, which are thread persistence (`threads::store`), not memory.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use crate::core::envelope::{ApiEnvelope, ApiError, ApiMeta, EmptyRequest, PaginationMeta};

/// Typed acknowledgement returned when a web chat message starts immediately.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct StartedChatAck {
    pub accepted: bool,
    pub client_id: String,
    pub thread_id: String,
    pub request_id: String,
}

/// Typed acknowledgement returned when a web chat message enters the run queue.
/// Attachment values are sanitized opaque references minted by the core.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct QueuedChatAck {
    pub queued: bool,
    pub queue_mode: String,
    pub client_id: String,
    pub thread_id: String,
    pub request_id: String,
    pub queue_depth: usize,
    pub queued_attachments: Vec<tinychannels_bus::ContentPart>,
}

/// Request to create a new conversation thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateConversationThreadRequest {
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    #[serde(default)]
    pub personality_id: Option<String>,
    /// Working folder the new thread's agent acts in. Absent or empty uses the
    /// global `action_dir`.
    #[serde(default)]
    pub action_dir: Option<String>,
}

/// Request to bind or clear the working folder of a thread that has no
/// messages yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConversationThreadWorkingDirRequest {
    pub thread_id: String,
    /// Absolute folder path; empty clears back to the global `action_dir`.
    pub action_dir: String,
}

/// Summary information for a workspace-backed conversation thread.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationThreadSummary {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<i64>,
    pub is_active: bool,
    pub message_count: usize,
    pub last_message_at: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_thread_id: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personality_id: Option<String>,
    /// Working folder bound to this thread; absent means the global default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_dir: Option<String>,
}

/// A single persisted conversation message.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessageRecord {
    pub id: String,
    pub content: String,
    /// Opaque, persisted media references attached to this message.
    /// Only OpenHuman returns these. Callers submit refs in the chat request,
    /// and the trusted runner revalidates them against this exact user row.
    #[serde(default, skip_serializing_if = "Vec::is_empty", skip_deserializing)]
    pub attachments: Vec<tinychannels_bus::ContentPart>,
    #[serde(rename = "type")]
    pub message_type: String,
    #[serde(default)]
    pub extra_metadata: serde_json::Value,
    pub sender: String,
    pub created_at: String,
}

/// Request to create or update a thread in workspace storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpsertConversationThreadRequest {
    pub id: String,
    pub title: String,
    pub created_at: String,
    #[serde(default)]
    pub parent_thread_id: Option<String>,
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    #[serde(default)]
    pub personality_id: Option<String>,
}

/// Request to update labels for a conversation thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConversationThreadLabelsRequest {
    pub thread_id: String,
    pub labels: Vec<String>,
}

/// Request to set a user-specified title on a conversation thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConversationThreadTitleRequest {
    pub thread_id: String,
    pub title: String,
}

/// Response payload for thread list operations.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationThreadsListResponse {
    pub threads: Vec<ConversationThreadSummary>,
    pub count: usize,
}

/// Request to fetch messages for a specific thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationMessagesRequest {
    pub thread_id: String,
}

/// Response payload for message list operations.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessagesResponse {
    pub messages: Vec<ConversationMessageRecord>,
    pub count: usize,
}

/// Request to append a message to a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppendConversationMessageRequest {
    pub thread_id: String,
    pub message: ConversationMessageRecord,
    /// Typed attachments for a user message, staged with it. Folded into the
    /// message content as upload markers at the RPC edge
    /// (`attachments::parts`), so callers never build markers.
    #[serde(default)]
    pub attachments: Vec<tinychannels_bus::ContentPart>,
}

/// Request to generate or refresh a thread title after the first exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateConversationThreadTitleRequest {
    pub thread_id: String,
    #[serde(default)]
    pub assistant_message: Option<String>,
}

/// Request to patch a persisted message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConversationMessageRequest {
    pub thread_id: String,
    pub message_id: String,
    #[serde(default)]
    pub extra_metadata: Option<serde_json::Value>,
}

/// Request to delete a thread and its message log.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteConversationThreadRequest {
    pub thread_id: String,
    pub deleted_at: String,
}

/// Response payload for single-thread deletion.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeleteConversationThreadResponse {
    pub deleted: bool,
}

/// Response payload for purging all workspace-backed conversations.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PurgeConversationThreadsResponse {
    pub messages_deleted: usize,
    pub agent_threads_deleted: usize,
    pub agent_messages_deleted: usize,
}

#[cfg(test)]
#[path = "rpc_models_tests.rs"]
mod tests;
