//! `threads.edit_message` / `threads.regenerate`: cancel the thread's
//! in-flight turn, fork the session transcript at a cut point (the sealed
//! generation it forks from is never touched — same guarantee a compaction
//! gives), trim the conversation-store message log to match, drop the
//! turn-state snapshots for every turn the fork drops, then restart the turn.
//!
//! ## Mapping a UI message id to a transcript cut point
//!
//! The frontend only has ids from `threads.messages_list`
//! (`ConversationMessageRecord.id`) — a different id space from the
//! model-facing transcript, which keys a turn's rows by
//! `TranscriptMessage::request_id`. The one place these two id spaces
//! provably correlate is an **assistant reply**: its store id is minted
//! deterministically as `agent:<request_id>`
//! ([`crate::threads::store::run_reply_message_id`], written by
//! `web_chat::reply_persistence` before the `chat_done` that announces it),
//! so [`crate::threads::store::reply_run_id`] recovers the exact turn
//! id the transcript recorded on every row of that turn.
//!
//! - `regenerate { message_id: Some(id) }` — `id` must be that deterministic
//!   reply id. Its `request_id` is the turn to redo: the transcript is cut
//!   right after that turn's first row (the user prompt — kept, so it can be
//!   resent), dropping the stored answer and everything after; the message
//!   log is truncated from that same reply's store id onward.
//! - `regenerate { message_id: None }` — redo the thread's last turn
//!   ([`tinyagents_session::transcript::TruncateCut::LastAssistantTurn`]),
//!   no id correlation needed; the retained prefix's last row is that turn's
//!   user prompt.
//! - `edit_message { message_id }` — `message_id` names the **user** message
//!   being edited, which carries no such correlation (the frontend mints it
//!   optimistically, before the server has picked a `request_id`). Instead,
//!   this resolves through the *next* deterministic reply id after it in the
//!   store's own message order, recovers that turn's `request_id`, and cuts
//!   the transcript right before that turn's first row (excluding it — the
//!   edit replaces it, so nothing about the old prompt is kept). A user
//!   message with no reply yet (editing the newest, still-unanswered
//!   message) has nothing on the model side to cut; only the message-log
//!   tail is truncated in that case, and the edit still lands as a fresh
//!   turn.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use tinyagents_session::transcript::{
    FileTranscriptLocator, SessionRef, SessionTranscript, TranscriptLocator, TranscriptMessage,
    TranscriptMeta, TruncateCut,
};

use crate::core::Outcome;
use crate::threads::store::{self as conversations, reply_run_id, run_reply_message_id};
use crate::threads::ThreadsError;

use super::support::workspace_dir;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditMessageRequest {
    pub thread_id: String,
    pub message_id: String,
    pub content: String,
    #[serde(default)]
    pub client_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegenerateRequest {
    pub thread_id: String,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EditOrRegenerateResponse {
    pub request_id: String,
}

/// Edit a past user message: cancel the in-flight turn (if any), fork the
/// session transcript and message log to drop that message and everything
/// after it, then restart the turn with `content` in its place.
pub async fn edit_message(request: EditMessageRequest) -> Result<Outcome<Value>, ThreadsError> {
    let client_id = request.client_id.unwrap_or_else(|| "system".to_string());
    let thread_id = request.thread_id;
    let dir = workspace_dir().await.map_err(ThreadsError::Message)?;

    crate::web_chat::cancel_chat(&client_id, &thread_id)
        .await
        .map_err(ThreadsError::Message)?;

    // Correlate through the next assistant reply, if any — see the module
    // doc's mapping section. `None` means the edited message has no reply
    // yet, so there is nothing to cut on the model side.
    let cut_request_id = next_reply_request_id_after(&dir, &thread_id, &request.message_id)
        .await
        .map_err(ThreadsError::Message)?;

    if let Some(cut_request_id) = &cut_request_id {
        let dir_for_blocking = dir.clone();
        let thread_id_owned = thread_id.clone();
        let cut_request_id_owned = cut_request_id.clone();
        tokio::task::spawn_blocking(move || {
            truncate_transcript_before_turn(
                &dir_for_blocking,
                &thread_id_owned,
                &cut_request_id_owned,
            )
        })
        .await
        .map_err(|e| ThreadsError::Message(format!("truncate transcript task: {e}")))?
        .map_err(ThreadsError::Message)?;
        clear_dropped_turn_states(&dir, &thread_id, cut_request_id).await;
    }

    // Truncate the message log at the edited message itself (inclusive) —
    // it and everything after it is replaced by the fresh turn below.
    super::delete_after(&thread_id, &request.message_id).await?;

    crate::web_chat::invalidate_thread_sessions(&thread_id).await;

    let new_request_id = crate::web_chat::start_chat(
        &client_id,
        &thread_id,
        &request.content,
        None,
        None,
        None,
        None,
        crate::web_chat::ChatRequestMetadata::default(),
    )
    .await
    .map_err(|e| ThreadsError::Message(e.to_string()))?;

    Ok(Outcome::single_log(
        json!(EditOrRegenerateResponse {
            request_id: new_request_id,
        }),
        "message edited, turn restarted",
    ))
}

/// Regenerate a past assistant reply (or, with no `message_id`, the thread's
/// last turn): cancel the in-flight turn (if any), fork the session
/// transcript and message log to drop the answer and everything after it,
/// then restart the turn with the same user prompt that produced it.
pub async fn regenerate(request: RegenerateRequest) -> Result<Outcome<Value>, ThreadsError> {
    let client_id = request.client_id.unwrap_or_else(|| "system".to_string());
    let thread_id = request.thread_id;
    let dir = workspace_dir().await.map_err(ThreadsError::Message)?;

    crate::web_chat::cancel_chat(&client_id, &thread_id)
        .await
        .map_err(ThreadsError::Message)?;

    let target_request_id = match &request.message_id {
        Some(message_id) => Some(reply_run_id(message_id).map(str::to_string).ok_or_else(
            || {
                ThreadsError::Message(format!(
                    "message {message_id} is not a regenerable assistant reply"
                ))
            },
        )?),
        None => None,
    };

    let dir_for_blocking = dir.clone();
    let thread_id_for_blocking = thread_id.clone();
    let target_for_blocking = target_request_id.clone();
    let (prompt, cut_request_id) = tokio::task::spawn_blocking(move || {
        truncate_transcript_for_regenerate(
            &dir_for_blocking,
            &thread_id_for_blocking,
            target_for_blocking.as_deref(),
        )
    })
    .await
    .map_err(|e| ThreadsError::Message(format!("truncate transcript task: {e}")))?
    .map_err(ThreadsError::Message)?
    .ok_or_else(|| {
        ThreadsError::Message(format!("thread {thread_id} has no turn to regenerate"))
    })?;

    clear_dropped_turn_states(&dir, &thread_id, &cut_request_id).await;
    super::delete_after(&thread_id, &run_reply_message_id(&cut_request_id)).await?;

    crate::web_chat::invalidate_thread_sessions(&thread_id).await;

    let new_request_id = crate::web_chat::start_chat(
        &client_id,
        &thread_id,
        &prompt,
        None,
        None,
        None,
        None,
        crate::web_chat::ChatRequestMetadata::default(),
    )
    .await
    .map_err(|e| ThreadsError::Message(e.to_string()))?;

    Ok(Outcome::single_log(
        json!(EditOrRegenerateResponse {
            request_id: new_request_id,
        }),
        "turn regenerated",
    ))
}

/// Scan a thread's message log (append order) for the first deterministic
/// assistant-reply id after `after_message_id`, and return the `request_id`
/// it was minted for. `Ok(None)` when `after_message_id` has no reply yet
/// (or is the log's last message).
async fn next_reply_request_id_after(
    dir: &std::path::Path,
    thread_id: &str,
    after_message_id: &str,
) -> Result<Option<String>, String> {
    let messages =
        conversations::blocking::get_messages(dir.to_path_buf(), thread_id.to_string()).await?;
    let Some(start) = messages.iter().position(|m| m.id == after_message_id) else {
        return Ok(None);
    };
    Ok(messages[start + 1..]
        .iter()
        .find_map(|m| reply_run_id(&m.id).map(str::to_string)))
}

/// Resolve the head generation of `thread_id`'s session transcript: the
/// `SessionRef`, its locator, and its current content.
///
/// `find_root_transcript_for_thread` locates *some* root file for the thread
/// (used only to recover the `agent_id` a fresh [`SessionRef::scoped`]
/// needs); the actual head — walking any compaction/edit generations already
/// on disk — is then resolved through the locator itself
/// (`TranscriptLocator::head_generation`), never by trusting file-name sort
/// order (`tinyagents_session::transcript::view`'s resolver module doc explains why that
/// is unsafe: `.g1` sorts before the un-suffixed root).
fn resolve_head_transcript(
    workspace_dir: &std::path::Path,
    thread_id: &str,
) -> Result<
    (
        SessionRef,
        std::sync::Arc<dyn TranscriptLocator>,
        SessionTranscript,
    ),
    String,
> {
    let root_path =
        tinyagents_session::transcript::find_root_transcript_for_thread(workspace_dir, thread_id)
            .ok_or_else(|| format!("thread {thread_id} has no session transcript"))?;
    let root_transcript = tinyagents_session::transcript::read_transcript(&root_path)
        .map_err(|e| format!("read root transcript for thread {thread_id}: {e}"))?;
    let agent_id = root_transcript.meta.agent_id.clone().unwrap_or_default();
    let session_root = SessionRef::scoped(thread_id, agent_id);
    let locator: std::sync::Arc<dyn TranscriptLocator> =
        std::sync::Arc::new(FileTranscriptLocator::new(workspace_dir.to_path_buf()));
    let head = locator.head_generation(&session_root);
    let head_path = tinyagents_session::transcript::resolve_keyed_transcript_path(
        workspace_dir,
        &tinyagents_session::transcript::session_stem(&head),
    )
    .map_err(|e| format!("resolve head transcript path for thread {thread_id}: {e}"))?;
    let head_transcript = tinyagents_session::transcript::read_transcript(&head_path)
        .map_err(|e| format!("read head transcript for thread {thread_id}: {e}"))?;
    Ok((head, locator, head_transcript))
}

/// A truncation seed carrying the head transcript's own metadata forward
/// (agent name/id, provider, model, thread/parent-session linkage), stamped
/// with a fresh `updated`. Mirrors
/// `OpenHumanSessionHost::runtime_transcript_meta`'s field set — the
/// successor generation `truncate_into_next_generation` opens is written
/// with this exactly the way a compaction's own successor is.
fn truncation_seed(head: &SessionRef, current: &TranscriptMeta) -> TranscriptMeta {
    let now = chrono::Utc::now().to_rfc3339();
    TranscriptMeta {
        updated: now,
        session_id: Some(head.session_id()),
        parent_session_id: head.parent_session_id(),
        ..current.clone()
    }
}

/// Cut the head transcript right before the turn's first row (its user
/// prompt), keeping that prompt but dropping the turn's answer and
/// everything after — backs `edit_message`'s correlation-through-next-reply
/// path.
fn truncate_transcript_before_turn(
    workspace_dir: &std::path::Path,
    thread_id: &str,
    request_id: &str,
) -> Result<(), String> {
    let (head, locator, transcript) = resolve_head_transcript(workspace_dir, thread_id)?;
    let cut_index = transcript
        .messages
        .iter()
        .position(|m| m.request_id.as_deref() == Some(request_id))
        .ok_or_else(|| format!("no transcript row for turn {request_id} in thread {thread_id}"))?;
    let seed = truncation_seed(&head, &transcript.meta);
    locator
        .truncate_into_next_generation(&head, TruncateCut::BeforeIndex(cut_index), seed)
        .map(|_| ())
        .map_err(|e| format!("truncate transcript for thread {thread_id}: {e}"))
}

/// Cut the head transcript for a `regenerate` call: either a specific past
/// turn (kept up to and including that turn's user prompt) or, with
/// `target_request_id: None`, the thread's last turn
/// (`TruncateCut::LastAssistantTurn`). Returns the resent prompt and the
/// dropped turn's `request_id`, or `Ok(None)` when the thread has no turn to
/// regenerate (no transcript yet, or an empty one).
fn truncate_transcript_for_regenerate(
    workspace_dir: &std::path::Path,
    thread_id: &str,
    target_request_id: Option<&str>,
) -> Result<Option<(String, String)>, String> {
    let (head, locator, transcript) = resolve_head_transcript(workspace_dir, thread_id)?;
    let cut = match target_request_id {
        Some(request_id) => {
            let index = transcript
                .messages
                .iter()
                .position(|m| m.request_id.as_deref() == Some(request_id))
                .ok_or_else(|| {
                    format!("no transcript row for turn {request_id} in thread {thread_id}")
                })?;
            // Keep the turn's own first row (its user prompt) — cut right
            // after it, dropping the answer and everything after.
            TruncateCut::BeforeIndex(index + 1)
        }
        None => TruncateCut::LastAssistantTurn,
    };
    let seed = truncation_seed(&head, &transcript.meta);
    let (_, _, kept) = locator
        .truncate_into_next_generation(&head, cut, seed)
        .map_err(|e| format!("truncate transcript for thread {thread_id}: {e}"))?;
    let Some(last): Option<&TranscriptMessage> = kept.last() else {
        return Ok(None);
    };
    let request_id = match target_request_id {
        Some(request_id) => request_id.to_string(),
        None => last.request_id.clone().ok_or_else(|| {
            format!("last turn in thread {thread_id} has no request_id to regenerate")
        })?,
    };
    Ok(Some((last.content.clone(), request_id)))
}

/// Drop every turn-state snapshot the truncation orphaned: `cut_request_id`
/// itself, plus every later turn on the thread (by `started_at`). Best
/// effort — a store error here only means a stale "Agentic task insights"
/// entry lingers for a turn that no longer exists, not a failed edit.
async fn clear_dropped_turn_states(
    workspace_dir: &std::path::Path,
    thread_id: &str,
    cut_request_id: &str,
) {
    let dir = workspace_dir.to_path_buf();
    let thread_id_owned = thread_id.to_string();
    let cut_request_id_owned = cut_request_id.to_string();
    let result = tokio::task::spawn_blocking(move || {
        let turns =
            tinyagents_session::turn_state::store::list_thread(dir.clone(), &thread_id_owned)?;
        let Some(cut_started_at) = turns
            .iter()
            .find(|t| t.request_id == cut_request_id_owned)
            .map(|t| t.started_at.clone())
        else {
            // Never got a snapshot (e.g. a turn that errored before its
            // first progress event) — nothing to drop but itself.
            return tinyagents_session::turn_state::store::delete_turn(
                dir,
                &thread_id_owned,
                &cut_request_id_owned,
            );
        };
        let mut removed_any = false;
        for turn in turns.into_iter().filter(|t| t.started_at >= cut_started_at) {
            if tinyagents_session::turn_state::store::delete_turn(
                dir.clone(),
                &thread_id_owned,
                &turn.request_id,
            )
            .unwrap_or(false)
            {
                removed_any = true;
            }
        }
        Ok(removed_any)
    })
    .await;
    match result {
        Ok(Ok(_)) => {}
        Ok(Err(err)) => log::warn!(
            "[threads][edit] failed to clear dropped turn-state snapshots thread_id={thread_id} cut_request_id={cut_request_id} err={err}"
        ),
        Err(err) => log::warn!(
            "[threads][edit] clear-turn-state task did not run thread_id={thread_id} cut_request_id={cut_request_id} err={err}"
        ),
    }
}

#[cfg(test)]
#[path = "edit_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "edit_turn_state_tests.rs"]
mod turn_state_tests;
