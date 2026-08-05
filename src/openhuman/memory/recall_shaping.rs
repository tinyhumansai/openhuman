//! Bound the size of one recalled memory before it reaches the model.
//!
//! Recall returns whole documents, and a document can be large — a synced email
//! thread, or (from an older build) a subagent's entire system-prompt envelope
//! saved as a "conversation". One such document filled the recall tool's whole
//! result budget, hid every other hit, and pushed the agent to re-search the
//! live source it had just been handed the answer from.
//!
//! So a recalled entry is condensed to the handful of *chunks* most relevant to
//! the query — the same chunk unit the store already splits documents into —
//! and then hard-capped. The chunk selection is the useful part (the reader
//! sees the matching passages, not a wall of unrelated text); the char cap is
//! the backstop that also bounds documents stored before any of this existed.

use crate::openhuman::memory::store::UnifiedMemory;
use crate::openhuman::util::truncate_with_ellipsis;

/// Most chunks surfaced from one source document. Enough to carry the matching
/// passage plus a little surrounding context; few enough that one document
/// cannot dominate the result.
pub(crate) const MAX_CHUNKS_PER_SOURCE: usize = 3;

/// Per-chunk display budget. Each surfaced chunk is trimmed to this so three of
/// them fit inside the entry cap and the reader sees the head of each relevant
/// passage rather than one long chunk crowding out the others.
const MAX_CHARS_PER_CHUNK: usize = 400;

/// Hard ceiling on one entry's rendered characters, whatever its chunking —
/// the backstop that bounds even a single un-splittable blob (an old subagent
/// envelope saved as one document). Sized to hold the three per-chunk budgets
/// plus the elision markers between them.
pub(crate) const MAX_CHARS_PER_ENTRY: usize = 1400;

/// Token budget per chunk, matching the store's own `upsert_document` chunking
/// so "chunk" means the same thing on the way out as on the way in.
const CHUNK_MAX_TOKENS: usize = 225;

/// Condense `content` to at most [`MAX_CHUNKS_PER_SOURCE`] query-relevant chunks
/// and [`MAX_CHARS_PER_ENTRY`] characters.
pub(crate) fn condense_recall_content(query: &str, content: &str) -> String {
    condense_with(
        query,
        content,
        MAX_CHUNKS_PER_SOURCE,
        MAX_CHARS_PER_CHUNK,
        MAX_CHARS_PER_ENTRY,
    )
}

fn condense_with(
    query: &str,
    content: &str,
    max_chunks: usize,
    max_chars_per_chunk: usize,
    max_chars: usize,
) -> String {
    let trimmed = content.trim();
    let chunks = UnifiedMemory::chunk_document_content(trimmed, CHUNK_MAX_TOKENS);
    // Counts and lengths only — this function's input is recalled memory, so
    // its content must not reach the log.
    log::debug!(
        "[memory::recall_shaping] condense entry chars={} chunks={} max_chunks={max_chunks} \
         max_chars={max_chars} query_chars={}",
        trimmed.chars().count(),
        chunks.len(),
        query.chars().count()
    );

    // Short, few-chunk content is the common case for a real fact — pass it
    // through untouched so nothing is lost to condensation it didn't need.
    if chunks.len() <= max_chunks && trimmed.chars().count() <= max_chars {
        log::debug!(
            "[memory::recall_shaping] condense exit branch=passthrough chars={}",
            trimmed.chars().count()
        );
        return trimmed.to_string();
    }
    if chunks.is_empty() {
        let out = hard_cap(trimmed, max_chars);
        log::debug!(
            "[memory::recall_shaping] condense exit branch=unsplittable chars={}",
            out.chars().count()
        );
        return out;
    }

    let query_terms = query_terms(query);
    // Rank by relevance to pick *which* chunks, but keep the survivors in their
    // original order so the passage still reads top-to-bottom.
    let mut ranked: Vec<(usize, usize)> = chunks
        .iter()
        .enumerate()
        .map(|(idx, chunk)| (idx, chunk_relevance(chunk, &query_terms)))
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    // Once anything matches, an unmatched chunk is not a runner-up — it is
    // unrelated text, and padding the remaining slots with it hands the model
    // content the query never asked for next to content it did. Only when
    // nothing matches does the head of the document become the best available
    // answer, which is the fallback the all-zero case keeps (the sort's
    // index tie-break leaves the chunks in document order).
    let has_match = ranked.iter().any(|(_, score)| *score > 0);
    let mut keep: Vec<usize> = ranked
        .into_iter()
        .filter(|(_, score)| !has_match || *score > 0)
        .take(max_chunks)
        .map(|(idx, _)| idx)
        .collect();
    keep.sort_unstable();

    let dropped = chunks.len() - keep.len();
    let mut rendered = String::new();
    for (position, &idx) in keep.iter().enumerate() {
        if position > 0 {
            rendered.push_str(" … ");
        }
        // Trim each kept chunk so three of them share the entry budget rather
        // than the first one consuming it all.
        rendered.push_str(&truncate_with_ellipsis(&chunks[idx], max_chars_per_chunk));
    }
    if dropped > 0 {
        rendered.push_str(&format!(" … (+{dropped} more section(s))"));
    }

    let out = hard_cap(&rendered, max_chars);
    log::debug!(
        "[memory::recall_shaping] condense exit branch=selected kept={} dropped={dropped} \
         had_match={has_match} chars={}",
        keep.len(),
        out.chars().count()
    );
    out
}

/// Truncate so the result is at most `max_chars` characters *including* the
/// ellipsis — `truncate_with_ellipsis` alone can exceed its budget by the
/// suffix length, which would defeat a hard cap.
/// Characters `truncate_with_ellipsis` appends when it truncates.
const ELLIPSIS_CHARS: usize = 3;

fn hard_cap(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    // `truncate_with_ellipsis` appends "..." ON TOP of the budget it is given,
    // so the suffix has to come out of the budget or the result is three chars
    // over the cap this function exists to enforce.
    truncate_with_ellipsis(text, max_chars.saturating_sub(ELLIPSIS_CHARS))
}

/// Lowercased query tokens, split on whitespace. Substring matching keeps this
/// correct for Korean (no word segmenter needed): a query token like `콜로라도`
/// matches any chunk containing it.
fn query_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|term| term.trim().to_lowercase())
        .filter(|term| !term.is_empty())
        .collect()
}

/// How many distinct query terms appear in this chunk. Cheap and
/// language-agnostic — enough to pick the matching passages out of a document.
fn chunk_relevance(chunk: &str, query_terms: &[String]) -> usize {
    let lower = chunk.to_lowercase();
    query_terms
        .iter()
        .filter(|term| lower.contains(term.as_str()))
        .count()
}

#[cfg(test)]
#[path = "recall_shaping_tests.rs"]
mod tests;
