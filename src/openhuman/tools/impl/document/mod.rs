//! Tool: `generate_document` — build a `.docx` file from a structured
//! section spec via the native-Rust [`engine`] module.
//!
//! Flow (mirrors `generate_presentation` exactly so the two artifact
//! producers stay parallel):
//! 1. Validate the JSON-Schema input early (`types::validate_input`) so
//!    the agent gets a structured `InvalidInput` it can self-correct on.
//! 2. Allocate an artifact dir via `artifacts::create_artifact` (kind =
//!    [`ArtifactKind::Document`], extension `"docx"`). The returned
//!    `meta` starts at `ArtifactStatus::Pending` so an interrupted run
//!    never surfaces as Ready.
//! 3. Persist the verbatim args via `save_artifact_args` for regeneration
//!    parity (the failed-card Retry path, #3162).
//! 4. Generate the `.docx` bytes via [`engine::generate`] — pure Rust,
//!    `docx-rs`-backed, no subprocess. Wrapped in `spawn_blocking` +
//!    `tokio::time::timeout`.
//! 5. Write the bytes, stat for size, flip the artifact to `Ready` via
//!    `finalize_artifact`, return the artifact id + path.
//! 6. On any failure: flip the artifact to `Failed` via `fail_artifact`
//!    so the UI surfaces the reason instead of an indefinite spinner.
//!
//! Added in GH #4847 (Problem 3: `.docx` export). The
//! [`ArtifactKind::Document`] enum variant already existed but had no
//! producer; the byte-agnostic artifact pipeline (Save-As dialog +
//! Downloads copy) and the frontend `document → docx` extension map need
//! no format-specific changes.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::artifacts::{
    create_artifact, fail_artifact, finalize_artifact, ArtifactKind,
};
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

mod engine;
mod types;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use self::types::{validate_input, GenerateDocumentInput, GenerateDocumentOutput};

/// Generation timeout. `docx-rs` typically completes the full section cap
/// in well under a second; the 30 s ceiling is a defensive bound against
/// pathological inputs slipping past `validate_input` and worst-case
/// `spawn_blocking` thread-acquisition latency on a saturated runtime.
const GENERATION_TIMEOUT: Duration = Duration::from_secs(30);

/// Tool name surfaced to the agent. Stable; do not rename without
/// coordinating with the orchestrator agent definition list.
pub const TOOL_NAME: &str = "generate_document";

/// One-shot `.docx` generator. See module docs for the request flow.
pub struct DocumentTool {
    workspace_dir: PathBuf,
    /// Retained for constructor parity with [`PresentationTool`] (both are
    /// registered identically in `tools::ops`) and for future features
    /// (e.g. embedding a `File`-source image) that will need the same
    /// path-validation surface. Not read by the current text-only engine.
    #[allow(dead_code)]
    security: Arc<SecurityPolicy>,
}

impl DocumentTool {
    /// Production constructor. The engine is stateless. Pass the workspace
    /// directory the artifact pipeline writes into, plus the active
    /// [`SecurityPolicy`] (same signature as [`PresentationTool::new`] so
    /// both tools register with an identical call).
    pub fn new(workspace_dir: PathBuf, security: Arc<SecurityPolicy>) -> Self {
        Self {
            workspace_dir,
            security,
        }
    }
}

#[async_trait]
impl Tool for DocumentTool {
    fn name(&self) -> &str {
        TOOL_NAME
    }

    fn description(&self) -> &str {
        // Router-rule format per the existing tool conventions: tell the
        // orchestrator when to use this tool and when NOT to.
        "Generate a Word (.docx) document from a structured section spec. \
         USE THIS when the user asks for a document, a report, a letter, an \
         essay, meeting notes, or any prose deliverable they want as an \
         editable Word file. Provide `title` plus a `sections` array of \
         `{heading?, paragraphs?, bullets?}` objects; headings render bold, \
         bullets render as a list. NOT for: slide decks or presentations \
         (use generate_presentation), spreadsheets, or non-Word formats \
         (PDF, Google Docs exports). The generated file is persisted as an \
         artifact in the workspace and the tool returns the artifact id + \
         absolute path so the agent can reference it in the reply."
    }

    fn parameters_schema(&self) -> Value {
        // Built as separate `json!` bindings to keep macro-expansion depth
        // low and to mirror the presentation tool's schema style.
        let section_item_schema = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "heading": {
                    "type": "string",
                    "maxLength": types::MAX_TEXT_CHARS,
                    "description": "Optional section heading, rendered bold."
                },
                "paragraphs": {
                    "type": "array",
                    "maxItems": types::MAX_PARAGRAPHS_PER_SECTION,
                    "description": "Body paragraphs, one rendered paragraph each.",
                    "items": { "type": "string", "maxLength": types::MAX_PARAGRAPH_CHARS }
                },
                "bullets": {
                    "type": "array",
                    "maxItems": types::MAX_BULLETS_PER_SECTION,
                    "description": "Bullet-list items rendered after the paragraphs.",
                    "items": { "type": "string", "maxLength": types::MAX_PARAGRAPH_CHARS }
                }
            }
        });

        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["title", "sections"],
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Document title. Rendered as the leading title line and used as the artifact's human-readable name. Required, non-empty.",
                    "maxLength": types::MAX_TEXT_CHARS,
                },
                "author": {
                    "type": "string",
                    "description": "Optional author byline, rendered italic beneath the title.",
                    "maxLength": types::MAX_TEXT_CHARS,
                },
                "sections": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": types::MAX_SECTIONS,
                    "description": "Sections in display order. At least one entry required; each must have at least one of heading / paragraphs / bullets. Hard cap to bound generation time + output size.",
                    "items": section_item_schema,
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Writes a file to the workspace artifacts dir. No subprocess /
        // network reach.
        PermissionLevel::Write
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let input: GenerateDocumentInput = match serde_json::from_value(args.clone()) {
            Ok(v) => v,
            Err(err) => {
                let msg = format!("invalid generate_document arguments: {err}");
                tracing::warn!(target: "document", err = %err, "[document] deserialisation failed");
                return Ok(ToolResult::error(msg));
            }
        };

        if let Err(err) = validate_input(&input) {
            tracing::debug!(target: "document", err = %err, "[document] validation rejected input");
            return Ok(ToolResult::error(err.to_string()));
        }

        tracing::info!(
            target: "document",
            title_chars = input.title.chars().count(),
            has_author = input.author.is_some(),
            section_count = input.sections.len(),
            "[document] generation request accepted"
        );

        let (meta, output_path) = create_artifact(
            &self.workspace_dir,
            ArtifactKind::Document,
            &input.title,
            "docx",
        )
        .await
        .map_err(anyhow::Error::msg)?;

        // Persist the verbatim args next to meta.json so a failed card's
        // Retry can re-dispatch this exact spec (#3162). Best-effort: a
        // write failure only forfeits regeneration, never aborts an
        // otherwise-successful generation.
        if let Err(err) = crate::openhuman::artifacts::store::save_artifact_args(
            &self.workspace_dir,
            &meta.id,
            &args,
        )
        .await
        {
            tracing::warn!(
                target: "document",
                err = %err,
                artifact_id = %meta.id,
                "[document] failed to persist args.json; artifact will not be regenerable"
            );
        }

        let bytes = match engine::generate(&input, GENERATION_TIMEOUT).await {
            Ok(bytes) => bytes,
            Err(err) => {
                let _ = fail_artifact(&self.workspace_dir, &meta.id, &err.to_string()).await;
                tracing::warn!(
                    target: "document",
                    err = %err,
                    "[document] engine generation failed"
                );
                return Ok(ToolResult::error(err.to_string()));
            }
        };

        if let Err(err) = tokio::fs::write(&output_path, &bytes).await {
            let filename = output_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let reason = format!("failed to write generated document ({filename}): {err}");
            let _ = fail_artifact(&self.workspace_dir, &meta.id, &reason).await;
            tracing::warn!(
                target: "document",
                err = %err,
                artifact_id = %meta.id,
                filename = %filename,
                "[document] artifact file write failed"
            );
            return Ok(ToolResult::error(reason));
        }

        let size_bytes = bytes.len() as u64;
        let updated = match finalize_artifact(&self.workspace_dir, &meta.id, size_bytes).await {
            Ok(updated) => updated,
            Err(err) => {
                let reason = format!("failed to finalize artifact: {err}");
                // File is already on disk but the ledger transition failed.
                // Flip to Failed so the UI surfaces the error instead of a
                // stuck `Pending` spinner. Fail-artifact errors are
                // swallowed — they can only recur if the same ledger backend
                // is unavailable.
                let _ = fail_artifact(&self.workspace_dir, &meta.id, &reason).await;
                tracing::warn!(
                    target: "document",
                    err = %err,
                    artifact_id = %meta.id,
                    "[document] finalize_artifact failed; flipped to Failed"
                );
                return Ok(ToolResult::error(reason));
            }
        };

        tracing::info!(
            target: "document",
            artifact_id = %updated.id,
            size_bytes,
            section_count = input.sections.len(),
            "[document] generation complete"
        );

        let out = GenerateDocumentOutput {
            artifact_id: updated.id.clone(),
            artifact_path: output_path.display().to_string(),
            section_count: input.sections.len(),
            size_bytes,
        };
        let payload = serde_json::to_value(&out)?;
        let markdown = format!(
            "Generated a {}-section document at `{}` (artifact `{}`, {} bytes).",
            out.section_count, out.artifact_path, out.artifact_id, out.size_bytes
        );
        Ok(ToolResult::success_with_markdown(payload, markdown))
    }
}
