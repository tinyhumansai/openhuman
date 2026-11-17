//! Citations collected during a chat turn for the web chat surface.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::core::runtime::DispatchCtx;

/// A memory citation attached to a chat reply so the UI can show where an
/// answer came from. `key` is the item kind, `namespace` its source kind,
/// `timestamp` when it was observed, and `snippet` a short excerpt.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct TurnCitation {
    /// Item id.
    pub id: String,
    /// Item kind (`document`, `conversation`, `learning`).
    pub key: String,
    /// Source kind (`folder`, `agent`, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Relevance, when the engine reports one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// When the item was observed (RFC 3339), or empty.
    pub timestamp: String,
    /// Short excerpt.
    pub snippet: String,
}

impl From<&tinymemory_api::Hit> for TurnCitation {
    fn from(hit: &tinymemory_api::Hit) -> Self {
        Self {
            id: hit.id.0.clone(),
            key: hit.kind.as_str().to_string(),
            namespace: Some(hit.meta.source.kind.as_str().to_string()),
            score: Some(f64::from(hit.score)).filter(|score| *score > 0.0),
            timestamp: hit
                .meta
                .observed_at
                .map(|at| at.to_rfc3339())
                .unwrap_or_default(),
            snippet: hit.text.chars().take(TURN_CITATION_SNIPPET_CHARS).collect(),
        }
    }
}

impl From<&tinymemory_api::Citation> for TurnCitation {
    fn from(citation: &tinymemory_api::Citation) -> Self {
        Self {
            id: citation.id.0.clone(),
            key: citation.kind.as_str().to_string(),
            namespace: Some(citation.meta.source.kind.as_str().to_string()),
            score: citation.score.map(f64::from),
            timestamp: citation
                .meta
                .observed_at
                .map(|at| at.to_rfc3339())
                .unwrap_or_default(),
            snippet: citation
                .snippet
                .chars()
                .take(TURN_CITATION_SNIPPET_CHARS)
                .collect(),
        }
    }
}

/// Longest snippet a [`TurnCitation`] carries.
pub const TURN_CITATION_SNIPPET_CHARS: usize = 280;

/// Most citations kept per thread between two drains.
const MAX_TURN_CITATIONS: usize = 20;

type TurnCitations = Mutex<HashMap<String, Vec<TurnCitation>>>;

fn turn_citations(ctx: &DispatchCtx) -> Arc<TurnCitations> {
    crate::core::runtime::slot_for::<TurnCitations>(ctx)
}

fn extend_thread(
    ctx: &DispatchCtx,
    thread_id: &str,
    citations: impl IntoIterator<Item = TurnCitation>,
) {
    let citations_by_thread = turn_citations(ctx);
    let mut all = citations_by_thread
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let entry = all.entry(thread_id.to_string()).or_default();
    for citation in citations {
        if entry.len() >= MAX_TURN_CITATIONS {
            break;
        }
        if !entry.iter().any(|existing| existing.id == citation.id) {
            entry.push(citation);
        }
    }
}

/// Records citations returned by the memory tool for a thread's in-flight
/// turn.
pub(crate) fn record_recall_citations(
    ctx: &DispatchCtx,
    thread_id: &str,
    citations: &[tinymemory_api::Citation],
) {
    if citations.is_empty() {
        return;
    }
    extend_thread(ctx, thread_id, citations.iter().map(TurnCitation::from));
}

/// Records the citations in a turn's prefetched memory pack.
pub(crate) fn record_pack_citations(
    ctx: &DispatchCtx,
    thread_id: &str,
    citations: Vec<TurnCitation>,
) {
    if !citations.is_empty() {
        extend_thread(ctx, thread_id, citations);
    }
}

/// Drains the citations produced for `thread_id` since the last drain.
#[must_use]
pub(crate) fn take_turn_citations(ctx: &DispatchCtx, thread_id: &str) -> Vec<TurnCitation> {
    turn_citations(ctx)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(thread_id)
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "citations_tests.rs"]
mod tests;
