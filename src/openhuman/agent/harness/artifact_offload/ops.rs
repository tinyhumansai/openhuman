//! Write, pointer-render, and handoff plumbing for the artifact-offload
//! convention (#3883).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::openhuman::memory::store::safety::sanitize_text;
use crate::openhuman::security::SecurityPolicy;

use super::paths::{relative_to_action_dir, resolve_artifact_path, sanitize_component};
use super::types::{
    ArtifactKind, OffloadError, OffloadedArtifact, ABSTRACT_BUDGET_CHARS, ARTIFACT_POINTER_PREFIX,
};

/// Whether a payload of `bytes` should be offloaded at `threshold_bytes`.
///
/// A zero threshold disables offload entirely (used to opt a run out without
/// threading an `Option` through every call site).
pub fn should_offload(bytes: usize, threshold_bytes: usize) -> bool {
    threshold_bytes > 0 && bytes > threshold_bytes
}

/// Condense `content` down to at most `budget_chars` characters for the
/// pointer's abstract.
///
/// Prefers cutting at a line break, then at a word break, so the parent reads a
/// whole thought rather than a word sliced in half. Only falls back to a hard
/// character cut when neither boundary sits in the back half of the budget.
pub fn build_abstract(content: &str, budget_chars: usize) -> String {
    let trimmed = content.trim();
    if budget_chars == 0 {
        return String::new();
    }
    if trimmed.chars().count() <= budget_chars {
        return trimmed.to_string();
    }

    let mut head: String = trimmed.chars().take(budget_chars).collect();
    let floor = head.len() / 2;
    if let Some(idx) = head.rfind('\n').filter(|idx| *idx >= floor) {
        head.truncate(idx);
    } else if let Some(idx) = head.rfind(' ').filter(|idx| *idx >= floor) {
        head.truncate(idx);
    }
    format!("{}...", head.trim_end())
}

/// Render the text a worker hands back in place of an offloaded payload.
///
/// The first line is the machine-readable pointer ([`extract_artifact_paths`]
/// parses it); the rest is for the model reading the handoff.
pub fn render_artifact_pointer(artifact: &OffloadedArtifact, abstract_text: &str) -> String {
    let redaction_note = if artifact.redacted {
        " Credential/PII redaction was applied before storage."
    } else {
        ""
    };
    format!(
        "{ARTIFACT_POINTER_PREFIX} kind={kind} path={path} bytes={bytes}\n\
         read_with: file_read {{\"path\":\"{path}\"}}\n\
         note: The full result was written to the action workspace instead of being inlined. \
         Read the file for complete fidelity; the abstract below is not exhaustive.{redaction_note}\n\n\
         [abstract]\n{abstract_text}",
        kind = artifact.kind.as_str(),
        path = artifact.relative_path,
        bytes = artifact.stored_bytes,
    )
}

/// Pull every artifact path out of a handoff payload.
///
/// Scans for [`ARTIFACT_POINTER_PREFIX`] lines and returns their `path=` values
/// in encounter order, de-duplicated. A worker that inlined a pointer by hand
/// (following the prompt contract rather than being offloaded by the harness)
/// is picked up by exactly the same parse.
pub fn extract_artifact_paths(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim_start();
        if !line.starts_with(ARTIFACT_POINTER_PREFIX) {
            continue;
        }
        let Some(rest) = line.split(" path=").nth(1) else {
            continue;
        };
        // Split on the FIRST whitespace rather than `split_whitespace`, which
        // skips leading separators: an empty `path=` followed by another field
        // would otherwise yield that next field (`bytes=1`) as the "path".
        let path = rest.split(char::is_whitespace).next().unwrap_or_default();
        if path.is_empty() || found.iter().any(|existing| existing == path) {
            continue;
        }
        found.push(path.to_string());
    }
    found
}

/// Emit an `[artifact]` reference log for every path crossing a handoff, and
/// return how many were surfaced.
///
/// `stage` distinguishes the two ends of the same pointer so a run journal can
/// tell them apart instead of showing the identical line twice: the child
/// *recording* paths onto its outcome, and the parent *consuming* them. Use
/// [`HANDOFF_STAGE_RECORDED`] / [`HANDOFF_STAGE_CONSUMED`].
pub fn note_artifact_handoff(
    stage: &str,
    agent_id: &str,
    task_id: &str,
    paths: &[String],
) -> usize {
    for path in paths {
        tracing::info!(
            stage = %stage,
            agent_id = %agent_id,
            task_id = %task_id,
            path = %path,
            "[artifact] handoff carried an artifact path"
        );
    }
    paths.len()
}

/// Producing side: the child recorded these paths onto its outcome.
pub const HANDOFF_STAGE_RECORDED: &str = "recorded_by_child";

/// Consuming side: the parent took delivery of these paths.
pub const HANDOFF_STAGE_CONSUMED: &str = "consumed_by_parent";

/// Effective offload threshold for an agent whose definition caps its result at
/// `max_result_chars`.
///
/// A cap below the default would otherwise truncate the result before offload
/// ever fired — `flow_memory_agent` caps at 4 000 chars, `context_scout` at
/// 5 000, several built-ins at 8 000, all under the 8 KiB default. Offloading at
/// the tighter of the two means anything the cap would have cut is on disk
/// first. Chars are treated as a byte budget, which is conservative for
/// multibyte text (it offloads slightly earlier, never later).
pub fn effective_offload_threshold(
    default_threshold_bytes: usize,
    max_result_chars: Option<usize>,
) -> usize {
    match max_result_chars {
        Some(cap) if cap > 0 => default_threshold_bytes.min(cap),
        _ => default_threshold_bytes,
    }
}

/// Per-run writer for the offload convention.
///
/// Holds the resolved `action_dir`, the security policy used for the
/// fail-closed workspace checks, and the identifiers that name generated
/// artifacts and tag the `[artifact]` logs.
#[derive(Debug, Clone)]
pub struct ArtifactOffload {
    action_dir: PathBuf,
    /// Root the handed-back path is rendered against. Normally identical to
    /// `action_dir`; see [`Self::with_render_root`].
    render_root: PathBuf,
    policy: Option<Arc<SecurityPolicy>>,
    agent_id: String,
    task_id: String,
}

impl ArtifactOffload {
    pub fn new(
        action_dir: PathBuf,
        policy: Option<Arc<SecurityPolicy>>,
        agent_id: impl Into<String>,
        task_id: impl Into<String>,
    ) -> Self {
        Self {
            render_root: action_dir.clone(),
            action_dir,
            policy,
            agent_id: agent_id.into(),
            task_id: task_id.into(),
        }
    }

    /// Render handed-back paths relative to `render_root` instead of the write
    /// root.
    ///
    /// A worktree-isolated worker writes inside its own checkout, but the parent
    /// that receives the pointer no longer holds that worker's
    /// `WorkspaceDescriptor` — a bare `outputs/…` would resolve against the
    /// *parent's* action root and miss the file entirely. Rendering against the
    /// parent's root yields a relative path when the worktree is nested inside
    /// it, and falls back to an absolute path when it is not, so the pointer is
    /// never silently wrong.
    pub fn with_render_root(mut self, render_root: PathBuf) -> Self {
        self.render_root = render_root;
        self
    }

    /// Convention-stable name for a worker's offloaded final result:
    /// `<agent_id>/<task_id>-result.md` with both components sanitized.
    pub fn default_result_name(&self) -> String {
        format!(
            "{}/{}-result.md",
            sanitize_component(&self.agent_id),
            sanitize_component(&self.task_id)
        )
    }

    /// Resolve `relative` under this run's `action_dir` without writing.
    /// Exposed so callers can validate a model-supplied path before acting on
    /// it.
    pub fn resolve(&self, kind: ArtifactKind, relative: &str) -> Result<PathBuf, OffloadError> {
        resolve_artifact_path(&self.action_dir, self.policy.as_deref(), kind, relative)
    }

    /// Write `content` to `relative` under the convention directory for `kind`.
    ///
    /// The body is passed through credential/PII redaction before it touches
    /// disk, matching how oversized tool results are persisted.
    pub async fn write(
        &self,
        kind: ArtifactKind,
        relative: &str,
        content: &str,
    ) -> Result<OffloadedArtifact, OffloadError> {
        self.write_returning_stored(kind, relative, content)
            .await
            .map(|(artifact, _stored)| artifact)
    }

    /// Same as [`Self::write`], but also hands back the **stored** (redacted)
    /// body.
    ///
    /// Callers that surface any part of the artifact back into the model's
    /// context must render it from this value, never from their own input:
    /// building a preview from the raw text would re-expose exactly the
    /// credentials `sanitize_text` just scrubbed out of the file.
    pub async fn write_returning_stored(
        &self,
        kind: ArtifactKind,
        relative: &str,
        content: &str,
    ) -> Result<(OffloadedArtifact, String), OffloadError> {
        let absolute = self.resolve(kind, relative)?;
        if let Some(parent) = absolute.parent() {
            tokio::fs::create_dir_all(parent).await?;
            // The checks in `resolve` are lexical, so a pre-existing symlink
            // (`outputs -> /elsewhere`, or `outputs/x -> <workspace_dir>`) would
            // still be followed by the write below. Re-validate the parent that
            // actually materialised on disk before touching it.
            self.assert_real_parent_inside_root(kind, parent).await?;
        }
        let sanitized = sanitize_text(content);
        tokio::fs::write(&absolute, sanitized.value.as_bytes()).await?;

        let relative_path = relative_to_action_dir(&self.render_root, &absolute);
        let artifact = OffloadedArtifact {
            kind,
            relative_path,
            absolute_path: absolute,
            stored_bytes: sanitized.value.len(),
            original_bytes: content.len(),
            redacted: sanitized.report.changed(),
        };

        tracing::info!(
            agent_id = %self.agent_id,
            task_id = %self.task_id,
            kind = artifact.kind.as_str(),
            path = %artifact.relative_path,
            original_bytes = artifact.original_bytes,
            stored_bytes = artifact.stored_bytes,
            redacted = artifact.redacted,
            "[artifact] wrote worker artifact under action_dir"
        );

        Ok((artifact, sanitized.value))
    }

    /// Resolve `parent` through symlinks and confirm it still sits inside this
    /// kind's convention root (and outside `workspace_dir`).
    ///
    /// `resolve_artifact_path` cannot do this: its target usually does not
    /// exist yet, so it is lexical by necessity. Once `create_dir_all` has run
    /// the parent *does* exist, which is the first moment the real, link-
    /// resolved location can be checked.
    async fn assert_real_parent_inside_root(
        &self,
        kind: ArtifactKind,
        parent: &Path,
    ) -> Result<(), OffloadError> {
        let root = self.action_dir.join(kind.subdir());
        let real_parent = tokio::fs::canonicalize(parent).await?;
        // The root itself may be reached through a symlinked `action_dir` (macOS
        // `/tmp` -> `/private/tmp` is the everyday case), so compare like with
        // like rather than against the lexical root.
        let real_root = tokio::fs::canonicalize(&root).await?;
        if !real_parent.starts_with(&real_root) {
            return Err(OffloadError::SymlinkEscape {
                path: parent.display().to_string(),
                resolved: real_parent.display().to_string(),
            });
        }
        if let Some(policy) = self.policy.as_deref() {
            if policy.is_workspace_internal_path(&real_parent)
                || tokio::fs::canonicalize(&policy.workspace_dir)
                    .await
                    .map(|ws| real_parent.starts_with(&ws))
                    .unwrap_or(false)
            {
                return Err(OffloadError::SymlinkEscape {
                    path: parent.display().to_string(),
                    resolved: real_parent.display().to_string(),
                });
            }
        }
        Ok(())
    }

    /// Action directory this writer is rooted at.
    pub fn action_dir(&self) -> &Path {
        &self.action_dir
    }
}

/// Offload `output` when it exceeds `threshold_bytes`, returning the text the
/// parent should receive plus the artifact when one was written.
///
/// This is the deterministic half of the convention: it fires whether or not
/// the worker followed the prompt contract. Every failure mode is soft, the
/// caller gets the original payload back and the existing summarizer detour and
/// tool-result budget stay in charge as the fallback.
pub async fn offload_oversized_result(
    output: String,
    offload: &ArtifactOffload,
    threshold_bytes: usize,
) -> (String, Option<OffloadedArtifact>) {
    if !should_offload(output.len(), threshold_bytes) {
        return (output, None);
    }

    let name = offload.default_result_name();
    match offload
        .write_returning_stored(ArtifactKind::Output, &name, &output)
        .await
    {
        Ok((artifact, stored)) => {
            // Build the abstract from the STORED (redacted) body, never from
            // `output`. The pointer goes straight into the parent's context, so
            // rendering it from the raw text would re-expose the very
            // credentials `write` just scrubbed out of the file.
            let abstract_text = build_abstract(&stored, ABSTRACT_BUDGET_CHARS);
            let pointer = render_artifact_pointer(&artifact, &abstract_text);
            tracing::info!(
                path = %artifact.relative_path,
                inline_bytes = output.len(),
                pointer_bytes = pointer.len(),
                threshold_bytes,
                "[artifact] replaced oversized worker result with a path + abstract"
            );
            (pointer, Some(artifact))
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                inline_bytes = output.len(),
                threshold_bytes,
                "[artifact] offload refused; keeping the inline result (summarizer/truncation backstop applies)"
            );
            (output, None)
        }
    }
}
