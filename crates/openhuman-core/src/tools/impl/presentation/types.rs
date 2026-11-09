//! Typed input / output / error contracts for the `generate_presentation` tool.

use serde::{Deserialize, Serialize};
use tinydocs_bus::spec::presentation::{
    PresentationRequestSpec, PresentationSpec, SlideRequestSpec, SlideSpec,
};

use crate::modules::documents::DocumentCallError;

// The size limits are the wire contract's: the tool validates against the same
// numbers the `tinydocs` module enforces, so they are defined once, there.
pub(super) use tinydocs_bus::spec::presentation::{
    MAX_BULLETS_PER_SLIDE, MAX_IMAGES_PER_SLIDE, MAX_IMAGE_BYTES, MAX_SLIDES, MAX_TEXT_CHARS,
};

/// Where a slide image's bytes come from. `Url` is intentionally
/// **deferred** in v1 — fetching agent-supplied URLs at generation time
/// is an SSRF surface that needs its own all- / deny-list design.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlideImageSource {
    /// Bytes from a workspace artifact (e.g. a chart the agent produced
    /// earlier via another tool). Resolved through
    /// [`crate::artifacts::read_artifact_bytes`].
    Artifact { artifact_id: String },
    /// Bytes from a local filesystem path the agent already has read
    /// access to (e.g. a screenshot saved under the action dir).
    File { path: String },
}

/// One image the agent asks for on a slide (the tool's input shape; the bus
/// `SlideImage` it resolves to carries bytes, format and dimensions).
/// `caption`, when present, is rendered
/// as a trailing text bullet beneath the image in v1 (a true
/// image-anchored caption shape is deferred with the grid layout).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlideImageRequest {
    /// Image data source. See [`SlideImageSource`].
    pub source: SlideImageSource,
    /// Optional caption rendered as a bullet under the image.
    #[serde(default)]
    pub caption: Option<String>,
}

/// Slide spec — one entry per content slide in the generated deck.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlideRequest {
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
    /// Images attached to the slide, rendered single-column beneath the
    /// text. Resolved + validated (MIME / size / dimensions) at the
    /// async `execute()` boundary; a bad image is skipped with a
    /// warning rather than failing the whole deck.
    #[serde(default)]
    pub images: Vec<SlideImageRequest>,
}

/// Top-level input for the `generate_presentation` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratePresentationInput {
    /// Deck title. Surfaces on the title slide and as the artifact's
    /// human-readable name.
    pub title: String,
    /// Optional artifact id to update. Pass an artifact id from a prior
    /// `generate_presentation` call to save this deck as a new version
    /// of that artifact instead of creating a brand new one.
    #[serde(default)]
    pub artifact_id: Option<String>,
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
    pub slides: Vec<SlideRequest>,
}

/// Tool output returned via [`tinytools::ToolResult`]
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
    /// Per-image warnings for assets that could not be embedded (bad
    /// MIME, oversize, unreadable, undecodable dimensions). The deck is
    /// still produced without those images — partial success rather than
    /// a hard failure. Empty when every image resolved cleanly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_warnings: Vec<String>,
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

    /// The document module could not be loaded on this host.
    ///
    /// Distinct from `GenerationFailed`: that is a deck that might work on a
    /// retry, this is a capability that is not present and will not become
    /// present without a restart.
    #[error("presentation generation is unavailable: {reason}")]
    ModuleUnavailable { reason: String },

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

impl From<DocumentCallError> for PresentationError {
    /// Map a module-call failure onto the agent-facing shape.
    ///
    /// The three call outcomes mean three different things to an agent:
    /// `InvalidInput` is a spec it can rewrite, `Failed` is a deck that might
    /// work on a retry, and `Unavailable` means the capability is not present
    /// and it should stop asking.
    ///
    /// The structured `field` / `reason` pair does not survive the bus — an
    /// error crosses as a name plus a message — so an `InvalidInput` from the
    /// module names `spec`; the host validates the resolved bus spec before it
    /// starts artifact production.
    ///
    /// `exit_code` is `-1` because it always is: the field is a vestige of the
    /// python-pptx subprocess this path replaced in #2778, kept so the agent's
    /// error shape did not churn.
    fn from(err: DocumentCallError) -> Self {
        match err {
            DocumentCallError::InvalidInput(reason) => Self::InvalidInput {
                field: "spec".to_string(),
                reason: Self::truncate_stderr(&reason),
            },
            DocumentCallError::Unavailable(reason) => Self::ModuleUnavailable {
                reason: Self::truncate_stderr(&reason),
            },
            DocumentCallError::Failed(reason) => Self::GenerationFailed {
                exit_code: -1,
                stderr_truncated: Self::truncate_stderr(&reason),
            },
        }
    }
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

impl From<tinydocs_bus::Error> for PresentationError {
    fn from(error: tinydocs_bus::Error) -> Self {
        match error {
            tinydocs_bus::Error::InvalidInput { field, reason } => {
                Self::InvalidInput { field, reason }
            }
            other => Self::InvalidInput {
                field: "spec".to_string(),
                reason: other.to_string(),
            },
        }
    }
}

/// Validate source-agnostic request metadata before the host resolves image
/// references. Each requested image is represented by its caption, including
/// images whose host-side reference will later fail to resolve.
pub(super) fn validate_request(input: &GeneratePresentationInput) -> Result<(), PresentationError> {
    let request = PresentationRequestSpec {
        title: input.title.clone(),
        author: input.author.clone(),
        theme: input.theme.clone(),
        slides: input
            .slides
            .iter()
            .map(|slide| SlideRequestSpec {
                title: slide.title.clone(),
                body: slide.body.clone(),
                bullets: slide.bullets.clone(),
                speaker_notes: slide.speaker_notes.clone(),
                images: slide
                    .images
                    .iter()
                    .map(|image| image.caption.clone())
                    .collect(),
            })
            .collect(),
    };
    request.validate().map_err(PresentationError::from)
}

/// Reject empty host-owned image references before attempting to resolve them.
pub(super) fn validate_image_references(
    input: &GeneratePresentationInput,
) -> Result<(), PresentationError> {
    for (slide_idx, slide) in input.slides.iter().enumerate() {
        for (image_idx, image) in slide.images.iter().enumerate() {
            let (field, value) = match &image.source {
                SlideImageSource::Artifact { artifact_id } => (
                    format!("slides[{slide_idx}].images[{image_idx}].source.artifact_id"),
                    artifact_id,
                ),
                SlideImageSource::File { path } => (
                    format!("slides[{slide_idx}].images[{image_idx}].source.path"),
                    path,
                ),
            };
            if value.trim().is_empty() {
                return Err(PresentationError::InvalidInput {
                    field,
                    reason: "must not be empty".to_string(),
                });
            }
        }
    }
    Ok(())
}

/// Project tool input and host-resolved image bytes onto the owner bus contract.
pub(super) fn to_bus_spec(
    input: &GeneratePresentationInput,
    resolved_images: Vec<Vec<tinydocs_bus::spec::SlideImage>>,
) -> Result<PresentationSpec, PresentationError> {
    if input.slides.len() != resolved_images.len() {
        return Err(PresentationError::InvalidInput {
            field: "slides".to_string(),
            reason: "resolved image count did not match slide count".to_string(),
        });
    }

    let slides = input
        .slides
        .iter()
        .zip(resolved_images)
        .map(|(slide, images)| SlideSpec {
            title: slide.title.clone(),
            body: slide.body.clone(),
            bullets: slide.bullets.clone(),
            speaker_notes: slide.speaker_notes.clone(),
            images,
        })
        .collect();

    Ok(PresentationSpec {
        title: input.title.clone(),
        author: input.author.clone(),
        theme: input.theme.clone(),
        slides,
    })
}
