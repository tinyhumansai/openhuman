//! Persist oversized tool outputs as action-workspace artifacts.
//!
//! Tool results enter the model context before the provider has seen them, so
//! this is the last cheap point to replace large raw output with a bounded
//! preview. The full, scrubbed body is written under `action_dir` so normal
//! file-reading tools can inspect it later without exposing internal workspace
//! state.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::openhuman::agent::dispatcher::ToolExecutionResult;
use crate::openhuman::memory::safety::{sanitize_text, SanitizationReport};
use async_trait::async_trait;
use serde_json::Value;
use tinyagents_harness::store::Store;

const ARTIFACT_ROOT: &str = "artifacts/tool-results";
const AGGREGATE_PREVIEW_BUDGET_BYTES: usize = 512;
/// #4469 item 6: floor for how tightly a persisted `[tool_result_preview]`
/// envelope may be bounded during aggregate spill. `allowed_len` can saturate to
/// `0` (or a handful of bytes) once earlier-spilled results have already consumed
/// the aggregate budget; bounding the envelope to that would return `""` — or a
/// header cut mid-line — discarding the `artifact_path` pointer the model needs
/// to `file_read` the full output. This floor keeps the envelope header (through
/// the `artifact_path` / `read_with` lines) intact even when the raw budget math
/// says zero; `apply_tool_result_budget` retains the head, so the pointer always
/// survives. Slightly overshooting the aggregate budget here is the correct
/// trade — a valid pointer is worth a few hundred bytes.
const MIN_ENVELOPE_ALLOWANCE_BYTES: usize = 512;
pub(crate) const TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE: &str = "openhuman_tool_result_artifacts";
const TRAILER_RESERVED: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BudgetOutcome {
    original_bytes: usize,
    final_bytes: usize,
    truncated: bool,
}

impl BudgetOutcome {
    fn unchanged(len: usize) -> Self {
        Self {
            original_bytes: len,
            final_bytes: len,
            truncated: false,
        }
    }
}

fn apply_tool_result_budget(content: String, budget_bytes: usize) -> (String, BudgetOutcome) {
    let original_bytes = content.len();
    if budget_bytes == 0 || original_bytes <= budget_bytes {
        return (content, BudgetOutcome::unchanged(original_bytes));
    }

    let head_capacity = budget_bytes.saturating_sub(TRAILER_RESERVED).max(1);
    let mut cut = crate::openhuman::util::floor_char_boundary(&content, head_capacity);
    if cut == 0 {
        cut = content
            .char_indices()
            .next()
            .map(|(_, c)| c.len_utf8())
            .unwrap_or(0);
    }

    let dropped_bytes = original_bytes.saturating_sub(cut);
    let mut out = String::with_capacity(cut + TRAILER_RESERVED);
    out.push_str(&content[..cut]);
    out.push_str(&format!(
        "\n\n[… {dropped_bytes} bytes truncated by tool_result_budget — re-run with a narrower query to see the rest …]"
    ));

    let final_bytes = out.len();
    (
        out,
        BudgetOutcome {
            original_bytes,
            final_bytes,
            truncated: true,
        },
    )
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolResultArtifactOutcome {
    pub original_bytes: usize,
    pub final_bytes: usize,
    pub persisted: bool,
    pub artifact_path: Option<String>,
}

impl ToolResultArtifactOutcome {
    pub fn unchanged(len: usize) -> Self {
        Self {
            original_bytes: len,
            final_bytes: len,
            persisted: false,
            artifact_path: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ToolResultArtifactIndexStore {
    data: Arc<Mutex<std::collections::HashMap<String, std::collections::HashMap<String, Value>>>>,
}

impl ToolResultArtifactIndexStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Store for ToolResultArtifactIndexStore {
    async fn get(&self, namespace: &str, key: &str) -> tinyagents_harness::Result<Option<Value>> {
        let guard = self.data.lock().map_err(|_| {
            tinyagents_harness::TinyAgentsError::Memory("tool artifact index poisoned".into())
        })?;
        Ok(guard.get(namespace).and_then(|ns| ns.get(key).cloned()))
    }

    async fn put(
        &self,
        namespace: &str,
        key: &str,
        value: Value,
    ) -> tinyagents_harness::Result<()> {
        let mut guard = self.data.lock().map_err(|_| {
            tinyagents_harness::TinyAgentsError::Memory("tool artifact index poisoned".into())
        })?;
        guard
            .entry(namespace.to_string())
            .or_default()
            .insert(key.to_string(), value);
        Ok(())
    }

    async fn delete(&self, namespace: &str, key: &str) -> tinyagents_harness::Result<()> {
        let mut guard = self.data.lock().map_err(|_| {
            tinyagents_harness::TinyAgentsError::Memory("tool artifact index poisoned".into())
        })?;
        if let Some(ns) = guard.get_mut(namespace) {
            ns.remove(key);
        }
        Ok(())
    }

    async fn list(&self, namespace: &str) -> tinyagents_harness::Result<Vec<String>> {
        let guard = self.data.lock().map_err(|_| {
            tinyagents_harness::TinyAgentsError::Memory("tool artifact index poisoned".into())
        })?;
        Ok(guard
            .get(namespace)
            .map(|ns| ns.keys().cloned().collect())
            .unwrap_or_default())
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
                    // #4469 item 9: this branch does NOT fall back to inline
                    // truncation — the envelope is returned regardless, because it
                    // carries the `artifact_path` pointer to the full stored output
                    // (worth keeping even when the preview text nets no byte saving
                    // vs. the raw result). Log it as an observation only.
                    log::debug!(
                        "[agent][tool-result-artifacts] persisted envelope not smaller than raw result tool={} original_bytes={} final_bytes={} budget_bytes={} -- keeping envelope for its artifact_path pointer",
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
                        artifact_path: Some(persisted.path),
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

    // Reached two ways, and only one of them was audible: a persist that FAILED
    // warns just above, but a run with no artifact store configured falls
    // through to here silently — the oversized tail is discarded with nothing
    // recording that it happened. Say so, so "where did the rest of my search
    // result go" is answerable from the logs rather than by reading this
    // function.
    if store.is_none() {
        log::info!(
            "[agent][tool-result-artifacts] no artifact store configured; truncating oversized tool result inline tool={} original_bytes={} budget_bytes={} — the tail is discarded, not recoverable",
            tool_name,
            original_bytes,
            budget_bytes
        );
    }
    let (output, BudgetOutcome { final_bytes, .. }) =
        apply_tool_result_budget(content, budget_bytes);
    (
        output,
        ToolResultArtifactOutcome {
            original_bytes,
            final_bytes,
            persisted: false,
            artifact_path: None,
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
                // #4469 item 6: never bound the preview envelope below the minimum
                // that preserves its `[tool_result_preview]` header + artifact
                // pointer — `allowed_len` can be 0 here, which would blank the
                // result and strip the `artifact_path` the model reads to recover
                // the full output.
                let envelope_allowance = allowed_len.max(MIN_ENVELOPE_ALLOWANCE_BYTES);
                let (output, final_bytes) =
                    bound_text_to_budget(persisted.output, envelope_allowance);
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
#[path = "mod_tests.rs"]
mod tests;
