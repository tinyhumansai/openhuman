//! Tool: `generate_presentation` — build a `.pptx` deck from a
//! structured slide spec via the native-Rust [`engine`] module.
//!
//! Flow:
//! 1. Validate the JSON-Schema input early (`types::validate_input`)
//!    so the agent gets a structured `InvalidInput` it can self-correct
//!    on instead of a low-level error.
//! 2. Allocate an artifact dir via `artifacts::create_artifact`. The
//!    returned `meta` starts at `ArtifactStatus::Pending` so an
//!    interrupted run never surfaces as Ready.
//! 3. Generate the deck bytes via [`engine::generate`] — pure Rust,
//!    `ppt-rs`-backed, no Python runtime, no subprocess. Wrapped in
//!    `spawn_blocking` + `tokio::time::timeout` so the synchronous
//!    library work neither blocks the async executor nor can wedge
//!    the agent loop.
//! 4. Write the bytes to the artifact's output path, stat for size,
//!    flip artifact to `Ready` via `artifacts::finalize_artifact`,
//!    return the artifact id + path.
//! 5. On failure: flip artifact to `Failed` via
//!    `artifacts::fail_artifact` so the UI can surface the reason.
//!
//! Originally shipped in #2778 against a managed python-pptx venv;
//! refactored to a native-Rust engine in #2780-follow-up to drop the
//! Python runtime + first-call venv-install latency + 50 MB+ Python
//! disk footprint. Tool name / input schema / output schema / artifact
//! layout are byte-identical across the swap so #3017 ArtifactCard,
//! #3026 Files panel, and the orchestrator grounding rule in #3029
//! continue to work without change.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::artifacts::{
    create_artifact, fail_artifact, finalize_artifact, read_artifact_bytes, ArtifactKind,
};
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

mod engine;
mod image_util;
mod types;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use self::types::{
    validate_input, GeneratePresentationInput, GeneratePresentationOutput, ResolvedSlideImage,
    SlideImage, SlideImageSource, MAX_IMAGE_BYTES,
};

/// Generation timeout. `ppt-rs` typically completes the full 64-slide
/// cap in well under a second; the 30 s ceiling is a defensive bound
/// against pathological inputs slipping past `validate_input` and
/// the worst-case `spawn_blocking` thread-acquisition latency on a
/// saturated runtime.
const GENERATION_TIMEOUT: Duration = Duration::from_secs(30);

/// Tool name surfaced to the agent. Stable; do not rename without
/// coordinating with the orchestrator agent definition list.
pub const TOOL_NAME: &str = "generate_presentation";

/// One-shot `.pptx` generator. See module docs for the request flow.
pub struct PresentationTool {
    workspace_dir: PathBuf,
    /// Security policy used to validate agent-supplied `File` image paths
    /// before any filesystem read — an image path must pass the same
    /// `validate_path` checks (allowed-location, symlink-escape, forbidden
    /// dirs) as any other file-read operation.
    security: Arc<SecurityPolicy>,
}

impl PresentationTool {
    /// Production constructor. The engine is stateless — no runtime
    /// resolution, venv setup, or cache directory needed. Pass the
    /// workspace directory the artifact pipeline writes into, plus the
    /// active [`SecurityPolicy`] for validating `File`-source image paths.
    pub fn new(workspace_dir: PathBuf, security: Arc<SecurityPolicy>) -> Self {
        Self {
            workspace_dir,
            security,
        }
    }
}

#[async_trait]
impl Tool for PresentationTool {
    fn name(&self) -> &str {
        TOOL_NAME
    }

    fn description(&self) -> &str {
        // Router-rule format per the existing tool conventions (see
        // `current_time.rs` etc.): tell the orchestrator when to use
        // this tool and when NOT to.
        "Generate a PowerPoint (.pptx) presentation from a structured slide spec. \
         USE THIS when the user asks for slides, a deck, a presentation, or a \
         slide-by-slide breakdown of a topic. Provide `title` plus a `slides` \
         array of `{title, body?, bullets?, speaker_notes?}` objects. NOT for: \
         per-slide image generation, live editing of existing decks, or non-PPT \
         formats (PDF, Keynote, Google Slides exports). The generated file is \
         persisted as an artifact in the workspace and the tool returns the \
         artifact id + absolute path so the agent can reference it in the reply."
    }

    fn parameters_schema(&self) -> Value {
        // Built as separate `json!` bindings (rather than one deeply-nested
        // literal) to keep macro expansion within the crate's default
        // recursion limit — the per-slide `images` sub-schema adds enough
        // depth to overflow a single combined literal.
        let image_item_schema = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["source"],
            "properties": {
                "source": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["type"],
                    "description": "Image data source. Use `{type:'artifact', artifact_id}` for a prior tool's output, or `{type:'file', path}` for a readable local file.",
                    "properties": {
                        "type": { "type": "string", "enum": ["artifact", "file"] },
                        "artifact_id": { "type": "string", "description": "Required when type='artifact'." },
                        "path": { "type": "string", "description": "Required when type='file'. Absolute or action-dir-relative path." }
                    }
                },
                "caption": {
                    "type": "string",
                    "maxLength": types::MAX_TEXT_CHARS,
                    "description": "Optional caption rendered as a text bullet beneath the image."
                }
            }
        });

        let images_schema = json!({
            "type": "array",
            "maxItems": types::MAX_IMAGES_PER_SLIDE,
            "description": "Images to embed beneath the slide text, single-column. PNG or JPEG only (≤5 MB each, ≤8 per deck). Each image's bytes come from a workspace artifact or a local file path the agent can read. Only attach an image whose content you have actually inspected — do not claim an image shows something you have not verified.",
            "items": image_item_schema,
        });

        let slide_item_schema = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "title": { "type": "string", "maxLength": types::MAX_TEXT_CHARS },
                "body": { "type": "string", "maxLength": types::MAX_TEXT_CHARS },
                "bullets": {
                    "type": "array",
                    "maxItems": types::MAX_BULLETS_PER_SLIDE,
                    "items": { "type": "string", "maxLength": types::MAX_TEXT_CHARS }
                },
                "speaker_notes": { "type": "string", "maxLength": types::MAX_TEXT_CHARS },
                "images": images_schema,
            }
        });

        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["title", "slides"],
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Deck title. Surfaced on the title slide and used as the artifact's human-readable name. Required, non-empty.",
                    "maxLength": types::MAX_TEXT_CHARS,
                },
                "author": {
                    "type": "string",
                    "description": "Optional author byline shown on the title slide.",
                    "maxLength": types::MAX_TEXT_CHARS,
                },
                "theme": {
                    "type": "string",
                    "description": "Reserved for future template-selection work. Currently informational only.",
                    "maxLength": types::MAX_TEXT_CHARS,
                },
                "slides": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": types::MAX_SLIDES,
                    "description": "Slide specs in display order. At least one entry required; hard cap to bound generation time + output size.",
                    "items": slide_item_schema,
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // We write files to the workspace artifacts dir. Treat as
        // Write rather than ReadOnly. No subprocess / network reach.
        PermissionLevel::Write
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let input: GeneratePresentationInput = match serde_json::from_value(args.clone()) {
            Ok(v) => v,
            Err(err) => {
                let msg = format!("invalid generate_presentation arguments: {err}");
                tracing::warn!(target: "presentation", err = %err, "[presentation] deserialisation failed");
                return Ok(ToolResult::error(msg));
            }
        };

        if let Err(err) = validate_input(&input) {
            tracing::debug!(target: "presentation", err = %err, "[presentation] validation rejected input");
            return Ok(ToolResult::error(err.to_string()));
        }

        tracing::info!(
            target: "presentation",
            title_chars = input.title.chars().count(),
            has_author = input.author.is_some(),
            slide_count = input.slides.len(),
            "[presentation] generation request accepted"
        );

        // Resolve + validate per-slide images at the async boundary. A
        // bad image is skipped with a warning (partial success) rather
        // than failing the whole deck — mirrors the #3076 ethos.
        let (resolved_images, image_warnings) = self.resolve_images(&input).await;
        if !image_warnings.is_empty() {
            tracing::info!(
                target: "presentation",
                warning_count = image_warnings.len(),
                "[presentation] some images skipped with warnings"
            );
        }

        let (meta, output_path) = create_artifact(
            &self.workspace_dir,
            ArtifactKind::Presentation,
            &input.title,
            "pptx",
        )
        .await
        .map_err(anyhow::Error::msg)?;

        // Persist the verbatim args next to meta.json so a failed card's
        // Retry can re-dispatch this exact spec deterministically (#3162).
        // Best-effort: a write failure only forfeits future regeneration,
        // it must not abort an otherwise-successful generation.
        if let Err(err) = crate::openhuman::artifacts::store::save_artifact_args(
            &self.workspace_dir,
            &meta.id,
            &args,
        )
        .await
        {
            tracing::warn!(
                target: "presentation",
                err = %err,
                artifact_id = %meta.id,
                "[presentation] failed to persist args.json; artifact will not be regenerable"
            );
        }

        let bytes = match engine::generate(&input, &resolved_images, GENERATION_TIMEOUT).await {
            Ok(bytes) => bytes,
            Err(err) => {
                let _ = fail_artifact(&self.workspace_dir, &meta.id, &err.to_string()).await;
                tracing::warn!(
                    target: "presentation",
                    err = %err,
                    "[presentation] engine generation failed"
                );
                return Ok(ToolResult::error(err.to_string()));
            }
        };

        if let Err(err) = tokio::fs::write(&output_path, &bytes).await {
            let filename = output_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let reason = format!("failed to write generated deck ({filename}): {err}");
            let _ = fail_artifact(&self.workspace_dir, &meta.id, &reason).await;
            tracing::warn!(
                target: "presentation",
                err = %err,
                artifact_id = %meta.id,
                filename = %filename,
                "[presentation] artifact file write failed"
            );
            return Ok(ToolResult::error(reason));
        }

        let size_bytes = bytes.len() as u64;
        let updated = match finalize_artifact(&self.workspace_dir, &meta.id, size_bytes).await {
            Ok(updated) => updated,
            Err(err) => {
                let reason = format!("failed to finalize artifact: {err}");
                // File is already on disk but the ledger transition failed.
                // Flip the artifact to Failed so the UI surfaces the error
                // instead of leaving it stuck in `Pending`. Fail-artifact
                // errors are swallowed — they can only happen if the same
                // ledger backend is unavailable, in which case nothing we
                // do here will help.
                let _ = fail_artifact(&self.workspace_dir, &meta.id, &reason).await;
                tracing::warn!(
                    target: "presentation",
                    err = %err,
                    artifact_id = %meta.id,
                    "[presentation] finalize_artifact failed; flipped to Failed"
                );
                return Ok(ToolResult::error(reason));
            }
        };

        tracing::info!(
            target: "presentation",
            artifact_id = %updated.id,
            size_bytes,
            slide_count = input.slides.len(),
            "[presentation] generation complete"
        );

        let out = GeneratePresentationOutput {
            artifact_id: updated.id.clone(),
            artifact_path: output_path.display().to_string(),
            slide_count: input.slides.len(),
            size_bytes,
            image_warnings,
        };
        let payload = serde_json::to_value(&out)?;
        let mut markdown = format!(
            "Generated {}-slide presentation at `{}` (artifact `{}`, {} bytes).",
            out.slide_count, out.artifact_path, out.artifact_id, out.size_bytes
        );
        if !out.image_warnings.is_empty() {
            markdown.push_str("\n\n⚠️ Some images were skipped:");
            for warning in &out.image_warnings {
                markdown.push_str(&format!("\n- {warning}"));
            }
        }
        Ok(ToolResult::success_with_markdown(payload, markdown))
    }
}

impl PresentationTool {
    /// Resolve + validate every slide's images at the async boundary,
    /// returning a per-slide vec of embeddable images (aligned 1:1 with
    /// `input.slides`) plus a flat list of human-readable warnings for
    /// images that were skipped.
    ///
    /// A skip is never fatal: a deck with one bad image still renders,
    /// minus that image, and the agent is told why via the warning list.
    async fn resolve_images(
        &self,
        input: &GeneratePresentationInput,
    ) -> (Vec<Vec<ResolvedSlideImage>>, Vec<String>) {
        let mut per_slide: Vec<Vec<ResolvedSlideImage>> = Vec::with_capacity(input.slides.len());
        let mut warnings: Vec<String> = Vec::new();

        for (slide_idx, spec) in input.slides.iter().enumerate() {
            let mut resolved = Vec::with_capacity(spec.images.len());
            for (img_idx, image) in spec.images.iter().enumerate() {
                match self.resolve_one_image(image).await {
                    Ok(r) => resolved.push(r),
                    Err(reason) => {
                        // 1-based indices in the message (matches how the
                        // agent thinks about "slide 2, image 1").
                        warnings.push(format!(
                            "slide {} image {}: {reason}",
                            slide_idx + 1,
                            img_idx + 1
                        ));
                    }
                }
            }
            per_slide.push(resolved);
        }

        (per_slide, warnings)
    }

    /// Resolve a single [`SlideImage`] to validated, embeddable bytes.
    /// Returns `Err(reason)` (a short human-readable string) when the
    /// image cannot be embedded — the caller turns that into a skip
    /// warning.
    async fn resolve_one_image(&self, image: &SlideImage) -> Result<ResolvedSlideImage, String> {
        let bytes = match &image.source {
            SlideImageSource::Artifact { artifact_id } => {
                read_artifact_bytes(&self.workspace_dir, artifact_id)
                    .await
                    .map_err(|e| format!("artifact {artifact_id} unreadable: {e}"))?
            }
            SlideImageSource::File { path } => {
                // Validate the agent-supplied path against the security
                // policy BEFORE touching the filesystem. This enforces the
                // same allowed-location / symlink-escape / forbidden-dir
                // checks as every other file-read tool — without it an
                // agent could embed `/etc/shadow` or `~/.ssh/id_rsa`.
                // `validate_path` returns the canonical, in-policy path.
                let resolved = self
                    .security
                    .validate_path(path)
                    .await
                    .map_err(|e| format!("file {path} not allowed: {e}"))?;
                // Stat first so a pathologically large file is rejected
                // before we pull it into memory.
                let meta = tokio::fs::metadata(&resolved)
                    .await
                    .map_err(|e| format!("file {path} unreadable: {e}"))?;
                if meta.len() as usize > MAX_IMAGE_BYTES {
                    return Err(format!(
                        "file {path} is {} bytes, exceeds {MAX_IMAGE_BYTES}-byte cap",
                        meta.len()
                    ));
                }
                tokio::fs::read(&resolved)
                    .await
                    .map_err(|e| format!("file {path} unreadable: {e}"))?
            }
        };

        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(format!(
                "image is {} bytes, exceeds {MAX_IMAGE_BYTES}-byte cap",
                bytes.len()
            ));
        }

        let format = image_util::sniff_format(&bytes).ok_or_else(|| {
            "unsupported image type (only PNG and JPEG are embeddable)".to_string()
        })?;

        let (width_px, height_px) = image_util::pixel_dimensions(&bytes, format)
            .ok_or_else(|| format!("could not read {format} dimensions (corrupt header?)"))?;

        Ok(ResolvedSlideImage {
            bytes,
            format,
            width_px,
            height_px,
            caption: image.caption.clone(),
        })
    }
}
