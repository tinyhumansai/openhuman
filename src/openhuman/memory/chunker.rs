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
        
        // Combine heading and body to check initial size.
        let full = if let Some(ref h) = heading {
            format!("{h}\n{body}")
        } else {
            body.clone()
        };

        if full.len() <= max_chars {
            // Section fits entirely in one chunk.
            chunks.push(Chunk {
                index: chunks.len(),
                content: full.trim().to_string(),
                heading: heading.clone(),
            });
        } else {
            // Step 2: Section is too large; split into paragraphs.
            let paragraphs = split_on_blank_lines(&body);
            let mut current = heading
                .as_deref()
                .map_or_else(String::new, |h| format!("{h}\n"));

            for para in paragraphs {
                // If adding this paragraph exceeds the limit, emit the current chunk.
                if current.len() + para.len() > max_chars && !current.trim().is_empty() {
                    chunks.push(Chunk {
                        index: chunks.len(),
                        content: current.trim().to_string(),
                        heading: heading.clone(),
                    });
                    // Reset with the heading for context preservation.
                    current = heading
                        .as_deref()
                        .map_or_else(String::new, |h| format!("{h}\n"));
                }

                if para.len() > max_chars {
                    // Step 3: Paragraph is still too large; split it line-by-line.
                    if !current.trim().is_empty() {
                        chunks.push(Chunk {
                            index: chunks.len(),
                            content: current.trim().to_string(),
                            heading: heading.clone(),
                        });
                        current = heading
                            .as_deref()
                            .map_or_else(String::new, |h| format!("{h}\n"));
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

/// Identifies top-level markdown headings and groups their following text.
///
/// Recognizes `#`, `##`, and `###` as section boundaries.
fn split_on_headings(text: &str) -> Vec<(Option<String>, String)> {
    let mut sections = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut current_body = String::new();

    for line in text.lines() {
        if line.starts_with("# ") || line.starts_with("## ") || line.starts_with("### ") {
            if !current_body.trim().is_empty() || current_heading.is_some() {
                sections.push((current_heading.take(), std::mem::take(&mut current_body)));
            }
            current_heading = Some(line.to_string());
        } else {
            current_body.push_str(line);
            current_body.push('\n');
        }
    }

    if !current_body.trim().is_empty() || current_heading.is_some() {
        sections.push((current_heading, current_body));
    }

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
fn split_on_lines(text: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::with_capacity(text.len() / max_chars.max(1) + 1);
    let mut current = String::new();

    for line in text.lines() {
        // If the current line itself is larger than max_chars, it will be added anyway.
        // We don't currently split *within* a single line.
        if current.len() + line.len() + 1 > max_chars && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
        }
        current.push_str(line);
        current.push('\n');
    }

    if !current.is_empty() {
        chunks.push(current);
    }

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
    fn deeply_nested_headings_ignored() {
        // #### and deeper are NOT treated as heading splits
        let text = "# Top\nIntro\n#### Deep heading\nDeep content";
        let chunks = chunk_markdown(text, 512);
        // "#### Deep heading" should stay with its parent section
        assert!(!chunks.is_empty());
        let all_content: String = chunks.iter().map(|c| c.content.clone()).collect();
        assert!(all_content.contains("Deep heading"));
        assert!(all_content.contains("Deep content"));
    }

    #[test]
    fn very_long_single_line_no_newlines() {
        // One giant line with no newlines — can't split on lines effectively
        let text = "word ".repeat(5000);
        let chunks = chunk_markdown(&text, 50);
        // Should produce at least 1 chunk without panicking
        assert!(!chunks.is_empty());
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
