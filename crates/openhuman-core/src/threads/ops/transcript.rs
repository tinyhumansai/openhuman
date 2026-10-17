//! Paginated projection of a thread's settled transcript.

use super::support::{counts, envelope, transcript_store};
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;
use crate::threads::{ApiEnvelope, PaginationMeta};

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
    let page = tokio::task::spawn_blocking(move || {
        tinyagents_session::transcript::view::get_page_in(
            locator.as_ref(),
            &thread_id,
            None,
            cursor.as_deref(),
            limit,
        )
    })
    .await
    .map_err(|e| format!("transcript page task: {e}"))?;
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

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod tests;
