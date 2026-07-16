//! Typed input / output / error contracts for the `generate_document` tool.
//!
//! Deliberately mirrors the `generate_presentation` contracts
//! (`presentation::types`) so the two artifact producers stay
//! structurally parallel: same validate-early ethos, same size caps
//! pattern, same structured `InvalidInput` the agent can self-correct
//! on. Where the presentation tool models a deck of `slides`, the
//! document tool models a linear body of ordered `sections`.

use serde::{Deserialize, Serialize};

/// Maximum number of sections a single `generate_document` call may
/// produce. Hard cap to bound generation time + output size; the LLM is
/// asked to break larger documents into multiple calls.
pub(super) const MAX_SECTIONS: usize = 128;

/// Maximum length of a single short text field (title, author, section
/// heading). Bounds the payload handed to the `docx-rs` engine.
pub(super) const MAX_TEXT_CHARS: usize = 2_000;

/// Maximum length of a single body paragraph. Prose paragraphs run
/// longer than a slide's bullet, so this cap is more generous than
/// [`MAX_TEXT_CHARS`] while still bounding the worst-case payload.
pub(super) const MAX_PARAGRAPH_CHARS: usize = 20_000;

/// Maximum number of body paragraphs in a single section. Beyond this a
/// section should be split; keeps one `execute` call bounded.
pub(super) const MAX_PARAGRAPHS_PER_SECTION: usize = 200;

/// Maximum number of bullet-list items in a single section.
pub(super) const MAX_BULLETS_PER_SECTION: usize = 200;

/// Aggregate cap on the total body text (title + author + every heading,
/// paragraph, and bullet) across the whole document, in Unicode scalar
/// values. The per-field/per-section limits above bound each piece, but
/// their product (`MAX_SECTIONS × MAX_PARAGRAPHS_PER_SECTION ×
/// MAX_PARAGRAPH_CHARS` alone is ~512M chars) would still let a single
/// valid request build a multi-hundred-megabyte DOCX in memory. This
/// checked total keeps the worst-case in-memory payload bounded to a few
/// megabytes of text while remaining generous for any real document.
pub(super) const MAX_TOTAL_CHARS: usize = 2_000_000;

/// One section of the document, rendered in input order. A section is a
/// heading (optional) followed by any number of body paragraphs and/or a
/// bullet list. At least one of the three must be populated — a wholly
/// empty section is rejected by [`validate_input`] rather than rendering
/// a blank run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentSection {
    /// Section heading, rendered as a bold heading paragraph. Optional —
    /// a section may be pure body/bullets under the document title.
    #[serde(default)]
    pub heading: Option<String>,
    /// Body paragraphs, each rendered as its own normal paragraph in
    /// order. Empty / whitespace-only entries are dropped by the engine.
    #[serde(default)]
    pub paragraphs: Vec<String>,
    /// Bullet-list items, rendered as a single-level `•` bulleted list
    /// after the section's body paragraphs. Empty / whitespace-only
    /// entries are dropped by the engine.
    #[serde(default)]
    pub bullets: Vec<String>,
}

impl DocumentSection {
    /// `true` when the section carries no renderable content at all
    /// (heading blank/absent, and every paragraph/bullet blank). Used by
    /// [`validate_input`] to reject empty sections the same way the
    /// presentation tool rejects an empty slide.
    pub(super) fn is_empty(&self) -> bool {
        let has_heading = self
            .heading
            .as_deref()
            .map(|h| !h.trim().is_empty())
            .unwrap_or(false);
        let has_paragraph = self.paragraphs.iter().any(|p| !p.trim().is_empty());
        let has_bullet = self.bullets.iter().any(|b| !b.trim().is_empty());
        !(has_heading || has_paragraph || has_bullet)
    }
}

/// Top-level input for the `generate_document` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateDocumentInput {
    /// Document title. Surfaces as the leading title paragraph and as the
    /// artifact's human-readable name. Required, non-empty.
    pub title: String,
    /// Optional author byline, rendered as an italic line beneath the
    /// title.
    #[serde(default)]
    pub author: Option<String>,
    /// Section specs, in display order. Must contain at least one entry.
    #[serde(default)]
    pub sections: Vec<DocumentSection>,
}

/// Tool output returned via [`crate::openhuman::tools::traits::ToolResult`]
/// as the JSON `data` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateDocumentOutput {
    /// UUID of the persisted artifact record. Use with the
    /// `ai_get_artifact` / `ai_delete_artifact` RPCs.
    pub artifact_id: String,
    /// Absolute filesystem path to the generated `.docx`.
    pub artifact_path: String,
    /// Number of sections rendered into the document body.
    pub section_count: usize,
    /// On-disk size of the produced `.docx` in bytes.
    pub size_bytes: u64,
}

/// Structured error variants surfaced to the agent. Mirrors
/// [`presentation::types::PresentationError`](super::super::presentation)
/// so downstream error handling stays uniform across artifact producers.
#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    #[error("invalid input for field '{field}': {reason}")]
    InvalidInput { field: String, reason: String },

    #[error("document generation failed: {stderr_truncated}")]
    GenerationFailed { stderr_truncated: String },

    #[error("document generation exceeded {timeout_secs}s timeout")]
    GenerationTimeout { timeout_secs: u64 },
}

impl DocumentError {
    /// Truncate a library-error string to a 500-char cap (UTF-8-safe) so
    /// the variant never carries an unbounded payload back to the agent.
    /// Same cap/suffix as the presentation tool's `truncate_stderr`.
    pub(super) fn truncate_stderr(raw: &str) -> String {
        const MAX: usize = 500;
        const SUFFIX: &str = " […truncated]";
        let total = raw.chars().count();
        if total <= MAX {
            return raw.to_string();
        }
        let keep = MAX.saturating_sub(SUFFIX.chars().count());
        let mut out: String = raw.chars().take(keep).collect();
        out.push_str(SUFFIX);
        out
    }
}

/// Validate the input early — before invoking the `docx-rs` engine — so
/// the agent gets a structured `InvalidInput` it can self-correct on
/// instead of a generic engine error.
pub(super) fn validate_input(input: &GenerateDocumentInput) -> Result<(), DocumentError> {
    if input.title.trim().is_empty() {
        return Err(DocumentError::InvalidInput {
            field: "title".to_string(),
            reason: "must not be empty".to_string(),
        });
    }
    if input.title.chars().count() > MAX_TEXT_CHARS {
        return Err(DocumentError::InvalidInput {
            field: "title".to_string(),
            reason: format!("must be ≤ {MAX_TEXT_CHARS} chars"),
        });
    }
    if let Some(author) = input.author.as_deref() {
        if author.chars().count() > MAX_TEXT_CHARS {
            return Err(DocumentError::InvalidInput {
                field: "author".to_string(),
                reason: format!("must be ≤ {MAX_TEXT_CHARS} chars"),
            });
        }
    }
    if input.sections.is_empty() {
        return Err(DocumentError::InvalidInput {
            field: "sections".to_string(),
            reason: "must contain at least one section".to_string(),
        });
    }
    if input.sections.len() > MAX_SECTIONS {
        return Err(DocumentError::InvalidInput {
            field: "sections".to_string(),
            reason: format!("must contain ≤ {MAX_SECTIONS} sections"),
        });
    }
    for (i, section) in input.sections.iter().enumerate() {
        // Reject wholly-empty sections: the engine trims + drops empty
        // paragraph/bullet entries, so a section with only ["   "] would
        // render blank despite carrying entries.
        if section.is_empty() {
            return Err(DocumentError::InvalidInput {
                field: format!("sections[{i}]"),
                reason: "must have at least one of heading / paragraphs / bullets".to_string(),
            });
        }
        if let Some(heading) = section.heading.as_deref() {
            if heading.chars().count() > MAX_TEXT_CHARS {
                return Err(DocumentError::InvalidInput {
                    field: format!("sections[{i}].heading"),
                    reason: format!("must be ≤ {MAX_TEXT_CHARS} chars"),
                });
            }
        }
        if section.paragraphs.len() > MAX_PARAGRAPHS_PER_SECTION {
            return Err(DocumentError::InvalidInput {
                field: format!("sections[{i}].paragraphs"),
                reason: format!("must contain ≤ {MAX_PARAGRAPHS_PER_SECTION} paragraphs"),
            });
        }
        for (p, paragraph) in section.paragraphs.iter().enumerate() {
            if paragraph.chars().count() > MAX_PARAGRAPH_CHARS {
                return Err(DocumentError::InvalidInput {
                    field: format!("sections[{i}].paragraphs[{p}]"),
                    reason: format!("must be ≤ {MAX_PARAGRAPH_CHARS} chars"),
                });
            }
        }
        if section.bullets.len() > MAX_BULLETS_PER_SECTION {
            return Err(DocumentError::InvalidInput {
                field: format!("sections[{i}].bullets"),
                reason: format!("must contain ≤ {MAX_BULLETS_PER_SECTION} bullets"),
            });
        }
        for (b, bullet) in section.bullets.iter().enumerate() {
            if bullet.chars().count() > MAX_PARAGRAPH_CHARS {
                return Err(DocumentError::InvalidInput {
                    field: format!("sections[{i}].bullets[{b}]"),
                    reason: format!("must be ≤ {MAX_PARAGRAPH_CHARS} chars"),
                });
            }
        }
    }
    // Aggregate budget: the per-field caps above bound each piece, but not
    // their sum, so a valid request could still assemble a huge in-memory DOCX.
    // Sum every text field with saturating arithmetic (can't overflow) and
    // reject once the whole document exceeds MAX_TOTAL_CHARS.
    let mut total_chars = input.title.chars().count();
    if let Some(author) = input.author.as_deref() {
        total_chars = total_chars.saturating_add(author.chars().count());
    }
    for section in &input.sections {
        if let Some(heading) = section.heading.as_deref() {
            total_chars = total_chars.saturating_add(heading.chars().count());
        }
        for paragraph in &section.paragraphs {
            total_chars = total_chars.saturating_add(paragraph.chars().count());
        }
        for bullet in &section.bullets {
            total_chars = total_chars.saturating_add(bullet.chars().count());
        }
    }
    if total_chars > MAX_TOTAL_CHARS {
        return Err(DocumentError::InvalidInput {
            field: "sections".to_string(),
            reason: format!("total document text must be ≤ {MAX_TOTAL_CHARS} chars"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One valid section carrying a heading + a paragraph + a bullet.
    fn section() -> DocumentSection {
        DocumentSection {
            heading: Some("Overview".to_string()),
            paragraphs: vec!["A body paragraph.".to_string()],
            bullets: vec!["A bullet".to_string()],
        }
    }

    /// A minimal valid input; individual tests mutate one field to drive a
    /// single rejection branch.
    fn base() -> GenerateDocumentInput {
        GenerateDocumentInput {
            title: "Charter".to_string(),
            author: Some("Alice".to_string()),
            sections: vec![section()],
        }
    }

    /// Assert `validate_input` rejects `input` naming `field` in the error.
    fn assert_rejects(input: &GenerateDocumentInput, field: &str) {
        match validate_input(input) {
            Err(DocumentError::InvalidInput { field: f, .. }) => {
                assert!(
                    f.contains(field),
                    "expected error field to contain {field:?}, got {f:?}"
                );
            }
            other => panic!("expected InvalidInput({field}), got {other:?}"),
        }
    }

    #[test]
    fn accepts_a_well_formed_input() {
        assert!(validate_input(&base()).is_ok());
    }

    #[test]
    fn is_empty_reflects_content_presence() {
        assert!(!section().is_empty());
        assert!(DocumentSection {
            heading: Some("   ".to_string()),
            paragraphs: vec!["  ".to_string(), String::new()],
            bullets: vec!["\t".to_string()],
        }
        .is_empty());
        // Any one populated field is enough.
        assert!(!DocumentSection {
            heading: None,
            paragraphs: vec![],
            bullets: vec!["x".to_string()],
        }
        .is_empty());
    }

    #[test]
    fn rejects_empty_title() {
        let mut i = base();
        i.title = "   ".to_string();
        assert_rejects(&i, "title");
    }

    #[test]
    fn rejects_oversize_title() {
        let mut i = base();
        i.title = "t".repeat(MAX_TEXT_CHARS + 1);
        assert_rejects(&i, "title");
    }

    #[test]
    fn rejects_oversize_author() {
        let mut i = base();
        i.author = Some("a".repeat(MAX_TEXT_CHARS + 1));
        assert_rejects(&i, "author");
    }

    #[test]
    fn rejects_no_sections() {
        let mut i = base();
        i.sections = vec![];
        assert_rejects(&i, "sections");
    }

    #[test]
    fn rejects_too_many_sections() {
        let mut i = base();
        i.sections = (0..(MAX_SECTIONS + 1)).map(|_| section()).collect();
        assert_rejects(&i, "sections");
    }

    #[test]
    fn rejects_empty_section() {
        let mut i = base();
        i.sections = vec![DocumentSection {
            heading: Some("  ".to_string()),
            paragraphs: vec![" ".to_string()],
            bullets: vec![],
        }];
        assert_rejects(&i, "sections[0]");
    }

    #[test]
    fn rejects_oversize_heading() {
        let mut i = base();
        i.sections[0].heading = Some("h".repeat(MAX_TEXT_CHARS + 1));
        assert_rejects(&i, "sections[0].heading");
    }

    #[test]
    fn rejects_too_many_paragraphs() {
        let mut i = base();
        i.sections[0].paragraphs = (0..(MAX_PARAGRAPHS_PER_SECTION + 1))
            .map(|n| format!("p{n}"))
            .collect();
        assert_rejects(&i, "sections[0].paragraphs");
    }

    #[test]
    fn rejects_oversize_paragraph() {
        let mut i = base();
        i.sections[0].paragraphs = vec!["p".repeat(MAX_PARAGRAPH_CHARS + 1)];
        assert_rejects(&i, "sections[0].paragraphs[0]");
    }

    #[test]
    fn rejects_too_many_bullets() {
        let mut i = base();
        i.sections[0].bullets = (0..(MAX_BULLETS_PER_SECTION + 1))
            .map(|n| format!("b{n}"))
            .collect();
        assert_rejects(&i, "sections[0].bullets");
    }

    #[test]
    fn rejects_oversize_bullet() {
        let mut i = base();
        i.sections[0].bullets = vec!["b".repeat(MAX_PARAGRAPH_CHARS + 1)];
        assert_rejects(&i, "sections[0].bullets[0]");
    }

    #[test]
    fn accepts_document_exactly_at_total_char_budget() {
        // 99 full paragraphs + one one-short paragraph + a 1-char title sum to
        // exactly MAX_TOTAL_CHARS, while every field stays within its own cap.
        let mut paragraphs: Vec<String> =
            (0..99).map(|_| "p".repeat(MAX_PARAGRAPH_CHARS)).collect();
        paragraphs.push("p".repeat(MAX_PARAGRAPH_CHARS - 1));
        assert_eq!(
            1 + 99 * MAX_PARAGRAPH_CHARS + (MAX_PARAGRAPH_CHARS - 1),
            MAX_TOTAL_CHARS,
            "test fixture must total exactly the budget"
        );
        let input = GenerateDocumentInput {
            title: "T".to_string(),
            author: None,
            sections: vec![DocumentSection {
                heading: None,
                paragraphs,
                bullets: vec![],
            }],
        };
        assert!(
            validate_input(&input).is_ok(),
            "at-limit input must be accepted"
        );
    }

    #[test]
    fn rejects_document_over_total_char_budget() {
        // 101 max-size paragraphs = 2_020_000 chars > MAX_TOTAL_CHARS, even
        // though each paragraph and the section/paragraph counts stay within
        // their own limits — only the aggregate budget catches it.
        let mut i = base();
        i.sections[0].paragraphs = (0..101).map(|_| "p".repeat(MAX_PARAGRAPH_CHARS)).collect();
        match validate_input(&i) {
            Err(DocumentError::InvalidInput { field, reason }) => {
                assert_eq!(field, "sections");
                assert!(
                    reason.contains("total document text"),
                    "expected aggregate-budget error, got: {reason}"
                );
            }
            other => panic!("expected InvalidInput(sections), got {other:?}"),
        }
    }

    #[test]
    fn truncate_stderr_caps_and_passes_short_through() {
        let long = "z".repeat(4000);
        let out = DocumentError::truncate_stderr(&long);
        assert!(out.chars().count() <= 500);
        assert!(out.ends_with("[…truncated]"));
        assert_eq!(DocumentError::truncate_stderr("short"), "short");
    }
}
