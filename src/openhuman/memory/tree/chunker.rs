//! Markdown → bounded chunks with stable sequence numbers (Phase 1 / #707).
//!
//! The canonicalisers produce one big canonical Markdown blob per source
//! record; the chunker slices that into chunks of at most [`DEFAULT_CHUNK_MAX_TOKENS`]
//! so later phases (#709 seal budget = 10k tokens) can ingest them without
//! blowing past the summariser ceiling.
//!
//! Splitting strategy: prefer paragraph boundaries (`\n\n`), then single
//! newlines, then hard character-count cut. We never split a character in half.

use crate::openhuman::memory::tree::types::{approx_token_count, Chunk, Metadata, SourceKind};

/// Default upper bound on per-chunk tokens.
///
/// Aligned with the LLD's summariser 10k budget so a single chunk never blows
/// a seal on its own.
pub const DEFAULT_CHUNK_MAX_TOKENS: u32 = 10_000;

/// Tunable settings for the chunker.
#[derive(Clone, Debug)]
pub struct ChunkerOptions {
    pub max_tokens: u32,
}

impl Default for ChunkerOptions {
    fn default() -> Self {
        Self {
            max_tokens: DEFAULT_CHUNK_MAX_TOKENS,
        }
    }
}

/// Input to the chunker: the canonicalised source and its provenance.
///
/// Callers (typically canonicalisers via [`super::ingest`]) own construction;
/// the chunker does not interpret metadata beyond cloning it onto each chunk.
#[derive(Clone, Debug)]
pub struct ChunkerInput {
    pub source_kind: SourceKind,
    pub source_id: String,
    /// Canonical Markdown content — possibly very long.
    pub markdown: String,
    /// Base metadata; per-chunk `timestamp` defaults to `metadata.timestamp`.
    pub metadata: Metadata,
}

/// Slice `input.markdown` into chunks ≤ `opts.max_tokens` tokens each.
///
/// Returns chunks in source order with stable sequence numbers starting at 0.
/// Chunk IDs are deterministic (`types::chunk_id`), so re-chunking yields the
/// same ids for identical input.
pub fn chunk_markdown(input: &ChunkerInput, opts: &ChunkerOptions) -> Vec<Chunk> {
    let now = chrono::Utc::now();
    let pieces = split_by_token_budget(&input.markdown, opts.max_tokens);
    log::debug!(
        "[memory_tree::chunker] source_kind={} source_id={} len={} pieces={}",
        input.source_kind.as_str(),
        input.source_id,
        input.markdown.len(),
        pieces.len()
    );

    pieces
        .into_iter()
        .enumerate()
        .map(|(idx, content)| {
            let seq = idx as u32;
            let token_count = approx_token_count(&content);
            let id = super::types::chunk_id(input.source_kind, &input.source_id, seq, &content);
            Chunk {
                id,
                content,
                metadata: input.metadata.clone(),
                token_count,
                seq_in_source: seq,
                created_at: now,
            }
        })
        .collect()
}

/// Split `text` into pieces each ≤ `max_tokens` tokens.
///
/// Preference order for split boundaries:
/// 1. Paragraph (`\n\n`)
/// 2. Line (`\n`)
/// 3. Hard character cut (last resort; preserves UTF-8 code points)
fn split_by_token_budget(text: &str, max_tokens: u32) -> Vec<String> {
    let max_tokens = max_tokens.max(1);
    if text.is_empty() {
        return vec![String::new()];
    }
    if approx_token_count(text) <= max_tokens {
        return vec![text.to_string()];
    }

    // Approximate max chars per chunk (4 chars ≈ 1 token).
    let max_chars: usize = (max_tokens as usize).saturating_mul(4);

    // First: try paragraph split. Walk paragraphs, greedy-accumulate into
    // chunks ≤ max_chars.
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    if paragraphs.len() > 1 {
        if let Some(out) = pack_segments(&paragraphs, "\n\n", max_chars) {
            return out;
        }
    }

    // Fall back to line split.
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.len() > 1 {
        if let Some(out) = pack_segments(&lines, "\n", max_chars) {
            return out;
        }
    }

    // Fall back to hard character-count cut preserving UTF-8 boundaries.
    hard_split_by_chars(text, max_chars)
}

/// Greedily pack pre-split segments into chunks ≤ max_chars. Returns `None`
/// if any single segment is already too large — caller should try a finer
/// split.
fn pack_segments(segments: &[&str], sep: &str, max_chars: usize) -> Option<Vec<String>> {
    let sep_len = sep.len();
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();

    for seg in segments {
        let seg_len = seg.chars().count();
        // A single segment larger than max_chars forces a finer split.
        if seg_len > max_chars {
            return None;
        }
        let projected = if current.is_empty() {
            seg_len
        } else {
            current.chars().count() + sep_len + seg_len
        };
        if projected > max_chars {
            out.push(std::mem::take(&mut current));
            current.push_str(seg);
        } else {
            if !current.is_empty() {
                current.push_str(sep);
            }
            current.push_str(seg);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    Some(out)
}

/// Hard character-count cut preserving UTF-8 code-point boundaries.
fn hard_split_by_chars(text: &str, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut count = 0usize;
    for ch in text.chars() {
        if count + 1 > max_chars {
            out.push(std::mem::take(&mut current));
            count = 0;
        }
        current.push(ch);
        count += 1;
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn meta() -> Metadata {
        Metadata::point_in_time(SourceKind::Chat, "slack:#eng", "alice", Utc::now())
    }

    #[test]
    fn tiny_input_produces_single_chunk() {
        let input = ChunkerInput {
            source_kind: SourceKind::Chat,
            source_id: "slack:#eng".into(),
            markdown: "hello world".into(),
            metadata: meta(),
        };
        let chunks = chunk_markdown(&input, &ChunkerOptions::default());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, "hello world");
        assert_eq!(chunks[0].seq_in_source, 0);
    }

    #[test]
    fn empty_input_produces_one_empty_chunk() {
        let input = ChunkerInput {
            source_kind: SourceKind::Chat,
            source_id: "x".into(),
            markdown: "".into(),
            metadata: meta(),
        };
        let chunks = chunk_markdown(&input, &ChunkerOptions::default());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, "");
    }

    #[test]
    fn paragraph_boundaries_preferred() {
        // Build something that exceeds token budget so it must split.
        let para1 = "a".repeat(400); // ~100 tokens
        let para2 = "b".repeat(400);
        let para3 = "c".repeat(400);
        let text = format!("{para1}\n\n{para2}\n\n{para3}");
        let input = ChunkerInput {
            source_kind: SourceKind::Document,
            source_id: "doc1".into(),
            markdown: text,
            metadata: meta(),
        };
        let chunks = chunk_markdown(
            &input,
            &ChunkerOptions {
                max_tokens: 150, // forces split at paragraph
            },
        );
        assert!(chunks.len() >= 2);
        // Every chunk should be a multiple of complete paragraphs — so starts
        // with 'a', 'b', or 'c' and doesn't mix them arbitrarily.
        for c in &chunks {
            let first = c.content.chars().next().unwrap();
            assert!(
                matches!(first, 'a' | 'b' | 'c'),
                "chunk starts with unexpected char: {:?}",
                c.content.chars().take(10).collect::<String>()
            );
        }
    }

    #[test]
    fn falls_back_to_line_split_when_no_paragraphs() {
        let text = (0..30)
            .map(|i| format!("line-{i}-{}", "x".repeat(40)))
            .collect::<Vec<_>>()
            .join("\n");
        let input = ChunkerInput {
            source_kind: SourceKind::Chat,
            source_id: "x".into(),
            markdown: text,
            metadata: meta(),
        };
        let chunks = chunk_markdown(
            &input,
            &ChunkerOptions {
                max_tokens: 80, // forces several splits
            },
        );
        assert!(chunks.len() >= 2);
        for c in &chunks {
            assert!(!c.content.contains("\n\n")); // no paragraph joins in output
        }
    }

    #[test]
    fn chunk_ids_are_stable_across_runs() {
        let input = ChunkerInput {
            source_kind: SourceKind::Chat,
            source_id: "slack:#eng".into(),
            markdown: "hello".into(),
            metadata: meta(),
        };
        let a = chunk_markdown(&input, &ChunkerOptions::default());
        let b = chunk_markdown(&input, &ChunkerOptions::default());
        assert_eq!(
            a.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
            b.iter().map(|c| c.id.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn sequence_numbers_start_at_zero() {
        let text = (0..10)
            .map(|i| format!("p{i}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let input = ChunkerInput {
            source_kind: SourceKind::Document,
            source_id: "d".into(),
            markdown: text,
            metadata: meta(),
        };
        let chunks = chunk_markdown(
            &input,
            &ChunkerOptions {
                max_tokens: 2, // forces one chunk per paragraph basically
            },
        );
        for (idx, c) in chunks.iter().enumerate() {
            assert_eq!(c.seq_in_source, idx as u32);
        }
    }

    #[test]
    fn utf8_boundaries_preserved_on_hard_split() {
        // Single long line with no paragraph/line splits → falls to hard cut.
        // Use a 3-byte-per-char Unicode codepoint (Chinese) and force a cut.
        let text = "中".repeat(400);
        let input = ChunkerInput {
            source_kind: SourceKind::Document,
            source_id: "d".into(),
            markdown: text.clone(),
            metadata: meta(),
        };
        let chunks = chunk_markdown(
            &input,
            &ChunkerOptions {
                max_tokens: 50, // ~200 chars
            },
        );
        // Rejoining must equal the original.
        let rejoined: String = chunks.iter().map(|c| c.content.as_str()).collect();
        assert_eq!(rejoined, text);
    }

    #[test]
    fn zero_token_budget_is_clamped_without_empty_leading_chunk() {
        let input = ChunkerInput {
            source_kind: SourceKind::Document,
            source_id: "d".into(),
            markdown: "abcdef".into(),
            metadata: meta(),
        };
        let chunks = chunk_markdown(&input, &ChunkerOptions { max_tokens: 0 });
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|chunk| !chunk.content.is_empty()));
        let rejoined: String = chunks.iter().map(|c| c.content.as_str()).collect();
        assert_eq!(rejoined, "abcdef");
    }
}
