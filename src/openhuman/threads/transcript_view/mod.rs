//! Transcript-derived view: project the append-only `session_raw/*.jsonl`
//! source of truth into typed display items for the chat renderer, with
//! newest-first pagination over a bounded in-memory cache.
//!
//! Entry point: [`get_page`] (used by the `threads.transcript_get` RPC).

mod cache;
mod project;
pub mod types;

use std::path::Path;

use serde::Serialize;

pub use types::{DisplayItem, ProjectedTranscript, ToolCallStatus};

const LOG_PREFIX: &str = "[threads][transcript]";

/// Default page size — roughly one screen of chat items.
pub const DEFAULT_LIMIT: usize = 50;
/// Hard upper bound on a single page so a client can't request an unbounded
/// projection slice.
pub const MAX_LIMIT: usize = 500;

/// One newest-first page of a thread's projected transcript.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptPage {
    pub thread_id: String,
    /// Display items for this page, **newest-first**.
    pub items: Vec<DisplayItem>,
    /// Total top-level items available for the thread.
    pub total: usize,
    /// Opaque cursor to pass back for the next (older) page; `null` at the end.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// `true` when more (older) items remain beyond this page.
    pub has_more: bool,
    /// `false` when the thread has no persisted transcript yet (empty page).
    pub has_transcript: bool,
}

/// Project `thread_id`'s transcript and return one newest-first page.
///
/// `cursor` is the opaque token returned as `next_cursor` by a previous call
/// (an offset from the newest item); `None`/empty starts at the newest item.
/// `limit` defaults to [`DEFAULT_LIMIT`] and is clamped to [`MAX_LIMIT`].
pub fn get_page(
    workspace_dir: &Path,
    thread_id: &str,
    cursor: Option<&str>,
    limit: Option<usize>,
) -> TranscriptPage {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = parse_cursor(cursor);

    let Some(projected) = cache::global().get_or_project(workspace_dir, thread_id) else {
        log::debug!("{LOG_PREFIX} get_page thread={thread_id}: no transcript");
        return TranscriptPage {
            thread_id: thread_id.to_string(),
            items: Vec::new(),
            total: 0,
            next_cursor: None,
            has_more: false,
            has_transcript: false,
        };
    };

    let total = projected.items.len();
    let start = offset.min(total);
    let end = (offset + limit).min(total);
    // Newest-first: item `offset` is the newest, walking backwards from the end.
    let items: Vec<DisplayItem> = (start..end)
        .map(|i| projected.items[total - 1 - i].clone())
        .collect();
    let has_more = end < total;
    let next_cursor = has_more.then(|| end.to_string());

    log::debug!(
        "{LOG_PREFIX} get_page thread={thread_id} total={total} offset={offset} returned={} has_more={has_more}",
        items.len()
    );

    TranscriptPage {
        thread_id: thread_id.to_string(),
        items,
        total,
        next_cursor,
        has_more,
        has_transcript: true,
    }
}

/// Parse the opaque cursor into a numeric offset (0 on absent/invalid).
fn parse_cursor(cursor: Option<&str>) -> usize {
    cursor
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .and_then(|c| c.parse::<usize>().ok())
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
