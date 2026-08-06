//! Domain types for the worker artifact-offload convention (#3883).

use std::path::PathBuf;

/// Deliverables directory under `action_dir`. Artifacts here are meant to
/// outlive the step that produced them and to be handed to a parent agent or a
/// later step by **path**, not by value.
pub const OUTPUTS_DIR: &str = "outputs";

/// Scratch directory under `action_dir`. Intermediate files a worker needs
/// while it works but does not intend to hand back.
pub const SCRATCH_DIR: &str = "workspace";

/// Byte threshold above which a worker's final result is written to
/// `outputs/` and replaced by a pointer + abstract.
///
/// ~2 000 tokens at the harness-wide 4-chars-per-token estimate (the same
/// heuristic used by `tinyagents::payload_summarizer` and
/// `subagent_runner::handoff`). Below this, inlining is cheaper than a file
/// round-trip plus the pointer envelope.
pub const DEFAULT_OFFLOAD_THRESHOLD_BYTES: usize = 8_192;

/// Characters of the offloaded body reproduced as the abstract in the pointer.
/// Enough for a parent to decide whether to `file_read` the full artifact.
pub const ABSTRACT_BUDGET_CHARS: usize = 600;

/// Line prefix every pointer line carries. Grep-friendly and the anchor
/// [`super::extract_artifact_paths`] keys off when reading a handoff.
pub const ARTIFACT_POINTER_PREFIX: &str = "[artifact]";

/// Which of the two convention directories an artifact belongs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    /// A deliverable, written under `action_dir/outputs/`.
    Output,
    /// Scratch, written under `action_dir/workspace/`.
    ///
    /// Note this is `action_dir/workspace`, which is NOT the core's internal
    /// `workspace_dir`. The offload resolver refuses to place an artifact under
    /// `workspace_dir` regardless of kind.
    Scratch,
}

impl ArtifactKind {
    /// Directory name under `action_dir` for this kind.
    pub fn subdir(self) -> &'static str {
        match self {
            Self::Output => OUTPUTS_DIR,
            Self::Scratch => SCRATCH_DIR,
        }
    }

    /// Stable, log-friendly label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Output => "output",
            Self::Scratch => "scratch",
        }
    }
}

/// A worker artifact that was successfully written to disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffloadedArtifact {
    /// Which convention directory it landed in.
    pub kind: ArtifactKind,
    /// Path relative to `action_dir`, always `/`-separated so it can be pasted
    /// straight into a `file_read` call.
    pub relative_path: String,
    /// Absolute path on disk.
    pub absolute_path: PathBuf,
    /// Bytes actually stored (post-redaction).
    pub stored_bytes: usize,
    /// Bytes of the caller's original payload (pre-redaction).
    pub original_bytes: usize,
    /// Whether credential/PII redaction rewrote the body before storage.
    pub redacted: bool,
}

/// Why an offload was refused. Every variant is non-fatal at the call site:
/// the caller keeps the inline payload and the summarizer / truncation
/// backstops still apply.
#[derive(Debug, thiserror::Error)]
pub enum OffloadError {
    /// The requested relative path was empty or whitespace-only.
    #[error("artifact name is empty")]
    EmptyName,

    /// The relative path was absolute (or carried a Windows drive/UNC prefix).
    #[error("artifact path must be relative to action_dir, got {path}")]
    AbsolutePath { path: String },

    /// The relative path escaped its convention directory (`..` traversal).
    #[error("artifact path escapes {root}: {path}")]
    PathEscape { root: String, path: String },

    /// The resolved path landed inside the core's internal `workspace_dir`.
    /// Fail-closed: offload targets resolve under `action_dir`, never
    /// `workspace_dir`.
    #[error(
        "artifact path resolves inside workspace_dir, which agent writes may never reach: {path}"
    )]
    WorkspaceTarget { path: String },

    /// The resolved path is an internal workspace state location per
    /// `SecurityPolicy::is_workspace_internal_path`.
    #[error("artifact path is workspace-internal state: {path}")]
    WorkspaceInternal { path: String },

    /// The parent directory that actually materialised on disk resolves, through
    /// symlinks, to somewhere outside the convention root. The lexical checks in
    /// `resolve_artifact_path` cannot see this because the target does not exist
    /// yet when they run.
    #[error("artifact parent escapes its root through a symlink: {path} resolves to {resolved}")]
    SymlinkEscape { path: String, resolved: String },

    /// Creating the parent directory or writing the file failed.
    #[error("artifact write failed: {0}")]
    Io(#[from] std::io::Error),
}
