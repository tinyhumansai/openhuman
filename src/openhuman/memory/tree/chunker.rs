//! Markdown → bounded chunks with stable sequence numbers (Phase 1 / #707).
//!
//! The canonicalisers produce one big canonical Markdown blob per source
//! record; the chunker slices that into chunks of at most [`DEFAULT_CHUNK_MAX_TOKENS`]
//! so later phases (#709 seal budget = 10k tokens) can ingest them without
//! blowing past the summariser ceiling.
//!
//! ## Dispatch by source kind (Phase B)
//!
//! - **Chat**: split at `## ` message boundaries. Each message becomes one
//!   chunk. If a single message exceeds `max_tokens`, fall back to the
//!   paragraph/line/char splitter for that unit only and emit each piece with
//!   `partial_message = true`.
//! - **Email**: split at `---\nFrom:` separators. Each email in the thread
//!   becomes one chunk. Same oversize fallback as Chat.
//! - **Document**: original paragraph-based greedy packing (unchanged).

use crate::openhuman::memory::tree::types::{approx_token_count, Chunk, Metadata, SourceKind};

/// Default upper bound on per-chunk tokens.
///
/// Sized below the L0 seal budget (`source_tree::types::TOKEN_BUDGET = 4_500`)
/// so each seal accumulates roughly 1–3 chunks before firing — natural pacing
/// for the local 1B summariser, which produces noticeably better summaries
/// with smaller (≤4–5k) inputs than at the previous 10k cap.
pub const DEFAULT_CHUNK_MAX_TOKENS: u32 = 3_000;

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
///
/// ## Dispatch by source kind
///
/// - **Chat / Email**: split at message/email boundaries, then greedy-pack
///   consecutive units into a single chunk until adding the next unit would
///   exceed `max_tokens`. Oversize units (a single message > `max_tokens`)
///   fall back to the paragraph/line/char splitter and emit each piece with
///   `partial_message = true`.
/// - **Document**: original paragraph-based greedy packing (unchanged).
pub fn chunk_markdown(input: &ChunkerInput, opts: &ChunkerOptions) -> Vec<Chunk> {
    let now = chrono::Utc::now();
    let max_tokens = opts.max_tokens.max(1);
    let max_chars = (max_tokens as usize).saturating_mul(4);

    // Dispatch: pick splitting units based on source kind.
    let units: Vec<String> = match input.source_kind {
        SourceKind::Chat => split_chat_messages(&input.markdown),
        SourceKind::Email => split_email_messages(&input.markdown),
        SourceKind::Document => {
            // Document: run the existing paragraph splitter directly on the
            // whole blob. No message-unit concept.
            log::debug!(
                "[memory_tree::chunker] document source_id={} len={} — paragraph split",
                input.source_id,
                input.markdown.len()
            );
            split_by_token_budget(&input.markdown, max_tokens)
        }
    };

    if matches!(input.source_kind, SourceKind::Document) {
        // Already split by budget; wrap directly.
        return units
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
                    partial_message: false,
                }
            })
            .collect();
    }

    log::debug!(
        "[memory_tree::chunker] source_kind={} source_id={} len={} units={}",
        input.source_kind.as_str(),
        input.source_id,
        input.markdown.len(),
        units.len()
    );

    // For Chat and Email: greedy-pack consecutive units into chunks.
    // Units are accumulated until adding the next would exceed max_chars;
    // oversize single units fall back to sub-splitting with partial_message=true.
    let unit_separator = "\n\n";
    let sep_chars = unit_separator.chars().count();

    let mut out: Vec<Chunk> = Vec::new();
    let mut acc: Vec<String> = Vec::new();
    let mut acc_chars = 0usize;

    // Flush accumulated units as one packed chunk.
    let flush = |acc: &mut Vec<String>, acc_chars: &mut usize, out: &mut Vec<Chunk>| {
        if acc.is_empty() {
            return;
        }
        let content = acc.join(unit_separator);
        let seq = out.len() as u32;
        let tc = approx_token_count(&content);
        let id = super::types::chunk_id(input.source_kind, &input.source_id, seq, &content);
        out.push(Chunk {
            id,
            content,
            metadata: input.metadata.clone(),
            token_count: tc,
            seq_in_source: seq,
            created_at: now,
            partial_message: false,
        });
        acc.clear();
        *acc_chars = 0;
    };

    for unit in units {
        let unit_chars = unit.chars().count();

        if unit_chars > max_chars {
            // Oversize: flush any pending accumulator first, then sub-split.
            flush(&mut acc, &mut acc_chars, &mut out);
            let sub_pieces = split_by_token_budget(&unit, max_tokens);
            for piece in sub_pieces {
                let seq = out.len() as u32;
                let tc = approx_token_count(&piece);
                let id = super::types::chunk_id(input.source_kind, &input.source_id, seq, &piece);
                out.push(Chunk {
                    id,
                    content: piece,
                    metadata: input.metadata.clone(),
                    token_count: tc,
                    seq_in_source: seq,
                    created_at: now,
                    partial_message: true,
                });
            }
            continue;
        }

        // Compute projected size if we add this unit to the accumulator.
        let projected = if acc.is_empty() {
            unit_chars
        } else {
            acc_chars + sep_chars + unit_chars
        };

        if projected > max_chars {
            // Adding this unit would overflow — flush the accumulator first.
            flush(&mut acc, &mut acc_chars, &mut out);
        }

        if !acc.is_empty() {
            acc_chars += sep_chars;
        }
        acc_chars += unit_chars;
        acc.push(unit);
    }

    // Flush any remaining accumulated units.
    flush(&mut acc, &mut acc_chars, &mut out);

    if out.is_empty() {
        // Degenerate: empty input → one empty chunk, matching original behaviour.
        let id = super::types::chunk_id(input.source_kind, &input.source_id, 0, "");
        out.push(Chunk {
            id,
            content: String::new(),
            metadata: input.metadata.clone(),
            token_count: 0,
            seq_in_source: 0,
            created_at: now,
            partial_message: false,
        });
    }

    out
}

/// Split a canonical chat blob into per-message units at `## ` boundaries.
///
/// Each returned string starts with `## ` and includes everything up to but
/// not including the next `## ` boundary. If the blob starts with a `# `
/// header (legacy or unexpected), everything before the first `## ` is
/// dropped silently.
fn split_chat_messages(md: &str) -> Vec<String> {
    let mut pieces: Vec<String> = Vec::new();
    let mut current: Option<String> = None;

    for line in md.split_inclusive('\n') {
        if line.starts_with("## ") {
            if let Some(prev) = current.take() {
                let trimmed = prev.trim_end().to_string();
                if !trimmed.is_empty() {
                    pieces.push(trimmed);
                }
            }
            current = Some(line.to_string());
        } else if let Some(ref mut buf) = current {
            buf.push_str(line);
        }
        // Lines before the first `## ` (e.g. a leading `# ` header) are dropped.
    }

    if let Some(prev) = current.take() {
        let trimmed = prev.trim_end().to_string();
        if !trimmed.is_empty() {
            pieces.push(trimmed);
        }
    }

    if pieces.is_empty() && !md.trim().is_empty() {
        // No `## ` found at all — treat whole blob as one unit.
        pieces.push(md.trim_end().to_string());
    }

    pieces
}

/// Split a canonical email thread blob into per-email units.
///
/// Splits at `---` (alone on a line, optional trailing whitespace) followed
/// by a `From:` line within the next 8 lines. Each piece includes the `---`
/// separator and everything up to but not including the next `---\nFrom:`
/// boundary. Content before the first `---` separator is dropped (handles
/// any leading header that might have slipped through).
fn split_email_messages(md: &str) -> Vec<String> {
    let lines: Vec<&str> = md.split('\n').collect();
    let n = lines.len();
    let mut split_positions: Vec<usize> = Vec::new();

    for i in 0..n {
        let line = lines[i].trim_end();
        if line == "---" {
            // Check if one of the next 8 lines starts with `From:`
            let window_end = (i + 9).min(n);
            for j in (i + 1)..window_end {
                if lines[j].starts_with("From:") {
                    split_positions.push(i);
                    break;
                }
                // Skip blank lines between `---` and `From:`
                if !lines[j].trim().is_empty() {
                    break;
                }
            }
        }
    }

    if split_positions.is_empty() {
        // No email separator found — treat whole blob as one unit.
        let trimmed = md.trim_end().to_string();
        if trimmed.is_empty() {
            return Vec::new();
        }
        return vec![trimmed];
    }

    let mut pieces: Vec<String> = Vec::new();
    for (idx, &start) in split_positions.iter().enumerate() {
        let end = if idx + 1 < split_positions.len() {
            split_positions[idx + 1]
        } else {
            n
        };
        let piece_lines: Vec<&str> = lines[start..end].iter().copied().collect();
        let piece = piece_lines.join("\n").trim_end().to_string();
        if !piece.is_empty() {
            pieces.push(piece);
        }
    }

    pieces
}

/// Split `text` into pieces each ≤ `max_tokens` tokens.
///
/// Preference order for split boundaries:
/// 1. Paragraph (`\n\n`)
/// 2. Line (`\n`)
/// 3. Hard character cut (last resort; preserves UTF-8 code points)
pub(crate) fn split_by_token_budget(text: &str, max_tokens: u32) -> Vec<String> {
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

    fn meta_email() -> Metadata {
        Metadata::point_in_time(SourceKind::Email, "gmail:t1", "alice", Utc::now())
    }

    fn meta_doc() -> Metadata {
        Metadata::point_in_time(SourceKind::Document, "doc1", "alice", Utc::now())
    }

    #[test]
    fn tiny_input_produces_single_chunk() {
        // Chat input without a `## ` header produces one chunk via the empty-
        // result fallback (whole blob as one unit).
        let input = ChunkerInput {
            source_kind: SourceKind::Chat,
            source_id: "slack:#eng".into(),
            markdown: "## 2026-01-01T00:00:00Z — alice\nhello world".into(),
            metadata: meta(),
        };
        let chunks = chunk_markdown(&input, &ChunkerOptions::default());
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("hello world"));
        assert_eq!(chunks[0].seq_in_source, 0);
        assert!(!chunks[0].partial_message);
    }

    #[test]
    fn empty_chat_input_produces_one_empty_chunk() {
        let input = ChunkerInput {
            source_kind: SourceKind::Chat,
            source_id: "x".into(),
            markdown: "".into(),
            metadata: meta(),
        };
        let chunks = chunk_markdown(&input, &ChunkerOptions::default());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, "");
        assert!(!chunks[0].partial_message);
    }

    #[test]
    fn chat_messages_pack_into_one_chunk_when_small() {
        // Two small chat messages both fit under default max_tokens → greedy
        // packing emits ONE chunk containing both, joined by \n\n.
        let md = "## 2026-01-01T00:00:00Z — alice\nHello world\n\n## 2026-01-01T00:01:00Z — bob\nParagraph one.\n\nParagraph two.".to_string();
        let input = ChunkerInput {
            source_kind: SourceKind::Chat,
            source_id: "slack:#eng".into(),
            markdown: md.clone(),
            metadata: meta(),
        };
        let chunks = chunk_markdown(&input, &ChunkerOptions::default());
        // Both small messages fit under 10k tokens → one packed chunk.
        assert_eq!(
            chunks.len(),
            1,
            "small messages should be packed into one chunk; got {chunks:?}"
        );
        assert!(
            chunks[0].content.contains("alice"),
            "chunk must contain alice's message"
        );
        assert!(
            chunks[0].content.contains("bob"),
            "chunk must contain bob's message"
        );
        assert!(chunks[0].content.contains("Paragraph one."));
        assert!(chunks[0].content.contains("Paragraph two."));
        assert!(!chunks[0].partial_message);
    }

    #[test]
    fn chat_messages_split_at_boundary_when_large() {
        // Messages that together exceed max_tokens split at message boundaries
        // into multiple chunks. Each chunk contains whole messages only.
        // Each message is ~3k tokens at 4 chars/token = 12k chars;
        // two messages = ~6k tokens > 5k budget → must split.
        let msg_body = "x".repeat(12_000);
        let md = format!(
            "## 2026-01-01T00:00:00Z — alice\n{msg_body}\n\n## 2026-01-01T00:01:00Z — bob\n{msg_body}"
        );
        let input = ChunkerInput {
            source_kind: SourceKind::Chat,
            source_id: "slack:#eng".into(),
            markdown: md,
            metadata: meta(),
        };
        // Use a 5k token budget so two ~3k-token messages don't fit together.
        let chunks = chunk_markdown(&input, &ChunkerOptions { max_tokens: 5_000 });
        assert_eq!(
            chunks.len(),
            2,
            "two large messages should land in separate chunks; got {chunks:?}"
        );
        assert!(chunks[0].content.contains("alice"));
        assert!(chunks[1].content.contains("bob"));
        for c in &chunks {
            assert!(!c.partial_message, "whole messages must not be partial");
        }
    }

    #[test]
    fn email_threads_pack_into_one_chunk_when_small() {
        // Three short emails all fit under default max_tokens → one packed chunk.
        let md = "---\nFrom: alice@example.com\nSubject: Hello\nDate: 2026-01-01T00:00:00Z\n\nFirst body.\n---\nFrom: bob@example.com\nSubject: Re: Hello\nDate: 2026-01-01T00:01:00Z\n\nSecond body.\n---\nFrom: carol@example.com\nSubject: Re: Hello\nDate: 2026-01-01T00:02:00Z\n\nThird body.".to_string();
        let input = ChunkerInput {
            source_kind: SourceKind::Email,
            source_id: "gmail:t1".into(),
            markdown: md,
            metadata: meta_email(),
        };
        let chunks = chunk_markdown(&input, &ChunkerOptions::default());
        assert_eq!(
            chunks.len(),
            1,
            "three small emails should pack into one chunk; got {chunks:?}"
        );
        assert!(chunks[0].content.contains("First body."));
        assert!(chunks[0].content.contains("Second body."));
        assert!(chunks[0].content.contains("Third body."));
        assert!(!chunks[0].partial_message);
    }

    #[test]
    fn email_thread_large_splits_at_email_boundaries() {
        // Messages totaling >12k tokens split into 2 chunks at email boundaries.
        // Each email is ~4k tokens (16k chars); 3 emails × 4k = 12k tokens.
        // With a 5k budget, 2 emails fit per chunk → 2 chunks for 3 emails.
        let email_body = "y".repeat(16_000); // ~4k tokens
        let md = format!(
            "---\nFrom: a@x.com\nDate: 2026-01-01T00:00:00Z\n\n{email_body}\n\
             ---\nFrom: b@x.com\nDate: 2026-01-01T00:01:00Z\n\n{email_body}\n\
             ---\nFrom: c@x.com\nDate: 2026-01-01T00:02:00Z\n\n{email_body}"
        );
        let input = ChunkerInput {
            source_kind: SourceKind::Email,
            source_id: "gmail:t1".into(),
            markdown: md,
            metadata: meta_email(),
        };
        let chunks = chunk_markdown(&input, &ChunkerOptions { max_tokens: 5_000 });
        assert!(
            chunks.len() >= 2,
            "large thread must split into multiple chunks; got {}",
            chunks.len()
        );
        for c in &chunks {
            assert!(!c.partial_message, "whole-email chunks must not be partial");
        }
    }

    #[test]
    fn oversize_single_email_splits_with_partial_flag() {
        // A single email body > max_tokens must produce partial_message=true pieces.
        let big_body = "z".repeat(50_000); // ~12.5k tokens at 4 chars/token
        let md = format!("---\nFrom: a@x.com\nDate: 2026-01-01T00:00:00Z\n\n{big_body}");
        let input = ChunkerInput {
            source_kind: SourceKind::Email,
            source_id: "gmail:t1".into(),
            markdown: md,
            metadata: meta_email(),
        };
        let chunks = chunk_markdown(&input, &ChunkerOptions { max_tokens: 1_000 });
        assert!(chunks.len() > 1, "oversize email must split");
        for c in &chunks {
            assert!(
                c.partial_message,
                "all sub-pieces of an oversize email must have partial_message=true"
            );
        }
    }

    #[test]
    fn packed_units_joined_by_double_newline() {
        // Two chat messages packed together must be separated by \n\n.
        let md = "## 2026-01-01T00:00:00Z — alice\nfoo\n\n## 2026-01-01T00:01:00Z — bob\nbar"
            .to_string();
        let input = ChunkerInput {
            source_kind: SourceKind::Chat,
            source_id: "x".into(),
            markdown: md,
            metadata: meta(),
        };
        let chunks = chunk_markdown(&input, &ChunkerOptions::default());
        assert_eq!(chunks.len(), 1);
        // The two messages must be separated by \n\n in the packed content.
        assert!(
            chunks[0].content.contains("\n\n"),
            "packed units must be joined by \\n\\n; content={:?}",
            chunks[0].content
        );
    }

    #[test]
    fn oversize_message_falls_back_with_partial_flag() {
        // Single chat message that is way over max_tokens.
        let long_body = "x".repeat(8000); // ~2000 tokens at 4 chars/token
        let md = format!("## 2026-01-01T00:00:00Z — alice\n{long_body}");
        let input = ChunkerInput {
            source_kind: SourceKind::Chat,
            source_id: "x".into(),
            markdown: md,
            metadata: meta(),
        };
        let chunks = chunk_markdown(&input, &ChunkerOptions { max_tokens: 100 });
        assert!(chunks.len() > 1, "oversize message must split");
        for c in &chunks {
            assert!(
                c.partial_message,
                "all sub-pieces of an oversize message must have partial_message=true"
            );
        }
        // Reuniting all pieces must reconstruct the message content (minus `## ` line).
        let rejoined: String = chunks.iter().map(|c| c.content.as_str()).collect();
        assert!(rejoined.contains(&long_body[..100]));
    }

    #[test]
    fn document_falls_through_to_paragraph_split() {
        let para1 = "a".repeat(400); // ~100 tokens
        let para2 = "b".repeat(400);
        let para3 = "c".repeat(400);
        let text = format!("{para1}\n\n{para2}\n\n{para3}");
        let input = ChunkerInput {
            source_kind: SourceKind::Document,
            source_id: "doc1".into(),
            markdown: text,
            metadata: meta_doc(),
        };
        let chunks = chunk_markdown(
            &input,
            &ChunkerOptions {
                max_tokens: 150, // forces split at paragraph boundary
            },
        );
        assert!(chunks.len() >= 2);
        for c in &chunks {
            let first = c.content.chars().next().unwrap();
            assert!(
                matches!(first, 'a' | 'b' | 'c'),
                "document chunk starts with unexpected char: {:?}",
                c.content.chars().take(10).collect::<String>()
            );
            assert!(
                !c.partial_message,
                "document chunks must not have partial_message=true"
            );
        }
    }

    #[test]
    fn header_line_dropped_in_chat() {
        // Simulate a blob that has a leading `# Chat transcript` header.
        let md = "# Chat transcript — slack / #eng\n\n## 2026-01-01T00:00:00Z — alice\nhello"
            .to_string();
        let input = ChunkerInput {
            source_kind: SourceKind::Chat,
            source_id: "x".into(),
            markdown: md,
            metadata: meta(),
        };
        let chunks = chunk_markdown(&input, &ChunkerOptions::default());
        assert_eq!(chunks.len(), 1);
        // The `# Chat transcript` header must be absent from the chunk content.
        assert!(
            !chunks[0].content.contains("# Chat transcript"),
            "leading `# ` header must be dropped from chunk content"
        );
        assert!(chunks[0].content.contains("hello"));
    }

    #[test]
    fn chunk_ids_are_stable_across_runs() {
        let input = ChunkerInput {
            source_kind: SourceKind::Chat,
            source_id: "slack:#eng".into(),
            markdown: "## 2026-01-01T00:00:00Z — alice\nhello".into(),
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
        let msgs: String = (0..5)
            .map(|i| format!("## 2026-01-01T00:0{}:00Z — user{i}\nContent {i}\n\n", i))
            .collect();
        let input = ChunkerInput {
            source_kind: SourceKind::Chat,
            source_id: "x".into(),
            markdown: msgs,
            metadata: meta(),
        };
        let chunks = chunk_markdown(&input, &ChunkerOptions::default());
        for (idx, c) in chunks.iter().enumerate() {
            assert_eq!(c.seq_in_source, idx as u32);
        }
    }

    #[test]
    fn paragraph_boundaries_preferred_for_documents() {
        // Build something that exceeds token budget so it must split.
        let para1 = "a".repeat(400); // ~100 tokens
        let para2 = "b".repeat(400);
        let para3 = "c".repeat(400);
        let text = format!("{para1}\n\n{para2}\n\n{para3}");
        let input = ChunkerInput {
            source_kind: SourceKind::Document,
            source_id: "doc1".into(),
            markdown: text,
            metadata: meta_doc(),
        };
        let chunks = chunk_markdown(
            &input,
            &ChunkerOptions {
                max_tokens: 150, // forces split at paragraph
            },
        );
        assert!(chunks.len() >= 2);
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
    fn falls_back_to_line_split_when_no_paragraphs_document() {
        let text = (0..30)
            .map(|i| format!("line-{i}-{}", "x".repeat(40)))
            .collect::<Vec<_>>()
            .join("\n");
        let input = ChunkerInput {
            source_kind: SourceKind::Document,
            source_id: "x".into(),
            markdown: text,
            metadata: meta_doc(),
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
    fn utf8_boundaries_preserved_on_hard_split_document() {
        // Single long line with no paragraph/line splits → falls to hard cut.
        let text = "中".repeat(400);
        let input = ChunkerInput {
            source_kind: SourceKind::Document,
            source_id: "d".into(),
            markdown: text.clone(),
            metadata: meta_doc(),
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
    fn zero_token_budget_is_clamped_without_empty_leading_chunk_document() {
        let input = ChunkerInput {
            source_kind: SourceKind::Document,
            source_id: "d".into(),
            markdown: "abcdef".into(),
            metadata: meta_doc(),
        };
        let chunks = chunk_markdown(&input, &ChunkerOptions { max_tokens: 0 });
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|chunk| !chunk.content.is_empty()));
        let rejoined: String = chunks.iter().map(|c| c.content.as_str()).collect();
        assert_eq!(rejoined, "abcdef");
    }
}
