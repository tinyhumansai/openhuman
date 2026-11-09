//! Tool: `generate_document` — build a `.docx` file from a structured
//! section spec via the native-Rust [`engine`] module.
//!
//! Flow (mirrors `generate_presentation` exactly so the two artifact
//! producers stay parallel):
//! 1. Validate the JSON-Schema input early (`types::validate_input`) so
//!    the agent gets a structured `InvalidInput` it can self-correct on.
//! 2. Start a [`Production`]: it reserves `<title>.docx` in the files folder
//!    (the card shows "generating…") and opens the versioned artifact — or,
//!    with an `artifact_id` arg, a new version of that artifact under the same
//!    id and file.
//! 3. Persist the verbatim args via `save_artifact_args` for regeneration
//!    parity (the failed-card Retry path, #3162).
//! 4. Generate the `.docx` bytes via [`engine::generate`] — pure Rust,
//!    `docx-rs`-backed, no subprocess. Wrapped in `spawn_blocking` +
//!    `tokio::time::timeout`.
//! 5. Write the bytes and [`Production::complete`]: the file becomes the
//!    artifact's newest version and the card turns ready.
//! 6. On any failure: [`Production::fail`] so the UI surfaces the reason
//!    instead of an indefinite spinner.
//!
//! Added in GH #4847 (Problem 3: `.docx` export). The legacy
//! `ArtifactKind::Document` enum variant already existed but had no
//! producer; the byte-agnostic artifact pipeline (Save-As dialog +
//! Downloads copy) and the frontend `document → docx` extension map need
//! no format-specific changes.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::artifacts::production::Production;
use crate::security::SecurityPolicy;
use tinytools::{PermissionLevel, Tool, ToolResult};

mod engine;
/// The document wire contract, shared with the `tinydocs` module.
///
/// This was 1,873 lines of this repository — `format/error/`, `format/spec/` —
/// and every line of it also existed in `crates/tinydocs-bus/src/` upstream,
/// differing only in the paths inside doc links. Two definitions of a contract
/// is the drift risk the contract exists to remove: the specs here are what an
/// LLM is shown as a JSON tool schema and what the module validates against,
/// so a limit that moved on one side would become a tool description promising
/// something the module does not enforce.
///
/// Aliased rather than re-exported item by item so the ~30 existing
/// `…::document::format::…` paths keep resolving unchanged.
pub(crate) use tinydocs_bus as format;
mod types;

#[cfg(test)]
#[path = "document_tests.rs"]
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
    /// Visible folder the generated document is written into (#5505).
    files_dir: crate::artifacts::FileRoots,
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
    pub fn new(
        workspace_dir: PathBuf,
        files_dir: impl Into<crate::artifacts::FileRoots>,
        security: Arc<SecurityPolicy>,
    ) -> Self {
        Self {
            workspace_dir,
            files_dir: files_dir.into(),
            security,
        }
    }

    /// The agent-registry constructor: the artifact metadata goes to
    /// `config.workspace_dir`, the document to the files folder (#5505).
    pub fn for_config(config: &crate::config::Config, security: Arc<SecurityPolicy>) -> Self {
        Self::new(
            config.workspace_dir.clone(),
            crate::artifacts::FileRoots::from_config(config),
            security,
        )
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
                "artifact_id": {
                    "type": "string",
                    "description": "Optional artifact id to update. Pass an id from a prior generate_document call to save this document as a new version of that artifact instead of creating a new one.",
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
        self.run(args, None, &crate::core::runtime::DispatchCtx::ambient())
            .await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: tinytools::ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let approval = crate::agent::tinyagents::host::run_context::approval_for_tool(context);
        self.run(
            args,
            approval.chat.as_ref(),
            &crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(context),
        )
        .await
    }
}

impl DocumentTool {
    /// Produces the artifact for a turn whose chat routing is `chat` (`None`
    /// outside a chat turn).
    pub(crate) async fn run(
        &self,
        args: Value,
        chat: Option<&crate::security::approval::ApprovalChatContext>,
        _ctx: &crate::core::runtime::DispatchCtx,
    ) -> anyhow::Result<ToolResult> {
        // The document wire contract is `deny_unknown_fields`, so the optional
        // `artifact_id` is read here (and stripped from the args before
        // deserialisation) rather than deserialised into
        // `GenerateDocumentInput` (Phase 2d).
        let artifact_id = args
            .get("artifact_id")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let mut spec_args = args.clone();
        if let Some(obj) = spec_args.as_object_mut() {
            obj.remove("artifact_id");
        }
        let input: GenerateDocumentInput = match serde_json::from_value(spec_args) {
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

        // Reserve the file and open the artifact: a new one, or a new version
        // of `artifact_id` (same id, same file, history kept).
        let production = Production::start(
            &self.workspace_dir,
            self.files_dir.clone(),
            tiny_artifacts::ArtifactKind::File,
            &input.title,
            "docx",
            None,
            artifact_id.as_deref(),
            chat,
        )
        .await
        .map_err(anyhow::Error::msg)?;
        let output_path = production.path().to_path_buf();

        // Persist the verbatim args next to meta.json so a failed card's
        // Retry can re-dispatch this exact spec (#3162). Best-effort: a
        // write failure only forfeits regeneration, never aborts an
        // otherwise-successful generation.
        if let Err(err) =
            crate::artifacts::store::save_artifact_args(&self.workspace_dir, production.id(), &args)
                .await
        {
            tracing::warn!(
                target: "document",
                err = %err,
                artifact_id = %production.id(),
                "[document] failed to persist args.json; artifact will not be regenerable"
            );
        }

        let bytes = match engine::generate(&input, GENERATION_TIMEOUT).await {
            Ok(bytes) => bytes,
            Err(err) => {
                production.fail(&err.to_string()).await;
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
            production.fail(&reason).await;
            tracing::warn!(
                target: "document",
                err = %err,
                artifact_id = %production.id(),
                filename = %filename,
                "[document] artifact file write failed"
            );
            return Ok(ToolResult::error(reason));
        }

        // Record the file as the artifact's newest version; the card turns
        // ready from that record.
        let size_bytes = match production.complete().await {
            Ok(size) => size,
            Err(reason) => {
                tracing::warn!(
                    target: "document",
                    artifact_id = %production.id(),
                    "[document] {reason}"
                );
                return Ok(ToolResult::error(reason));
            }
        };

        tracing::info!(
            target: "document",
            artifact_id = %production.id(),
            size_bytes,
            section_count = input.sections.len(),
            "[document] generation complete"
        );

        let out = GenerateDocumentOutput {
            artifact_id: production.id().to_string(),
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
