//! Typed input / output / error contracts for the `generate_presentation` tool.

use serde::{Deserialize, Serialize};

/// Maximum number of slides a single `generate_presentation` call may
/// produce. Hard cap to bound generation time and output size; the
/// LLM is asked to break larger decks into multiple calls.
pub(super) const MAX_SLIDES: usize = 64;

/// Maximum length of a single text field (title, body, individual
/// bullet, speaker notes). Bounds the payload size sent to the
/// `ppt-rs` engine and avoids pathological inputs that would balloon
/// the deck.
pub(super) const MAX_TEXT_CHARS: usize = 2_000;

/// Maximum number of bullets per slide. Higher counts produce
/// unreadable slides and bloat the output file.
pub(super) const MAX_BULLETS_PER_SLIDE: usize = 32;

/// Slide spec — one entry per content slide in the generated deck.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlideSpec {
    /// Slide title. Empty / omitted is allowed for visually
    /// minimalist decks but at least one of `title` / `body` /
    /// `bullets` must be populated.
    #[serde(default)]
    pub title: String,
    /// Paragraph body text. Plain text only — rendered into the
    /// default content layout's body placeholder by `ppt-rs`.
    #[serde(default)]
    pub body: Option<String>,
    /// Bullet points rendered after the body text (if any).
    #[serde(default)]
    pub bullets: Vec<String>,
    /// Speaker notes attached to the slide.
    #[serde(default)]
    pub speaker_notes: Option<String>,
}

/// Top-level input for the `generate_presentation` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratePresentationInput {
    /// Deck title. Surfaces on the title slide and as the artifact's
    /// human-readable name.
    pub title: String,
    /// Optional author byline, surfaced on the title slide.
    #[serde(default)]
    pub author: Option<String>,
    /// Optional theme hint. Currently informational only; the `ppt-rs`
    /// engine uses its default template regardless. Reserved for
    /// future template-selection work.
    #[serde(default)]
    pub theme: Option<String>,
    /// Slide specs, in display order. Must contain at least one entry.
    #[serde(default)]
    pub slides: Vec<SlideSpec>,
}

/// Tool output returned via [`crate::openhuman::tools::traits::ToolResult`]
/// as the JSON `data` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratePresentationOutput {
    /// UUID of the persisted artifact record. Use with the
    /// `ai_get_artifact` / `ai_delete_artifact` RPCs.
    pub artifact_id: String,
    /// Absolute filesystem path to the generated `.pptx`. Useful for
    /// the agent to reference in its reply ("saved to …").
    pub artifact_path: String,
    /// Number of content slides actually produced (excludes the
    /// title slide).
    pub slide_count: usize,
    /// On-disk size of the produced `.pptx` in bytes.
    pub size_bytes: u64,
}

/// Structured error variants surfaced to the agent. Aligned with the
/// taxonomy #2780 will surface to the user via the orchestrator.
#[derive(Debug, thiserror::Error)]
pub enum PresentationError {
    #[error("invalid input for field '{field}': {reason}")]
    InvalidInput { field: String, reason: String },

    #[error("presentation generation failed (exit={exit_code}): {stderr_truncated}")]
    GenerationFailed {
        exit_code: i32,
        stderr_truncated: String,
    },

    #[error("presentation generation exceeded {timeout_secs}s timeout")]
    GenerationTimeout { timeout_secs: u64 },

    /// Reserved for the planned `format` selector that will let callers
    /// request alternative deck formats (`.pdf` / `.key` / image
    /// strips). Today the tool only emits `.pptx`, so this variant is
    /// not constructed by `execute` — it exists ahead of #2780's
    /// follow-up wiring so downstream error-handling sites can pattern-
    /// match exhaustively without a churn-y enum bump later.
    #[allow(dead_code)]
    #[error("unsupported file type '{extension}'; supported: {supported}")]
    UnsupportedFileType {
        extension: String,
        supported: String,
    },
}

impl PresentationError {
    /// Truncate a stderr string to the per-#2780 cap of 500 chars
    /// (UTF-8-safe). Used when wrapping a non-zero exit into
    /// `GenerationFailed` so the variant never carries an unbounded
    /// payload back to the agent.
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

/// Validate the input early — before invoking the `ppt-rs` engine — so
/// the agent gets a structured `InvalidInput` it can self-correct on
/// instead of a generic engine error.
pub(super) fn validate_input(input: &GeneratePresentationInput) -> Result<(), PresentationError> {
    if input.title.trim().is_empty() {
        return Err(PresentationError::InvalidInput {
            field: "title".to_string(),
            reason: "must not be empty".to_string(),
        });
    }
    if input.title.chars().count() > MAX_TEXT_CHARS {
        return Err(PresentationError::InvalidInput {
            field: "title".to_string(),
            reason: format!("must be ≤ {MAX_TEXT_CHARS} chars"),
        });
    }
    if let Some(author) = input.author.as_deref() {
        if author.chars().count() > MAX_TEXT_CHARS {
            return Err(PresentationError::InvalidInput {
                field: "author".to_string(),
                reason: format!("must be ≤ {MAX_TEXT_CHARS} chars"),
            });
        }
    }
    if let Some(theme) = input.theme.as_deref() {
        if theme.chars().count() > MAX_TEXT_CHARS {
            return Err(PresentationError::InvalidInput {
                field: "theme".to_string(),
                reason: format!("must be ≤ {MAX_TEXT_CHARS} chars"),
            });
        }
    }
    if input.slides.is_empty() {
        return Err(PresentationError::InvalidInput {
            field: "slides".to_string(),
            reason: "must contain at least one slide".to_string(),
        });
    }
    if input.slides.len() > MAX_SLIDES {
        return Err(PresentationError::InvalidInput {
            field: "slides".to_string(),
            reason: format!("must contain ≤ {MAX_SLIDES} slides"),
        });
    }
    for (i, slide) in input.slides.iter().enumerate() {
        let has_title = !slide.title.trim().is_empty();
        let has_body = slide
            .body
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        // Reject whitespace-only bullets too: build_slides() trims and drops
        // empty entries, so a slide with only ["   "] would render blank
        // despite passing this "at least one of title/body/bullets" gate.
        let has_bullets = slide.bullets.iter().any(|b| !b.trim().is_empty());
        if !has_title && !has_body && !has_bullets {
            return Err(PresentationError::InvalidInput {
                field: format!("slides[{i}]"),
                reason: "must have at least one of title / body / bullets".to_string(),
            });
        }
        if slide.title.chars().count() > MAX_TEXT_CHARS {
            return Err(PresentationError::InvalidInput {
                field: format!("slides[{i}].title"),
                reason: format!("must be ≤ {MAX_TEXT_CHARS} chars"),
            });
        }
        if let Some(body) = slide.body.as_deref() {
            if body.chars().count() > MAX_TEXT_CHARS {
                return Err(PresentationError::InvalidInput {
                    field: format!("slides[{i}].body"),
                    reason: format!("must be ≤ {MAX_TEXT_CHARS} chars"),
                });
            }
        }
        if slide.bullets.len() > MAX_BULLETS_PER_SLIDE {
            return Err(PresentationError::InvalidInput {
                field: format!("slides[{i}].bullets"),
                reason: format!("must contain ≤ {MAX_BULLETS_PER_SLIDE} bullets"),
            });
        }
        for (b, bullet) in slide.bullets.iter().enumerate() {
            if bullet.chars().count() > MAX_TEXT_CHARS {
                return Err(PresentationError::InvalidInput {
                    field: format!("slides[{i}].bullets[{b}]"),
                    reason: format!("must be ≤ {MAX_TEXT_CHARS} chars"),
                });
            }
        }
        if let Some(notes) = slide.speaker_notes.as_deref() {
            if notes.chars().count() > MAX_TEXT_CHARS {
                return Err(PresentationError::InvalidInput {
                    field: format!("slides[{i}].speaker_notes"),
                    reason: format!("must be ≤ {MAX_TEXT_CHARS} chars"),
                });
            }
        }
    }
    Ok(())
}
