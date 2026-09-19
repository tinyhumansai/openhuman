//! Persist oversized tool outputs as action-workspace artifacts.
//!
//! Tool results enter the model context before the provider has seen them, so
//! this is the last cheap point to replace large raw output with a bounded
//! preview. The full, scrubbed body is written under `action_dir` so normal
//! file-reading tools can inspect it later without exposing internal workspace
//! state.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::memory::safety::{SanitizationReport, Sanitized, sanitize_text};
use async_trait::async_trait;
use serde_json::Value;
use tinyagents_harness::store::Store;
use tinytools_agent::dialect::ToolOutcome;

const ARTIFACT_ROOT: &str = "artifacts/tool-results";

/// A read of a persisted artifact, recognised from a tool call's arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtifactRead {
    pub path: String,
    pub offset: usize,
}

/// The only tool whose result is a read of an artifact's content.
const FILE_READ_TOOL: &str = "file_read";

/// The artifact a tool call reads, if any.
///
/// Only a `file_read` counts, because only its result *is* the stored body:
/// `file_write`, `glob`, `list` or `apply_patch` can name an artifact path too,
/// and their output must still take the normal ladder. The read may arrive
/// wrapped in `use_skill`, reported under that name
/// (`use_skill {"skill":"files","tool":"file_read","args":{"path":…}}`), so
/// `use_skill` — and only `use_skill`, the one tool whose result *is* the
/// wrapped tool's result — is followed into the tool it runs (#6284). Any
/// other tool that happens to carry `tool`/`args` fields is not a wrapper.
pub(crate) fn artifact_read_target(tool_name: &str, args: &Value) -> Option<ArtifactRead> {
    if tool_name == crate::tools::toolpacks::USE_SKILL {
        let inner_tool = args.get("tool").and_then(Value::as_str)?;
        return artifact_read_target(inner_tool, args.get("args")?);
    }
    if tool_name != FILE_READ_TOOL {
        return None;
    }
    let path = args.get("path").and_then(Value::as_str)?;
    // A path component match: `artifacts/tool-results-backup/…` shares the
    // prefix but is not the artifact directory.
    let under_root = path
        .trim_start_matches("./")
        .strip_prefix(ARTIFACT_ROOT)
        .is_some_and(|rest| rest.starts_with('/'));
    // An absent or null offset starts at 0. A present one that is not a
    // non-negative integer that fits `usize` is not a read `file_read` serves
    // (it rejects it), so it is not an artifact read either; never reinterpret
    // it as 0.
    let offset = match args.get("offset") {
        None | Some(Value::Null) => 0,
        Some(value) => usize::try_from(value.as_u64()?).ok()?,
    };
    under_root.then(|| ArtifactRead {
        path: path.to_string(),
        offset,
    })
}

/// Bound one page of an artifact read to `budget_bytes`, naming the exact
/// `offset` the next read continues from. Never persists: re-persisting a read
/// of an artifact creates a new artifact whose preview is the same bounded
/// head, and the model can loop between previews without ever reaching the
/// body.
pub(crate) fn page_artifact_read(
    content: String,
    read: &ArtifactRead,
    budget_bytes: usize,
) -> String {
    if budget_bytes == 0 {
        return content;
    }
    // A page is useless without its continuation, so a budget too small to
    // carry one is raised to the floor a persisted envelope already takes for
    // the same reason (`MIN_ENVELOPE_ALLOWANCE_BYTES`). Every page therefore
    // fits `max(budget_bytes, MIN_ENVELOPE_ALLOWANCE_BYTES)` and advances.
    let budget_bytes = budget_bytes.max(MIN_ENVELOPE_ALLOWANCE_BYTES);
    if content.len() <= budget_bytes {
        return content;
    }
    let start = read.offset;
    let Some(total) = start.checked_add(content.len()) else {
        // Only reachable with an offset no real read carries (`file_read`
        // rejects offsets past its at-most-10-MiB file). Bound the result but
        // advertise no continuation, since none could advance.
        let cut = crate::util::floor_char_boundary(&content, budget_bytes);
        return content[..cut].to_string();
    };
    let with_path = |next: usize| {
        format!(
            "\n\n[artifact page: bytes {start}..{next} of {total}. Continue with file_read {{\"path\":\"{}\",\"offset\":{next}}}]",
            read.path
        )
    };
    // Without the path (the caller already has it). At most ~100 bytes, so it
    // always leaves body room under the floor.
    let without_path = |next: usize| {
        format!(
            "\n\n[artifact page: bytes {start}..{next} of {total}. Continue with file_read at \"offset\":{next}]"
        )
    };
    // Sized from the trailer this page will actually carry, not a fixed
    // reservation. `next` never has more digits than `total`, so a trailer
    // rendered with `total` is its longest form.
    let use_path = with_path(total).len() + 4 <= budget_bytes;
    let longest = if use_path {
        with_path(total).len()
    } else {
        without_path(total).len()
    };
    let cut = crate::util::floor_char_boundary(&content, budget_bytes - longest);
    let trailer = if use_path {
        with_path(start + cut)
    } else {
        without_path(start + cut)
    };
    format!("{}{trailer}", &content[..cut])
}
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
    let mut cut = crate::util::floor_char_boundary(&content, head_capacity);
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

    /// Store `content` and return an envelope previewing it. When `content`
    /// would be unreadable once sanitized (see [`readable_body`]) and a
    /// `fallback` is given, the fallback is stored instead; when neither fits,
    /// this errors and the caller truncates inline rather than writing an
    /// artifact nobody can read.
    async fn persist(
        &self,
        tool_name: &str,
        call_id: Option<&str>,
        content: &str,
        fallback: Option<&str>,
        preview_budget_bytes: usize,
        reason: &str,
    ) -> anyhow::Result<PersistedToolResult> {
        let (content, sanitized) = readable_body(
            content,
            fallback,
            crate::tools::FileReadTool::MAX_FILE_SIZE_BYTES,
        )?;
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
             read_with: file_read {{\"path\":\"{relative_path}\"}} (a long read returns one page and names the \"offset\" to continue from)\n\
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

/// The body to store: `primary` if its sanitized form fits `limit`, else
/// `fallback` if *its* sanitized form does, else an error. Both checks are on
/// the sanitized size, the bytes actually written, because redaction can grow a
/// body (`+15551234567` becomes `[REDACTED_PII_PHONE]`): a raw body under the
/// limit can still produce an artifact `file_read` refuses to open.
fn readable_body<'a>(
    primary: &'a str,
    fallback: Option<&'a str>,
    limit: u64,
) -> anyhow::Result<(&'a str, Sanitized<String>)> {
    let sanitized = sanitize_text(primary);
    if sanitized.value.len() as u64 <= limit {
        return Ok((primary, sanitized));
    }
    if let Some(fallback) = fallback {
        let sanitized_fallback = sanitize_text(fallback);
        if sanitized_fallback.value.len() as u64 <= limit {
            return Ok((fallback, sanitized_fallback));
        }
    }
    anyhow::bail!(
        "tool result would not be readable once stored: {} sanitized bytes exceed the {limit}-byte file_read limit",
        sanitized.value.len()
    )
}

/// Persist an over-budget result and return its envelope.
///
/// `full_output` is the tool's output before any earlier stage rewrote it
/// (summarizer, TokenJuice). When given, *that* is what gets stored, so the
/// artifact holds what the tool returned rather than a compacted copy of it;
/// `content` still decides whether the budget was exceeded and is what the
/// model would otherwise have seen.
pub(crate) async fn apply_per_result_persistence(
    content: String,
    full_output: Option<String>,
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
                full_output.as_deref().unwrap_or(&content),
                full_output.as_ref().map(|_| content.as_str()),
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
                        // The size of what was stored, which `full_output` can
                        // make larger than `content`; the artifact index and its
                        // contents list read this number.
                        original_bytes: persisted.original_bytes,
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
    results: &mut [ToolOutcome],
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
                    None,
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
    let cut = crate::util::floor_char_boundary(&output, budget_bytes);
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
