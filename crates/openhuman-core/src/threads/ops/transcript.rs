//! Paginated projection of a thread's settled transcript.

use super::support::{counts, envelope, workspace_dir};
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

/// Project a thread's settled transcript (derived from `session_raw/*.jsonl`)
/// into typed display items, newest-first paginated. Returns an empty page with
/// `hasTranscript: false` when the thread has no persisted transcript yet.
pub async fn transcript_get(
    request: TranscriptGetRequest,
) -> Result<Outcome<ApiEnvelope<tinyagents_session::transcript::view::TranscriptPage>>, String> {
    let dir = workspace_dir().await?;
    let thread_id = request.thread_id.trim();
    if thread_id.is_empty() {
        return Err("thread_id is required".to_string());
    }
    let page = tinyagents_session::transcript::view::get_page(
        &dir,
        thread_id,
        request.cursor.as_deref(),
        request.limit,
    );
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
