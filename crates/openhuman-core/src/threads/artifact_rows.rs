//! Artifact rows in a thread's message log.
//!
//! When an artifact becomes ready in a chat, a row is appended to the thread's
//! message log recording which turn made it: `type: "artifact"` with
//! `extraMetadata: { artifactId, requestId }`. The UI reads the row to render
//! the artifact's card inside that turn's reply, so a card stays with its
//! message across refreshes, cleared client storage and other devices. The row
//! is the only record of that placement; the UI's own artifact state is a
//! cache.
//!
//! The rows are for the UI only. Every core reader of the message log goes
//! through [`crate::threads::store::get_messages`] or its blocking twin, which
//! drop them, so they never reach a model (title generation, memory backfill,
//! voice context, channel adapters, edit/regenerate). Only the UI's
//! `threads.messages_list` reads the log with them.
//!
//! Artifacts made before these rows existed get one on demand
//! ([`backfill_artifact_rows`]): the turn is the first agent reply saved at or
//! after the artifact, since a turn makes its artifacts before its reply is
//! saved.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::json;
use tinybus::{EventHandler, SubscriptionHandle};

use crate::artifacts::types::{ArtifactMeta, ArtifactStatus};
use crate::core::events::DomainEvent;
use crate::core::runtime::DispatchCtx;
use crate::threads::store::{self as conversations, ConversationMessage};

/// `type` of an artifact row.
pub const ARTIFACT_MESSAGE_TYPE: &str = "artifact";

const LOG_PREFIX: &str = "[threads:artifact-rows]";

/// Artifacts read per page when a thread is backfilled.
const BACKFILL_PAGE: usize = 1_000;

/// The row id for an artifact. One row per artifact: a new version, a retry or
/// a re-delivered ready event finds the row already there.
pub fn artifact_row_id(artifact_id: &str) -> String {
    format!("{ARTIFACT_ROW_ID_PREFIX}{artifact_id}")
}

/// Prefix of every artifact row's id ([`artifact_row_id`]).
const ARTIFACT_ROW_ID_PREFIX: &str = "artifact:";

/// Whether `message_id` names an artifact row: for readers that see ids but not
/// message types, such as search hits.
pub fn is_artifact_row_id(message_id: &str) -> bool {
    message_id.starts_with(ARTIFACT_ROW_ID_PREFIX)
}

/// Whether `message` is an artifact row.
pub fn is_artifact_row(message: &ConversationMessage) -> bool {
    message.message_type == ARTIFACT_MESSAGE_TYPE
}

/// `messages` without artifact rows: what every non-UI reader sees.
pub fn without_artifact_rows(messages: Vec<ConversationMessage>) -> Vec<ConversationMessage> {
    messages
        .into_iter()
        .filter(|message| !is_artifact_row(message))
        .collect()
}

fn artifact_row(
    artifact_id: &str,
    title: &str,
    request_id: Option<&str>,
    created_at: String,
) -> ConversationMessage {
    let mut extra_metadata = json!({ "artifactId": artifact_id });
    if let Some(request_id) = request_id {
        extra_metadata["requestId"] = json!(request_id);
    }
    ConversationMessage {
        id: artifact_row_id(artifact_id),
        content: title.to_string(),
        message_type: ARTIFACT_MESSAGE_TYPE.to_string(),
        extra_metadata,
        sender: "agent".to_string(),
        created_at,
    }
}

/// Append the row for `artifact_id` unless the thread already has it. Returns
/// whether a row was written.
pub async fn record_artifact_row(
    ctx: &DispatchCtx,
    workspace_dir: PathBuf,
    thread_id: &str,
    artifact_id: &str,
    title: &str,
    request_id: Option<&str>,
) -> Result<bool, String> {
    let messages = conversations::blocking::get_messages_with_artifact_rows(
        ctx.clone(),
        workspace_dir.clone(),
        thread_id,
    )
    .await?;
    let row_id = artifact_row_id(artifact_id);
    if messages.iter().any(|message| message.id == row_id) {
        return Ok(false);
    }
    let row = artifact_row(artifact_id, title, request_id, Utc::now().to_rfc3339());
    conversations::blocking::append_message(ctx.clone(), workspace_dir, thread_id.to_string(), row)
        .await?;
    log::debug!(
        "{LOG_PREFIX} recorded artifact_id={artifact_id} thread_id={thread_id} request_id={request_id:?}"
    );
    Ok(true)
}

fn parse_time(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

/// The turn that made an artifact created at `created_at`: the first agent
/// reply saved at or after it. `None` when the artifact is newer than every
/// reply (its turn has not saved one).
fn owning_request_id(
    messages: &[ConversationMessage],
    created_at: DateTime<Utc>,
) -> Option<String> {
    messages
        .iter()
        .filter(|message| message.sender == "agent" && !is_artifact_row(message))
        .filter_map(|message| {
            let request_id = message.extra_metadata.get("requestId")?.as_str()?;
            Some((parse_time(&message.created_at)?, request_id))
        })
        .filter(|(saved_at, _)| *saved_at >= created_at)
        .min_by_key(|(saved_at, _)| *saved_at)
        .map(|(_, request_id)| request_id.to_string())
}

/// Rows for `artifacts` the thread has none for, in creation order. Only ready
/// artifacts without a tool call get one: a tool call's artifact renders with
/// its tool call.
pub(crate) fn missing_rows(
    messages: &[ConversationMessage],
    artifacts: &[ArtifactMeta],
) -> Vec<ConversationMessage> {
    let mut missing: Vec<&ArtifactMeta> = artifacts
        .iter()
        .filter(|artifact| artifact.status == ArtifactStatus::Ready)
        .filter(|artifact| artifact.tool_call_id.is_none())
        .filter(|artifact| {
            let row_id = artifact_row_id(&artifact.id);
            !messages.iter().any(|message| message.id == row_id)
        })
        .collect();
    missing.sort_by_key(|artifact| artifact.created_at);
    missing
        .into_iter()
        .map(|artifact| {
            let request_id = owning_request_id(messages, artifact.created_at);
            artifact_row(
                &artifact.id,
                &artifact.title,
                request_id.as_deref(),
                artifact.created_at.to_rfc3339(),
            )
        })
        .collect()
}

/// Write rows for the thread's artifacts that have none (made before these
/// rows existed, or whose row write failed), given the thread's full message
/// log. Returns the rows written.
pub async fn backfill_artifact_rows(
    ctx: &DispatchCtx,
    workspace_dir: &Path,
    thread_id: &str,
    messages: &[ConversationMessage],
) -> Result<Vec<ConversationMessage>, String> {
    let mut artifacts = Vec::new();
    loop {
        let (page, total) = crate::artifacts::store::list_artifacts(
            workspace_dir,
            artifacts.len(),
            BACKFILL_PAGE,
            Some(thread_id),
        )
        .await?;
        let empty = page.is_empty();
        artifacts.extend(page);
        if empty || artifacts.len() >= total {
            break;
        }
    }
    let mut written = Vec::new();
    for row in missing_rows(messages, &artifacts) {
        let stored = conversations::blocking::append_message(
            ctx.clone(),
            workspace_dir.to_path_buf(),
            thread_id.to_string(),
            row,
        )
        .await?;
        written.push(stored);
    }
    if !written.is_empty() {
        log::info!(
            "{LOG_PREFIX} backfilled {} artifact row(s) thread_id={thread_id}",
            written.len()
        );
    }
    Ok(written)
}

/// Writes an artifact's row when it becomes ready in a chat.
struct ArtifactRowSubscriber;

#[async_trait]
impl EventHandler<DomainEvent> for ArtifactRowSubscriber {
    fn name(&self) -> &str {
        "threads::artifact_rows"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["artifact"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::ArtifactReady {
            artifact_id,
            title,
            workspace_dir,
            thread_id: Some(thread_id),
            tool_call_id: None,
            request_id,
            ..
        } = event
        else {
            return;
        };
        // A bus event carries no context of its own (see
        // `DispatchCtx::host_default`).
        if let Err(error) = record_artifact_row(
            &DispatchCtx::host_default(),
            PathBuf::from(workspace_dir),
            thread_id,
            artifact_id,
            title,
            request_id.as_deref(),
        )
        .await
        {
            log::warn!(
                "{LOG_PREFIX} row write failed artifact_id={artifact_id} thread_id={thread_id}: {error}"
            );
        }
    }
}

static ARTIFACT_ROW_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Register the subscriber that writes artifact rows. Idempotent.
pub fn register_artifact_row_subscriber() {
    if ARTIFACT_ROW_HANDLE.get().is_some() {
        return;
    }
    match crate::core::bus::BUS.subscribe(Arc::new(ArtifactRowSubscriber)) {
        Some(handle) => {
            let _ = ARTIFACT_ROW_HANDLE.set(handle);
            log::info!("{LOG_PREFIX} subscriber registered (domain=artifact)");
        }
        None => log::warn!("{LOG_PREFIX} failed to register subscriber — bus not initialized"),
    }
}

#[cfg(test)]
#[path = "artifact_rows_tests.rs"]
mod tests;
