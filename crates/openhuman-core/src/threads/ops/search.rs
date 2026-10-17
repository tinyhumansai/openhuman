//! `threads.search` — find messages across every thread, for the global
//! (`Cmd`/`Ctrl+K`) search. A thin RPC over [`transcript_search`]: the same
//! trigram/CJK-bigram inverted index the cross-chat context reader uses.
//!
//! Each hit carries a short snippet around the match rather than the whole
//! message, so a broad query never ships every long transcript to the UI.

use super::crud::transcript_search;
use super::support::envelope;
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;
use crate::threads::ApiEnvelope;

/// Hits returned when the caller does not ask for a number.
const DEFAULT_LIMIT: usize = 20;
/// Upper bound on hits per search.
const MAX_LIMIT: usize = 100;
/// Characters of context kept on each side of the match.
const SNIPPET_CONTEXT: usize = 60;

/// Request for [`thread_search`].
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ThreadSearchRequest {
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// One matched message.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSearchHit {
    pub thread_id: String,
    pub message_id: String,
    pub role: String,
    /// The message text around the first match, with `…` where it was cut.
    pub snippet: String,
    pub created_at: String,
}

/// Response for [`thread_search`]: hits newest first.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ThreadSearchResponse {
    pub hits: Vec<ThreadSearchHit>,
}

/// See the module docs. A blank query answers no hits without reading.
pub async fn thread_search(
    ctx: &DispatchCtx,
    request: ThreadSearchRequest,
) -> Result<Outcome<ApiEnvelope<ThreadSearchResponse>>, String> {
    let query = request.query.trim();
    if query.is_empty() {
        return Ok(envelope(
            ThreadSearchResponse { hits: Vec::new() },
            None,
            None,
        ));
    }
    let limit = request.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let hits = transcript_search(ctx, query, limit, None)
        .await?
        .into_iter()
        .map(|hit| ThreadSearchHit {
            snippet: snippet(&hit.content, query),
            thread_id: hit.thread_id,
            message_id: hit.message_id,
            role: hit.role,
            created_at: hit.created_at,
        })
        .collect();
    Ok(envelope(ThreadSearchResponse { hits }, None, None))
}

/// `content` cut to [`SNIPPET_CONTEXT`] characters either side of the first
/// case-insensitive occurrence of `query`, whitespace collapsed. The index
/// matches on trigrams, so a hit need not contain the query verbatim; then
/// the snippet is the message's start.
pub(super) fn snippet(content: &str, query: &str) -> String {
    let text: Vec<char> = content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .collect();
    let fold = |c: &char| c.to_lowercase().next().unwrap_or(*c);
    let needle: Vec<char> = query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .map(|c| fold(&c))
        .collect();
    let lower: Vec<char> = text.iter().map(fold).collect();
    let at = if needle.is_empty() || needle.len() > lower.len() {
        None
    } else {
        (0..=lower.len() - needle.len()).find(|&i| lower[i..i + needle.len()] == needle[..])
    };
    let (start, end) = match at {
        Some(i) => (
            i.saturating_sub(SNIPPET_CONTEXT),
            (i + needle.len() + SNIPPET_CONTEXT).min(text.len()),
        ),
        None => (0, (SNIPPET_CONTEXT * 2).min(text.len())),
    };
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(&text[start..end]);
    if end < text.len() {
        out.push('…');
    }
    out
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
