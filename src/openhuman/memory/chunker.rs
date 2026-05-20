//! Semantic markdown chunking for the OpenHuman memory system.
//!
//! This module provides the logic for splitting large markdown documents into
//! smaller, semantically meaningful chunks that fit within the context window
//! of an LLM or an embedding model. It prioritizes splitting on headings and
//! paragraph boundaries while preserving context by carrying over headings
//! to subsequent chunks.

use std::rc::Rc;

/// A single chunk of text extracted from a larger document.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// The zero-based index of this chunk within the original document.
    pub index: usize,
    /// The actual text content of the chunk.
    pub content: String,
    /// The most recent markdown heading that applies to this chunk's content.
    /// Uses `Rc<str>` for efficient sharing of the same heading across multiple chunks.
    pub heading: Option<Rc<str>>,
}

/// Splits markdown text into a sequence of [`Chunk`] objects.
///
/// Each chunk is designed to be approximately under the `max_tokens` limit.
/// The chunker uses a hierarchical splitting strategy:
/// 1. **Heading Boundaries**: Splits on `#`, `##`, and `###` headings.
/// 2. **Paragraph Boundaries**: If a heading section is too large, it splits on blank lines.
/// 3. **Line Boundaries**: If a paragraph is still too large, it splits on individual lines.
///
/// # Arguments
/// * `text` - The raw markdown text to chunk.
/// * `max_tokens` - The approximate maximum number of tokens per chunk (estimated at 4 chars/token).
///
/// # Returns
/// A vector of [`Chunk`] structs representing the document.
pub fn chunk_markdown(text: &str, max_tokens: usize) -> Vec<Chunk> {
    if text.trim().is_empty() {
        return Vec::new();
    }

    // Rough estimation: 4 characters per token for English text.
    let max_chars = max_tokens * 4;

    // Step 1: Divide the document into top-level sections based on headings.
    let sections = split_on_headings(text);
    let mut chunks = Vec::with_capacity(sections.len());

    for (heading, body) in sections {
        let heading: Option<Rc<str>> = heading.map(Rc::from);
        let heading_prefix = heading.as_deref().map(|h| {
            let mut prefix = String::with_capacity(h.len() + 1);
            prefix.push_str(h);
            prefix.push('\n');
            prefix
        });

        let full_len = body.len() + heading_prefix.as_ref().map_or(0, String::len);

        if full_len <= max_chars {
            // Section fits entirely in one chunk.
            let content = if let Some(prefix) = heading_prefix.as_deref() {
                let mut full = String::with_capacity(full_len);
                full.push_str(prefix);
                full.push_str(&body);
                full.trim().to_string()
            } else {
                body.trim().to_string()
            };
            chunks.push(Chunk {
                index: chunks.len(),
                content,
                heading: heading.clone(),
            });
        } else {
            // Step 2: Section is too large; split into paragraphs.
            let paragraphs = split_on_blank_lines(&body);
            let mut current = heading_prefix.clone().unwrap_or_default();

            for para in paragraphs {
                // If adding this paragraph exceeds the limit, emit the current chunk.
                if current.len() + para.len() > max_chars && !current.trim().is_empty() {
                    chunks.push(Chunk {
                        index: chunks.len(),
                        content: current.trim().to_string(),
                        heading: heading.clone(),
                    });
                    // Reset with the heading for context preservation.
                    reset_chunk_buffer(&mut current, heading_prefix.as_deref());
                }

                if para.len() > max_chars {
                    // Step 3: Paragraph is still too large; split it line-by-line.
                    if !current.trim().is_empty() {
                        chunks.push(Chunk {
                            index: chunks.len(),
                            content: current.trim().to_string(),
                            heading: heading.clone(),
                        });
                        reset_chunk_buffer(&mut current, heading_prefix.as_deref());
                    }
                    for line_chunk in split_on_lines(&para, max_chars) {
                        chunks.push(Chunk {
                            index: chunks.len(),
                            content: line_chunk.trim().to_string(),
                            heading: heading.clone(),
                        });
                    }
                } else {
                    current.push_str(&para);
                    current.push('\n');
                }
            }

            // Emit any remaining content as a final chunk for this section.
            if !current.trim().is_empty() {
                chunks.push(Chunk {
                    index: chunks.len(),
                    content: current.trim().to_string(),
                    heading: heading.clone(),
                });
            }
        }
    }

    // Clean up empty chunks and normalize indices.
    chunks.retain(|c| !c.content.is_empty());

    for (i, chunk) in chunks.iter_mut().enumerate() {
        chunk.index = i;
    }

    chunks
}

fn reset_chunk_buffer(current: &mut String, heading_prefix: Option<&str>) {
    current.clear();
    if let Some(prefix) = heading_prefix {
        current.push_str(prefix);
    }
}

/// Returns `true` if `line` starts with a valid ATX markdown heading
/// (1 to 6 `#` characters followed by a space).
fn is_atx_heading(line: &str) -> bool {
    const PREFIXES: &[&str] = &["# ", "## ", "### ", "#### ", "##### ", "###### "];
    PREFIXES.iter().any(|p| line.starts_with(p))
}

/// Identifies markdown ATX headings and groups their following text into
/// sections.
fn split_on_headings(text: &str) -> Vec<(Option<String>, String)> {
    log::debug!(
        "[memory::chunker] split_on_headings: entry text_len={}",
        text.len()
    );
    let mut sections = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut current_body = String::new();

    // Log lengths only, not heading or body text: a heading could contain
    // PII the user pasted into their docs (emails, usernames, etc.).
    for line in text.lines() {
        if is_atx_heading(line) {
            if !current_body.trim().is_empty() || current_heading.is_some() {
                log::debug!(
                    "[memory::chunker] split_on_headings: flushing section heading_len={} body_len={}",
                    current_heading.as_deref().map(str::len).unwrap_or(0),
                    current_body.len()
                );
                sections.push((current_heading.take(), std::mem::take(&mut current_body)));
            }
            current_heading = Some(line.to_string());
        } else {
            current_body.push_str(line);
            current_body.push('\n');
        }
    }

    if !current_body.trim().is_empty() || current_heading.is_some() {
        log::debug!(
            "[memory::chunker] split_on_headings: flushing final section heading_len={} body_len={}",
            current_heading.as_deref().map(str::len).unwrap_or(0),
            current_body.len()
        );
        sections.push((current_heading, current_body));
    }

    log::debug!(
        "[memory::chunker] split_on_headings: exit sections={}",
        sections.len()
    );
    sections
}

/// Splits text into strings based on blank line (paragraph) boundaries.
fn split_on_blank_lines(text: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        if line.trim().is_empty() {
            if !current.trim().is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }

    if !current.trim().is_empty() {
        paragraphs.push(current);
    }

    paragraphs
}

/// Splits text into chunks based on line boundaries to ensure size constraints.
/// Lines exceeding `max_chars` are further split on word boundaries.
fn split_on_lines(text: &str, max_chars: usize) -> Vec<String> {
    let effective_max = max_chars.max(1);
    let mut chunks = Vec::with_capacity(text.len() / effective_max + 1);
    let mut current = String::new();

    log::trace!(
        "[memory::chunker] split_on_lines: entry text_len={} max_chars={}",
        text.len(),
        effective_max
    );

    for line in text.lines() {
        if line.len() > effective_max {
            log::debug!(
                "[memory::chunker] split_on_lines: oversize line detected line_len={} max_chars={}",
                line.len(),
                effective_max
            );
            // Flush anything accumulated before the oversize line.
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            // Split the oversize line itself on word boundaries.
            for part in split_within_line(line, effective_max) {
                chunks.push(part);
            }
        } else if current.len() + line.len() + 1 > effective_max && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
            current.push_str(line);
            current.push('\n');
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

/// Splits a single oversize line into chunks of at most `max_chars`, preferring
/// word boundaries (spaces) to avoid cutting mid-word. Falls back to hard
/// character splits when no boundary exists within the limit.
fn split_within_line(line: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let bytes = line.as_bytes();

    log::trace!(
        "[memory::chunker] split_within_line: entry line_len={} max_chars={}",
        line.len(),
        max_chars
    );

    while start < line.len() {
        let remaining = line.len() - start;
        if remaining <= max_chars {
            chunks.push(format!("{}\n", &line[start..]));
            break;
        }

        // Find the end boundary, staying on a valid char boundary.
        let mut end = start + max_chars;
        // Walk back to a valid UTF-8 char boundary.
        while end > start && !line.is_char_boundary(end) {
            end -= 1;
        }

        // If max_chars is smaller than the next character (e.g., a 4-byte emoji
        // with max_chars=1), `end` can equal `start`. Advance to the next char
        // boundary to guarantee progress and avoid an infinite loop.
        if end == start {
            end = start + 1;
            while end < line.len() && !line.is_char_boundary(end) {
                end += 1;
            }
            log::debug!(
                "[memory::chunker] split_within_line: forced advance past multi-byte char start={} end={}",
                start, end
            );
        }

        // Try to find a space to break on (scan backwards from `end`).
        let mut split_at = end;
        while split_at > start && bytes[split_at - 1] != b' ' {
            split_at -= 1;
        }

        // If we couldn't find a space within the range, hard-split at `end`.
        if split_at == start {
            split_at = end;
            log::debug!(
                "[memory::chunker] split_within_line: hard split at {} (no word boundary)",
                split_at
            );
        }

        chunks.push(format!("{}\n", &line[start..split_at]));
        // Skip the space we split on (if it was a space).
        if split_at < line.len() && bytes[split_at] == b' ' {
            start = split_at + 1;
        } else {
            start = split_at;
        }
    }

    log::trace!(
        "[memory::chunker] split_within_line: exit parts={}",
        chunks.len()
    );
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text() {
        assert!(chunk_markdown("", 512).is_empty());
        assert!(chunk_markdown("   ", 512).is_empty());
    }

    #[test]
    fn single_short_paragraph() {
        let chunks = chunk_markdown("Hello world", 512);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, "Hello world");
        assert!(chunks[0].heading.is_none());
    }

    #[test]
    fn heading_sections() {
        let text = "# Title\nSome intro.\n\n## Section A\nContent A.\n\n## Section B\nContent B.";
        let chunks = chunk_markdown(text, 512);
        assert!(chunks.len() >= 3);
        assert!(chunks[0].heading.is_none() || chunks[0].heading.as_deref() == Some("# Title"));
    }

    #[test]
    fn respects_max_tokens() {
        // Build multi-line text (one sentence per line) to exercise line-level splitting
        let long_text: String = (0..200).fold(String::new(), |mut s, i| {
            use std::fmt::Write;
            let _ = writeln!(
                s,
                "This is sentence number {i} with some extra words to fill it up."
            );
            s
        });
        let chunks = chunk_markdown(&long_text, 50); // 50 tokens ≈ 200 chars
        assert!(
            chunks.len() > 1,
            "Expected multiple chunks, got {}",
            chunks.len()
        );
        for chunk in &chunks {
            // Allow some slack (heading re-insertion etc.)
            assert!(
                chunk.content.len() <= 300,
                "Chunk too long: {} chars",
                chunk.content.len()
            );
        }
    }

    #[test]
    fn preserves_heading_in_split_sections() {
        let mut text = String::from("## Big Section\n");
        for i in 0..100 {
            use std::fmt::Write;
            let _ = write!(text, "Line {i} with some content here.\n\n");
        }
        let chunks = chunk_markdown(&text, 50);
        assert!(chunks.len() > 1);
        // All chunks from this section should reference the heading
        for chunk in &chunks {
            if chunk.heading.is_some() {
                assert_eq!(chunk.heading.as_deref(), Some("## Big Section"));
            }
        }
    }

    #[test]
    fn indexes_are_sequential() {
        let text = "# A\nContent A\n\n# B\nContent B\n\n# C\nContent C";
        let chunks = chunk_markdown(text, 512);
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.index, i);
        }
    }

    #[test]
    fn chunk_count_reasonable() {
        let text = "Hello world. This is a test document.";
        let chunks = chunk_markdown(text, 512);
        assert_eq!(chunks.len(), 1);
    }

    // ── Edge cases ───────────────────────────────────────────────

    #[test]
    fn headings_only_no_body() {
        let text = "# Title\n## Section A\n## Section B\n### Subsection";
        let chunks = chunk_markdown(text, 512);
        // Should produce chunks for each heading (even with empty bodies)
        assert!(!chunks.is_empty());
    }

    #[test]
    fn deep_atx_headings_split_through_h6() {
        let text = "# Top\nIntro\n#### Deep heading\nDeep content";
        let chunks = chunk_markdown(text, 512);
        assert!(
            chunks.len() >= 2,
            "expected the #### heading to start a new section, got {} chunk(s)",
            chunks.len(),
        );
        let deep = chunks
            .iter()
            .find(|c| c.heading.as_deref() == Some("#### Deep heading"));
        assert!(
            deep.is_some(),
            "expected a chunk with heading '#### Deep heading'; chunks: {chunks:?}",
        );
    }

    #[test]
    fn all_atx_heading_levels_h1_through_h6_split() {
        let text = "# H1\na\n\n## H2\nb\n\n### H3\nc\n\n#### H4\nd\n\n##### H5\ne\n\n###### H6\nf";
        let chunks = chunk_markdown(text, 512);
        let headings: Vec<_> = chunks.iter().filter_map(|c| c.heading.as_deref()).collect();
        assert_eq!(
            headings,
            vec![
                "# H1",
                "## H2",
                "### H3",
                "#### H4",
                "##### H5",
                "###### H6"
            ],
            "each ATX heading depth h1-h6 must split into its own section",
        );
    }

    #[test]
    fn seven_or_more_hashes_are_not_a_heading() {
        let text = "# Top\nIntro\n####### Not a heading\nMore content";
        let chunks = chunk_markdown(text, 512);
        assert_eq!(
            chunks.len(),
            1,
            "7-hash line should not split; expected 1 chunk, got {}",
            chunks.len(),
        );
        assert_eq!(chunks[0].heading.as_deref(), Some("# Top"));
        assert!(chunks[0].content.contains("####### Not a heading"));
    }

    #[test]
    fn atx_heading_requires_trailing_space() {
        let text = "# Real heading\nIntro\n###NoSpace\nbody";
        let chunks = chunk_markdown(text, 512);
        assert_eq!(
            chunks.len(),
            1,
            "missing trailing space disqualifies the heading"
        );
        assert_eq!(chunks[0].heading.as_deref(), Some("# Real heading"));
    }

    #[test]
    fn very_long_single_line_no_newlines() {
        // One giant line with no newlines — must split within the line
        let text = "word ".repeat(5000); // 25000 chars
        let max_tokens = 50; // 200 chars
        let max_chars = max_tokens * 4;
        let chunks = chunk_markdown(&text, max_tokens);
        assert!(
            chunks.len() > 1,
            "Expected multiple chunks for a 25KB single-line input, got {}",
            chunks.len()
        );
        for chunk in &chunks {
            // Each chunk must respect the size limit (with reasonable slack for
            // trailing newline and word overshoot).
            assert!(
                chunk.content.len() <= max_chars + 50,
                "Chunk exceeds max_chars: {} chars (limit {})",
                chunk.content.len(),
                max_chars
            );
        }
    }

    #[test]
    fn oversize_line_splits_on_word_boundary() {
        // A single line of 100 words, each 5 chars + space = 600 chars
        let line = "abcde ".repeat(100);
        let text = format!("# Heading\n{line}");
        let chunks = chunk_markdown(&text, 25); // 25 tokens = 100 chars max
        assert!(chunks.len() > 1);
        // Verify no chunk contains a split mid-word
        for chunk in &chunks {
            // Words should be intact (no "abc\nde" splits)
            for word in chunk.content.split_whitespace() {
                if word.starts_with('#') {
                    continue; // heading
                }
                assert!(
                    word == "abcde" || word == "Heading",
                    "Unexpected split word: '{word}'"
                );
            }
        }
    }

    #[test]
    fn oversize_line_no_spaces_hard_splits() {
        // A single line with no word boundaries at all
        let text = "x".repeat(1000);
        let chunks = chunk_markdown(&text, 25); // 100 chars max
        assert!(
            chunks.len() > 1,
            "Should hard-split when no spaces exist, got {} chunk(s)",
            chunks.len()
        );
        // Reconstruct and verify no data loss
        let reassembled: String = chunks.iter().map(|c| c.content.trim()).collect();
        assert_eq!(reassembled.len(), 1000);
    }

    #[test]
    fn only_newlines_and_whitespace() {
        assert!(chunk_markdown("\n\n\n   \n\n", 512).is_empty());
    }

    #[test]
    fn max_tokens_zero() {
        // max_tokens=0 → max_chars=0, should not panic or infinite loop
        let chunks = chunk_markdown("Hello world", 0);
        // Every chunk will exceed 0 chars, so it splits maximally
        assert!(!chunks.is_empty());
    }

    #[test]
    fn max_tokens_one() {
        // max_tokens=1 → max_chars=4, very aggressive splitting
        let text = "Line one\nLine two\nLine three";
        let chunks = chunk_markdown(text, 1);
        assert!(!chunks.is_empty());
    }

    #[test]
    fn unicode_content() {
        let text = "# 日本語\nこんにちは世界\n\n## Émojis\n🦀 Rust is great 🚀";
        let chunks = chunk_markdown(text, 512);
        assert!(!chunks.is_empty());
        let all: String = chunks.iter().map(|c| c.content.clone()).collect();
        assert!(all.contains("こんにちは"));
        assert!(all.contains("🦀"));
    }

    #[test]
    fn fts5_special_chars_in_content() {
        let text = "Content with \"quotes\" and (parentheses) and * asterisks *";
        let chunks = chunk_markdown(text, 512);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("\"quotes\""));
    }

    #[test]
    fn multiple_blank_lines_between_paragraphs() {
        let text = "Paragraph one.\n\n\n\n\nParagraph two.\n\n\n\nParagraph three.";
        let chunks = chunk_markdown(text, 512);
        assert_eq!(chunks.len(), 1); // All fits in one chunk
        assert!(chunks[0].content.contains("Paragraph one"));
        assert!(chunks[0].content.contains("Paragraph three"));
    }

    #[test]
    fn heading_at_end_of_text() {
        let text = "Some content\n# Trailing Heading";
        let chunks = chunk_markdown(text, 512);
        assert!(!chunks.is_empty());
    }

    #[test]
    fn single_heading_no_content() {
        let text = "# Just a heading";
        let chunks = chunk_markdown(text, 512);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].heading.as_deref(), Some("# Just a heading"));
    }

    #[test]
    fn no_content_loss() {
        let text = "# A\nContent A line 1\nContent A line 2\n\n## B\nContent B\n\n## C\nContent C";
        let chunks = chunk_markdown(text, 512);
        let reassembled: String = chunks.iter().fold(String::new(), |mut s, c| {
            use std::fmt::Write;
            let _ = writeln!(s, "{}", c.content);
            s
        });
        // All original content words should appear
        for word in ["Content", "line", "1", "2"] {
            assert!(
                reassembled.contains(word),
                "Missing word '{word}' in reassembled chunks"
            );
        }
    }
}
