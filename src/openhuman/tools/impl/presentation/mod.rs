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
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::artifacts::{
    create_artifact, fail_artifact, finalize_artifact, ArtifactKind,
};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

mod engine;
mod types;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use self::types::{
    validate_input, GeneratePresentationInput, GeneratePresentationOutput, PresentationError,
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
}

impl PresentationTool {
    /// Production constructor. The engine is stateless — no runtime
    /// resolution, venv setup, or cache directory needed. Pass the
    /// workspace directory the artifact pipeline writes into.
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
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
                    "items": {
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
                            "speaker_notes": { "type": "string", "maxLength": types::MAX_TEXT_CHARS }
                        }
                    }
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

        let (meta, output_path) = create_artifact(
            &self.workspace_dir,
            ArtifactKind::Presentation,
            &input.title,
            "pptx",
        )
        .await
        .map_err(anyhow::Error::msg)?;

        let bytes = match engine::generate(&input, GENERATION_TIMEOUT).await {
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
        };
        let payload = serde_json::to_value(&out)?;
        let markdown = format!(
            "Generated {}-slide presentation at `{}` (artifact `{}`, {} bytes).",
            out.slide_count, out.artifact_path, out.artifact_id, out.size_bytes
        );
        Ok(ToolResult::success_with_markdown(payload, markdown))
    }
}
