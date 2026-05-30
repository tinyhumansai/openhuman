//! Tool: `generate_presentation` — build a `.pptx` deck from a
//! structured slide spec via a bundled `python-pptx` helper.
//!
//! Flow:
//! 1. Validate the JSON-Schema input early (`types::validate_input`)
//!    so the agent gets a structured `InvalidInput` it can self-correct
//!    on instead of a python-pptx traceback.
//! 2. Allocate an artifact dir via `artifacts::create_artifact`. The
//!    returned `meta` starts at `ArtifactStatus::Pending` so an
//!    interrupted run never surfaces as Ready.
//! 3. Materialise the bundled `generate_pptx.py` script
//!    (`script::materialise_script` — cached after first call).
//! 4. Invoke Python via the [`invoker::PythonInvoker`] seam (mockable
//!    for unit tests). Production path goes through
//!    `runtime_python::venv::ensure_venv` so `python-pptx` is
//!    installed in a managed venv on first call and reused after.
//! 5. On success: read final file size, flip artifact to `Ready` via
//!    `artifacts::finalize_artifact`, return the artifact id + path.
//! 6. On failure: flip artifact to `Failed` via
//!    `artifacts::fail_artifact` so the UI can surface the reason.
//!
//! Sub-task of #1535 (PR #2778). Orchestrator agent-definition
//! wiring + UX-side error mapping land in #2780; this PR registers
//! the tool globally so CLI / bus / test paths can drive it.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::artifacts::{
    create_artifact, fail_artifact, finalize_artifact, ArtifactKind,
};
use crate::openhuman::config::Config;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

mod invoker;
mod script;
mod types;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use self::invoker::{InvocationOutcome, PythonInvoker, RealPythonInvoker};
use self::types::{
    validate_input, GeneratePresentationInput, GeneratePresentationOutput, PresentationError,
};

/// Generation timeout. Mirrors the multimodal pipeline's PDF cap. A
/// 64-slide deck typically generates in under 5s on a warm venv; the
/// 60s ceiling absorbs cold-cache pip resolution.
const GENERATION_TIMEOUT: Duration = Duration::from_secs(60);

/// Tool name surfaced to the agent. Stable; do not rename without
/// coordinating with #2780's agent definition list.
pub const TOOL_NAME: &str = "generate_presentation";

/// One-shot `.pptx` generator. See module docs for the request flow.
pub struct PresentationTool {
    workspace_dir: PathBuf,
    invoker: Arc<dyn PythonInvoker>,
}

impl PresentationTool {
    /// Production constructor. Builds a [`RealPythonInvoker`] from the
    /// shared `Config` so `runtime_python.cache_dir` / managed
    /// interpreter resolution flow through the canonical bootstrap.
    pub fn new(config: Arc<Config>, workspace_dir: PathBuf) -> Self {
        let runtime_python = config.runtime_python.clone();
        Self {
            workspace_dir,
            invoker: Arc::new(RealPythonInvoker::new(runtime_python)),
        }
    }

    /// Test-only constructor that swaps in a mock invoker. The unit
    /// tests in `tests.rs` use this to cover success / failure /
    /// timeout / missing-runtime branches without a real Python
    /// install.
    #[cfg(test)]
    pub(super) fn with_invoker(workspace_dir: PathBuf, invoker: Arc<dyn PythonInvoker>) -> Self {
        Self {
            workspace_dir,
            invoker,
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
                    "description": "Optional theme hint ('default', 'minimal', 'dark'). Currently informational only — the bundled script uses python-pptx's default template regardless. Reserved for future template selection.",
                },
                "slides": {
                    "type": "array",
                    "description": "Content slides in display order. At least one slide is required.",
                    "minItems": 1,
                    "maxItems": types::MAX_SLIDES,
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "title": { "type": "string", "maxLength": types::MAX_TEXT_CHARS },
                            "body": { "type": "string", "maxLength": types::MAX_TEXT_CHARS },
                            "bullets": {
                                "type": "array",
                                "items": { "type": "string", "maxLength": types::MAX_TEXT_CHARS },
                                "maxItems": types::MAX_BULLETS_PER_SLIDE,
                            },
                            "speaker_notes": { "type": "string", "maxLength": types::MAX_TEXT_CHARS },
                        },
                    },
                },
            },
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // We write files to the workspace artifacts dir and run a
        // child process. Treat as Write rather than ReadOnly.
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

        // Wrap setup + invocation so any failure flips the artifact
        // from Pending to Failed instead of leaving it stuck in
        // Pending and surfacing as a partial deck downstream.
        let outcome = match async {
            let script_path = script::materialise_script().await?;
            let stdin_payload = serde_json::to_vec(&input)?;
            self.invoker
                .run(
                    &script_path,
                    stdin_payload,
                    &output_path,
                    GENERATION_TIMEOUT,
                )
                .await
        }
        .await
        {
            Ok(outcome) => outcome,
            Err(err) => {
                let reason = format!("presentation generation setup failed: {err}");
                let _ = fail_artifact(&self.workspace_dir, &meta.id, &reason).await;
                tracing::warn!(
                    target: "presentation",
                    err = %err,
                    "[presentation] setup/invocation failed before subprocess completion"
                );
                return Ok(ToolResult::error(reason));
            }
        };

        match outcome {
            InvocationOutcome::Success { stdout, stderr } => {
                let size_bytes = match tokio::fs::metadata(&output_path).await {
                    Ok(md) => md.len(),
                    Err(err) => {
                        let reason = format!(
                            "python script exited 0 but no output file at {}: {err}",
                            output_path.display()
                        );
                        let _ = fail_artifact(&self.workspace_dir, &meta.id, &reason).await;
                        return Ok(ToolResult::error(reason));
                    }
                };
                let updated = finalize_artifact(&self.workspace_dir, &meta.id, size_bytes)
                    .await
                    .map_err(anyhow::Error::msg)?;
                tracing::info!(
                    target: "presentation",
                    artifact_id = %updated.id,
                    size_bytes,
                    stderr_chars = stderr.chars().count(),
                    "[presentation] generation complete"
                );
                let _ = stdout; // currently unused — script status JSON is informational
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
            InvocationOutcome::NonZeroExit {
                exit_code,
                stdout: _,
                stderr,
            } => {
                let err = PresentationError::GenerationFailed {
                    exit_code,
                    stderr_truncated: PresentationError::truncate_stderr(&stderr),
                };
                let _ = fail_artifact(&self.workspace_dir, &meta.id, &err.to_string()).await;
                tracing::warn!(
                    target: "presentation",
                    exit_code,
                    stderr_chars = stderr.chars().count(),
                    "[presentation] python-pptx generation failed"
                );
                Ok(ToolResult::error(err.to_string()))
            }
            InvocationOutcome::Timeout { timeout_secs } => {
                let err = PresentationError::GenerationTimeout { timeout_secs };
                let _ = fail_artifact(&self.workspace_dir, &meta.id, &err.to_string()).await;
                tracing::warn!(
                    target: "presentation",
                    timeout_secs,
                    "[presentation] python-pptx generation exceeded timeout"
                );
                Ok(ToolResult::error(err.to_string()))
            }
            InvocationOutcome::MissingRuntime { reason } => {
                let err = PresentationError::MissingRuntime {
                    name: "python".to_string(),
                    install_hint: format!(
                        "ensure runtime_python is enabled and a Python 3.12+ interpreter is reachable (underlying: {reason})"
                    ),
                };
                let _ = fail_artifact(&self.workspace_dir, &meta.id, &err.to_string()).await;
                tracing::warn!(
                    target: "presentation",
                    reason = %reason,
                    "[presentation] runtime python unavailable"
                );
                Ok(ToolResult::error(err.to_string()))
            }
            InvocationOutcome::PackageInstallFailed { reason } => {
                let err = PresentationError::MissingPackage {
                    name: "python-pptx".to_string(),
                    install_hint: format!(
                        "first-call venv setup failed; check network connectivity and pip availability (underlying: {reason})"
                    ),
                };
                let _ = fail_artifact(&self.workspace_dir, &meta.id, &err.to_string()).await;
                tracing::warn!(
                    target: "presentation",
                    reason = %reason,
                    "[presentation] python-pptx package install failed"
                );
                Ok(ToolResult::error(err.to_string()))
            }
        }
    }
}
