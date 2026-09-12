//! Durable storage for the reply a web-channel turn is about to announce.
//!
//! An interactive turn's answer used to reach disk only if the viewing client
//! persisted the `chat_done` it received. That made the renderer the single
//! writer of a reply the core had already produced: one failed
//! `threads_message_append`, one socket reconnect that moved the client to a new
//! `client_id`, or one webview reload during a long turn, and the answer was
//! gone from the thread while the agent's own session history still held it
//! (#6034).
//!
//! Core-initiated turns never had that exposure — `task_session::append_final`
//! writes their closing row *before* the run announces it, and the client's
//! append collapses onto that row because both derive the same deterministic id
//! (#5933). This module gives interactive turns the same guarantee, so a reply
//! survives whatever the renderer was doing at the moment it was delivered.
//!
//! The id is [`run_reply_message_id`] over the turn's `request_id`, which is
//! what makes the second write a no-op rather than a duplicate bubble: the
//! conversation store is idempotent for exactly this id shape.

use std::path::Path;

use serde_json::json;

use crate::openhuman::memory::agent::memory_loader::MemoryCitation;
use crate::openhuman::memory::conversations::{self, run_reply_message_id, ConversationMessage};

/// Metadata scope stamped on a reply persisted by the web-channel delivery path.
///
/// Distinguishes it from `autonomous_task_result` (the same shape written by
/// `task_session::append_final`) when reading a thread back.
const REPLY_SCOPE: &str = "web_chat_reply";

/// Persist a delivered reply under the id the announcing client will reuse.
///
/// Returns `Ok(false)` when there was nothing to store (an empty or
/// whitespace-only response — the same guard `task_session::append_final`
/// applies), `Ok(true)` when the row is on disk, and `Err` when the store
/// refused the write (most commonly a thread that does not exist yet).
///
/// Callers must treat an `Err` as non-fatal and announce the reply anyway: the
/// client's own append is still a working fallback, and losing the delivery on
/// top of losing the row would turn a storage problem into a visibly dead turn.
///
/// **This row's metadata is what the reader ends up with, so it must carry
/// everything the client would have written.** Because the store is idempotent
/// for this id, the client's later append returns *this* row rather than its
/// own, and whatever is missing here is missing from the rendered message —
/// which is how citations vanished in review. `citations` therefore mirrors the
/// shape `chatDoneExtraMetadata` builds, and any field added to that helper
/// belongs here too.
pub(crate) fn persist_delivered_reply(
    workspace_dir: &Path,
    thread_id: &str,
    request_id: &str,
    full_response: &str,
    citations: &[MemoryCitation],
) -> Result<bool, String> {
    let content = full_response.trim();
    if content.is_empty() {
        return Ok(false);
    }
    let mut extra_metadata = json!({
        "scope": REPLY_SCOPE,
        "requestId": request_id,
    });
    if !citations.is_empty() {
        // Same key and payload the client stamps, so a row read back from disk
        // renders identical chips to one the client had appended itself.
        extra_metadata["citations"] = json!(citations);
    }
    conversations::append_message(
        workspace_dir.to_path_buf(),
        thread_id,
        ConversationMessage {
            id: run_reply_message_id(request_id),
            content: content.to_string(),
            message_type: "text".to_string(),
            extra_metadata,
            sender: "agent".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )?;
    Ok(true)
}

#[cfg(test)]
#[path = "reply_persistence_tests.rs"]
mod tests;
