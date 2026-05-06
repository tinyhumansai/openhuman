//! Subconscious → conversation surface bridge (#623).
//!
//! Owns:
//! - The stable thread id `system:subconscious` that backs the dedicated
//!   "OpenHuman — Proactive" thread.
//! - `ensure_subconscious_thread` — idempotent locate-or-create so the
//!   thread reliably exists before we post.
//! - `post_reflection` — append a Notify-disposition reflection's body
//!   (with rendered `proposed_action`) into that thread, embedding the
//!   reflection metadata in `extra_metadata` so the frontend can render
//!   the action button.

use std::path::PathBuf;

use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::openhuman::memory::conversations::{
    append_message, ensure_thread, ConversationMessage, ConversationThread,
    CreateConversationThread,
};

use super::reflection::{Disposition, Reflection};

/// Stable id for the system-owned subconscious conversation thread.
/// Treated as a sentinel by the frontend to opt-in distinct rendering
/// (different icon, label, background tint, etc.).
pub const SUBCONSCIOUS_THREAD_ID: &str = "system:subconscious";

/// Title presented in the conversation list. Stable copy.
pub const SUBCONSCIOUS_THREAD_TITLE: &str = "OpenHuman — Proactive";

/// Labels persisted on the thread row. Used by the frontend to filter
/// the system thread out of the regular conversation surface.
pub const SUBCONSCIOUS_THREAD_LABELS: &[&str] = &["system", "subconscious"];

/// Locate-or-create the dedicated subconscious thread. Idempotent —
/// repeated calls produce the same row.
///
/// `now_iso` is the timestamp to record on first creation; ignored if
/// the thread already exists. Caller-supplied so tests can be
/// deterministic.
pub fn ensure_subconscious_thread(
    workspace_dir: PathBuf,
    now_iso: String,
) -> Result<ConversationThread, String> {
    log::debug!(
        "[subconscious::conversation_post] ensure_subconscious_thread workspace={}",
        workspace_dir.display()
    );
    ensure_thread(
        workspace_dir,
        CreateConversationThread {
            id: SUBCONSCIOUS_THREAD_ID.to_string(),
            title: SUBCONSCIOUS_THREAD_TITLE.to_string(),
            created_at: now_iso,
            parent_thread_id: None,
            labels: Some(
                SUBCONSCIOUS_THREAD_LABELS
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect(),
            ),
        },
    )
}

/// Render a reflection's body for posting. If the reflection carries a
/// `proposed_action`, append it as a separate paragraph the frontend
/// can also surface via the action button — the user sees both.
pub fn render_message_body(reflection: &Reflection) -> String {
    let body = reflection.body.trim();
    match &reflection.proposed_action {
        Some(action) if !action.trim().is_empty() => {
            format!("{body}\n\n_Proposed action_: {}", action.trim())
        }
        _ => body.to_string(),
    }
}

/// Build the `extra_metadata` JSON payload that rides on the message.
/// Frontend keys are stable so the renderer can pick them off
/// regardless of message-store schema drift.
pub fn build_extra_metadata(reflection: &Reflection) -> serde_json::Value {
    json!({
        "reflection_id": reflection.id,
        "kind": reflection.kind.as_str(),
        "disposition": reflection.disposition.as_str(),
        "proposed_action": reflection.proposed_action,
        "source_refs": reflection.source_refs,
    })
}

/// Append a reflection as an assistant message to the subconscious
/// thread. Caller is responsible for ensuring the thread exists
/// (typically: call `ensure_subconscious_thread` once before the loop).
///
/// Returns the persisted message so the caller can stamp `surfaced_at`
/// on the reflection row using the message's timestamp if desired.
pub fn post_reflection(
    workspace_dir: PathBuf,
    reflection: &Reflection,
) -> Result<ConversationMessage, String> {
    if reflection.disposition != Disposition::Notify {
        return Err(format!(
            "post_reflection: refusing non-Notify disposition (id={})",
            reflection.id
        ));
    }
    let now_iso = Utc::now().to_rfc3339();
    let message = ConversationMessage {
        id: Uuid::new_v4().to_string(),
        content: render_message_body(reflection),
        message_type: "text".to_string(),
        extra_metadata: build_extra_metadata(reflection),
        sender: "assistant".to_string(),
        created_at: now_iso,
    };
    log::debug!(
        "[subconscious::conversation_post] posting reflection id={} thread={}",
        reflection.id,
        SUBCONSCIOUS_THREAD_ID
    );
    append_message(workspace_dir, SUBCONSCIOUS_THREAD_ID, message)
}

#[cfg(test)]
#[path = "conversation_post_tests.rs"]
mod tests;
