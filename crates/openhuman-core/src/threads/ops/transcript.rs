//! Paginated projection of a thread's settled transcript.

use super::support::{counts, envelope, transcript_store};
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;
use crate::threads::{store as conversations, ApiEnvelope, PaginationMeta};
use tinyagents_session::transcript::view::DisplayItem;

/// Request for [`transcript_get`]: the thread to project, plus newest-first
/// pagination controls. `cursor` is the opaque token from a prior page's
/// `nextCursor`; `limit` defaults to one screen.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TranscriptGetRequest {
    pub thread_id: String,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Project a thread's settled transcript (read from the session store the
/// request's tenant wrote it to) into typed display items, newest-first
/// paginated. Returns an empty page with `hasTranscript: false` when the
/// thread has no persisted transcript yet.
pub async fn transcript_get(
    ctx: &DispatchCtx,
    request: TranscriptGetRequest,
) -> Result<Outcome<ApiEnvelope<tinyagents_session::transcript::view::TranscriptPage>>, String> {
    let thread_id = request.thread_id.trim().to_string();
    if thread_id.is_empty() {
        return Err("thread_id is required".to_string());
    }
    // Resolved here, in the request's scope; the projection itself is blocking.
    let locator = transcript_store(ctx, &thread_id).await?.locator();
    let cursor = request.cursor.clone();
    let limit = request.limit;
    let page_thread_id = thread_id.clone();
    let mut page = tokio::task::spawn_blocking(move || {
        tinyagents_session::transcript::view::get_page_in(
            locator.as_ref(),
            &page_thread_id,
            None,
            cursor.as_deref(),
            limit,
        )
    })
    .await
    .map_err(|e| format!("transcript page task: {e}"))?;
    attach_voice_note_metadata(ctx, &thread_id, &mut page.items).await?;
    let counts = counts([
        ("items", page.items.len()),
        ("total", page.total),
        ("has_transcript", usize::from(page.has_transcript)),
    ]);
    let pagination = Some(PaginationMeta {
        limit: request
            .limit
            .unwrap_or(tinyagents_session::transcript::view::DEFAULT_LIMIT),
        offset: request
            .cursor
            .as_deref()
            .and_then(|c| c.trim().parse::<usize>().ok())
            .unwrap_or(0),
        count: page.total,
    });
    Ok(envelope(page, Some(counts), pagination))
}

/// Add voice-note references from the thread's durable UI rows to the
/// transcript projection. Conversation rows own UI-only metadata; copying it
/// into TinyAgents' transcript would change the frozen model history.
async fn attach_voice_note_metadata(
    ctx: &DispatchCtx,
    thread_id: &str,
    items: &mut [DisplayItem],
) -> Result<(), String> {
    let workspace_dir = ctx
        .load_config()
        .await
        .map_err(|error| format!("load config for voice-note projection: {error}"))?
        .workspace_dir;
    let rows =
        conversations::blocking::get_messages(ctx.clone(), workspace_dir, thread_id.to_string())
            .await?;

    for item in items {
        let (content, sender, request_id, projected_metadata) = match item {
            DisplayItem::UserMessage {
                content,
                request_id,
                extra_metadata,
                ..
            } => (
                content.as_str(),
                "user",
                request_id.as_deref(),
                extra_metadata,
            ),
            DisplayItem::AssistantMessage {
                content,
                request_id,
                extra_metadata,
                ..
            } => (
                content.as_str(),
                "agent",
                request_id.as_deref(),
                extra_metadata,
            ),
            _ => continue,
        };

        let matching_rows = rows
            .iter()
            .filter(|row| row.sender == sender)
            .filter(|row| {
                let row_request_id = row
                    .extra_metadata
                    .get("requestId")
                    .and_then(serde_json::Value::as_str);
                match (request_id, row_request_id) {
                    // A request id identifies the exact turn even when the
                    // transcript decorates the user text with model context.
                    // Assistant turns may contain several messages, so their
                    // content still selects the persisted final reply row.
                    (Some(projected), Some(stored)) => {
                        projected == stored && (sender == "user" || row.content == content)
                    }
                    // Legacy rows have no turn identity, so use exact content
                    // only when both sides are legacy.
                    (None, None) => row.content == content,
                    // A modern transcript item cannot fall back to a different
                    // or missing row request id. Legacy rows match only legacy
                    // items, and the unique-content check below stays strict.
                    _ => false,
                }
            })
            .collect::<Vec<_>>();
        if matching_rows.len() != 1 {
            continue;
        }
        let Some(voice_note) = matching_rows[0].extra_metadata.get("voiceNote") else {
            continue;
        };
        let metadata = projected_metadata.get_or_insert_with(|| serde_json::json!({}));
        if let Some(object) = metadata.as_object_mut() {
            object.insert("voiceNote".into(), voice_note.clone());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod tests;
