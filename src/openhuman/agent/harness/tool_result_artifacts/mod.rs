//! Persist oversized tool outputs as action-workspace artifacts.
//!
//! Tool results enter the model context before the provider has seen them, so
//! this is the last cheap point to replace large raw output with a bounded
//! preview. The full, scrubbed body is written under `action_dir` so normal
//! file-reading tools can inspect it later without exposing internal workspace
//! state.

use std::path::{Path, PathBuf};

use crate::openhuman::agent::dispatcher::ToolExecutionResult;
use crate::openhuman::context::{apply_tool_result_budget, BudgetOutcome};
use crate::openhuman::memory_store::safety::{sanitize_text, SanitizationReport};

const ARTIFACT_ROOT: &str = "artifacts/tool-results";
const AGGREGATE_PREVIEW_BUDGET_BYTES: usize = 512;

#[derive(Debug, Clone)]
pub(crate) struct ToolResultArtifactStore {
    action_dir: PathBuf,
    session_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PersistedToolResult {
    pub output: String,
    pub path: String,
    pub original_bytes: usize,
    pub stored_bytes: usize,
    pub redactions: SanitizationReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ToolResultArtifactOutcome {
    pub original_bytes: usize,
    pub final_bytes: usize,
    pub persisted: bool,
}

impl ToolResultArtifactOutcome {
    pub fn unchanged(len: usize) -> Self {
        Self {
            original_bytes: len,
            final_bytes: len,
            persisted: false,
        }
    }
}

impl ToolResultArtifactStore {
    pub(crate) fn new(action_dir: PathBuf, session_key: impl Into<String>) -> Self {
        Self {
            action_dir,
            session_key: sanitize_component(&session_key.into()),
        }
    }

    pub(crate) fn path_for_read_tool(&self, tool_name: &str, call_id: Option<&str>) -> String {
        let call = call_id
            .map(sanitize_component)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
        format!(
            "{ARTIFACT_ROOT}/{}/{}/{}.txt",
            self.session_key,
            sanitize_component(tool_name),
            call
        )
    }

    async fn persist(
        &self,
        tool_name: &str,
        call_id: Option<&str>,
        content: &str,
        preview_budget_bytes: usize,
        reason: &str,
    ) -> anyhow::Result<PersistedToolResult> {
        let sanitized = sanitize_text(content);
        let relative_path = self.path_for_read_tool(tool_name, call_id);
        let absolute_path = self.action_dir.join(&relative_path);
        assert_within_action_dir(&self.action_dir, &absolute_path)?;
        if let Some(parent) = absolute_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&absolute_path, sanitized.value.as_bytes()).await?;

        let (preview, preview_outcome) =
            apply_tool_result_budget(sanitized.value.clone(), preview_budget_bytes);
        let redaction_note = if sanitized.report.changed() {
            " Credential/PII redaction was applied before storage and preview exposure."
        } else {
            ""
        };
        let truncation_note = if preview_outcome.truncated {
            format!(
                " Preview is bounded; {} stored bytes are available via file_read.",
                sanitized.value.len()
            )
        } else {
            String::new()
        };

        let envelope = format!(
            "[tool_result_preview]\n\
             tool: {tool_name}\n\
             reason: {reason}\n\
             original_bytes: {}\n\
             stored_bytes: {}\n\
             artifact_path: {relative_path}\n\
             read_with: file_read {{\"path\":\"{relative_path}\"}}\n\
             notes: Full scrubbed output was persisted under the action workspace.{redaction_note}{truncation_note}\n\n\
             [preview]\n{preview}",
            content.len(),
            sanitized.value.len(),
        );

        Ok(PersistedToolResult {
            output: envelope,
            path: relative_path,
            original_bytes: content.len(),
            stored_bytes: sanitized.value.len(),
            redactions: sanitized.report,
        })
    }
}

pub(crate) async fn apply_per_result_persistence(
    content: String,
    store: Option<&ToolResultArtifactStore>,
    tool_name: &str,
    call_id: Option<&str>,
    budget_bytes: usize,
) -> (String, ToolResultArtifactOutcome) {
    let original_bytes = content.len();
    if budget_bytes == 0 || original_bytes <= budget_bytes {
        return (
            content,
            ToolResultArtifactOutcome::unchanged(original_bytes),
        );
    }

    if let Some(store) = store {
        match store
            .persist(
                tool_name,
                call_id,
                &content,
                budget_bytes,
                "per-result budget exceeded",
            )
            .await
        {
            Ok(persisted) => {
                let (output, final_bytes) = bound_text_to_budget(persisted.output, budget_bytes);
                if final_bytes >= original_bytes {
                    log::debug!(
                        "[agent][tool-result-artifacts] persisted envelope too large tool={} original_bytes={} final_bytes={} budget_bytes={} -- falling back to inline truncation",
                        tool_name,
                        original_bytes,
                        final_bytes,
                        budget_bytes
                    );
                }
                log::info!(
                    "[agent][tool-result-artifacts] persisted oversized tool result tool={} original_bytes={} stored_bytes={} path={} redacted={}",
                    tool_name,
                    persisted.original_bytes,
                    persisted.stored_bytes,
                    persisted.path,
                    persisted.redactions.changed()
                );
                return (
                    output,
                    ToolResultArtifactOutcome {
                        original_bytes,
                        final_bytes,
                        persisted: true,
                    },
                );
            }
            Err(err) => {
                log::warn!(
                    "[agent][tool-result-artifacts] persist failed tool={} original_bytes={} err={} — falling back to inline truncation",
                    tool_name,
                    original_bytes,
                    err
                );
            }
        }
    }

    let (output, BudgetOutcome { final_bytes, .. }) =
        apply_tool_result_budget(content, budget_bytes);
    (
        output,
        ToolResultArtifactOutcome {
            original_bytes,
            final_bytes,
            persisted: false,
        },
    )
}

pub(crate) async fn spill_aggregate_tool_results(
    results: &mut [ToolExecutionResult],
    store: Option<&ToolResultArtifactStore>,
    budget_bytes: usize,
) {
    if budget_bytes == 0 {
        return;
    }
    let Some(store) = store else {
        return;
    };

    let mut total: usize = results.iter().map(|result| result.output.len()).sum();
    if total <= budget_bytes {
        return;
    }

    let mut indexes: Vec<usize> = (0..results.len()).collect();
    indexes.sort_by_key(|idx| std::cmp::Reverse(results[*idx].output.len()));

    for idx in indexes {
        if total <= budget_bytes {
            break;
        }
        let original = results[idx].output.clone();
        let original_len = original.len();
        let allowed_len = budget_bytes.saturating_sub(total.saturating_sub(original_len));
        let persisted_output = if looks_like_preview_envelope(&original) {
            Ok(PersistedToolResult {
                output: original.clone(),
                path: "<existing-preview>".to_string(),
                original_bytes: original_len,
                stored_bytes: original_len,
                redactions: SanitizationReport::default(),
            })
        } else {
            store
                .persist(
                    &results[idx].name,
                    results[idx].tool_call_id.as_deref(),
                    &original,
                    allowed_len.min(AGGREGATE_PREVIEW_BUDGET_BYTES),
                    "aggregate tool-result budget exceeded",
                )
                .await
        };
        match persisted_output {
            Ok(persisted) => {
                let (output, final_bytes) = bound_text_to_budget(persisted.output, allowed_len);
                total = total
                    .saturating_sub(original_len)
                    .saturating_add(final_bytes);
                log::info!(
                    "[agent][tool-result-artifacts] aggregate spill tool={} original_bytes={} final_bytes={} total_bytes={} path={}",
                    results[idx].name,
                    original_len,
                    final_bytes,
                    total,
                    persisted.path
                );
                results[idx].output = output;
            }
            Err(err) => {
                log::warn!(
                    "[agent][tool-result-artifacts] aggregate spill failed tool={} bytes={} err={} -- falling back to inline budget trim",
                    results[idx].name,
                    original_len,
                    err
                );
                let (output, final_bytes) = bound_text_to_budget(original, allowed_len);
                total = total
                    .saturating_sub(original_len)
                    .saturating_add(final_bytes);
                results[idx].output = output;
            }
        }
    }
}

fn looks_like_preview_envelope(value: &str) -> bool {
    value.starts_with("[tool_result_preview]\n")
}

fn bound_text_to_budget(content: String, budget_bytes: usize) -> (String, usize) {
    if budget_bytes == 0 {
        return (String::new(), 0);
    }
    let (mut output, BudgetOutcome { final_bytes, .. }) =
        apply_tool_result_budget(content, budget_bytes);
    if final_bytes <= budget_bytes {
        return (output, final_bytes);
    }
    let cut = crate::openhuman::util::floor_char_boundary(&output, budget_bytes);
    output.truncate(cut);
    let final_bytes = output.len();
    (output, final_bytes)
}

fn sanitize_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len().min(80));
    for ch in value.chars().take(80) {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "unknown".to_string()
    } else {
        out
    }
}

fn assert_within_action_dir(action_dir: &Path, path: &Path) -> anyhow::Result<()> {
    if path.starts_with(action_dir) {
        return Ok(());
    }
    anyhow::bail!(
        "tool-result artifact path escaped action_dir: {}",
        path.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};
    use crate::openhuman::tools::traits::Tool;
    use crate::openhuman::tools::FileReadTool;
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn threshold_persists_preview_and_readable_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ToolResultArtifactStore::new(tmp.path().to_path_buf(), "session/one");
        let raw = format!(
            "{} {}",
            "x".repeat(4096),
            "ghp_abcdefghijklmnopqrstuvwxyz123456"
        );

        let (out, outcome) =
            apply_per_result_persistence(raw.clone(), Some(&store), "shell", Some("call-1"), 1024)
                .await;

        assert!(outcome.persisted);
        assert!(out.contains("artifact_path: artifacts/tool-results/session_one/shell/call-1.txt"));
        assert!(out.contains("original_bytes:"));
        assert!(out.contains("[preview]"));
        assert!(!out.contains("ghp_abcdefghijklmnopqrstuvwxyz123456"));

        let policy = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            action_dir: tmp.path().to_path_buf(),
            workspace_dir: tmp.path().to_path_buf(),
            ..SecurityPolicy::default()
        });
        let reader = FileReadTool::new(policy);
        let read = reader
            .execute(json!({"path": "artifacts/tool-results/session_one/shell/call-1.txt"}))
            .await
            .unwrap();
        assert!(!read.is_error, "{}", read.output());
        assert!(read.output().contains("xxxx"));
        assert!(!read
            .output()
            .contains("ghp_abcdefghijklmnopqrstuvwxyz123456"));
    }

    #[tokio::test]
    async fn fallback_truncates_when_store_missing() {
        let raw = "z".repeat(4096);
        let (out, outcome) =
            apply_per_result_persistence(raw, None, "shell", Some("call"), 512).await;
        assert!(!outcome.persisted);
        assert!(out.contains("truncated by tool_result_budget"));
        assert!(out.len() < 4096);
    }

    #[tokio::test]
    async fn persisted_preview_is_bounded_for_small_budget() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ToolResultArtifactStore::new(tmp.path().to_path_buf(), "session");
        let raw = "x".repeat(800);

        let (out, outcome) =
            apply_per_result_persistence(raw, Some(&store), "shell", Some("call"), 320).await;

        assert!(outcome.persisted);
        assert!(outcome.final_bytes <= 320, "final={}", outcome.final_bytes);
        assert_eq!(out.len(), outcome.final_bytes);
        assert!(out.contains("[tool_result_preview]"));
        assert!(tmp
            .path()
            .join("artifacts/tool-results/session/shell/call.txt")
            .exists());
    }

    #[tokio::test]
    async fn aggregate_spills_largest_until_under_budget() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ToolResultArtifactStore::new(tmp.path().to_path_buf(), "session");
        let mut results = vec![
            ToolExecutionResult {
                name: "small".into(),
                output: "a".repeat(100),
                success: true,
                tool_call_id: Some("small".into()),
            },
            ToolExecutionResult {
                name: "largest".into(),
                output: "b".repeat(2000),
                success: true,
                tool_call_id: Some("largest".into()),
            },
            ToolExecutionResult {
                name: "medium".into(),
                output: "c".repeat(900),
                success: true,
                tool_call_id: Some("medium".into()),
            },
        ];

        spill_aggregate_tool_results(&mut results, Some(&store), 1800).await;

        assert!(results[1].output.starts_with("[tool_result_preview]\n"));
        let total: usize = results.iter().map(|result| result.output.len()).sum();
        assert!(total <= 1800, "total={total}");
        assert!(!results[0].output.starts_with("[tool_result_preview]\n"));
        assert!(tmp
            .path()
            .join("artifacts/tool-results/session/largest/largest.txt")
            .exists());
    }

    #[tokio::test]
    async fn aggregate_forces_budget_when_envelope_has_no_savings() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ToolResultArtifactStore::new(tmp.path().to_path_buf(), "session");
        let mut results = vec![
            ToolExecutionResult {
                name: "one".into(),
                output: "a".repeat(350),
                success: true,
                tool_call_id: Some("one".into()),
            },
            ToolExecutionResult {
                name: "two".into(),
                output: "b".repeat(350),
                success: true,
                tool_call_id: Some("two".into()),
            },
            ToolExecutionResult {
                name: "three".into(),
                output: "c".repeat(350),
                success: true,
                tool_call_id: Some("three".into()),
            },
        ];

        spill_aggregate_tool_results(&mut results, Some(&store), 500).await;

        let total: usize = results.iter().map(|result| result.output.len()).sum();
        assert!(total <= 500, "total={total}");
        assert!(tmp
            .path()
            .join("artifacts/tool-results/session/one/one.txt")
            .exists());
    }
}
