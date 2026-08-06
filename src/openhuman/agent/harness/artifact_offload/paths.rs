//! Path hardening for the artifact-offload convention (#3883).
//!
//! Everything an agent offloads resolves under `action_dir/<outputs|workspace>`.
//! The resolver is deliberately fail-closed: it rejects absolute paths, `..`
//! traversal, anything that lands outside its convention root after lexical
//! normalization, and anything that reaches the core's internal `workspace_dir`
//! (whether or not the specific subdirectory is one of the
//! `is_workspace_internal_path` state locations).

use std::path::{Component, Path, PathBuf};

use crate::openhuman::security::SecurityPolicy;

use super::types::{ArtifactKind, OffloadError};

/// Maximum characters kept from a single path component when deriving a name
/// from an agent id / task id. Keeps generated names well inside filesystem
/// limits on every supported platform.
const MAX_COMPONENT_CHARS: usize = 80;

/// Reduce an arbitrary string to a safe single path component.
///
/// Anything that is not ASCII alphanumeric, `-`, or `_` becomes `_`, so a
/// task id like `sub-1a/2b` or an agent id carrying a path separator can never
/// introduce a directory level of its own.
pub fn sanitize_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len().min(MAX_COMPONENT_CHARS));
    for ch in value.chars().take(MAX_COMPONENT_CHARS) {
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

/// Lexically normalize `path` by dropping `.` components and popping a
/// directory for each `..`.
///
/// Purely lexical on purpose: the target usually does not exist yet, so
/// `canonicalize` is unavailable. [`resolve_artifact_path`] rejects `..`
/// outright before calling this; the popping here is defence in depth for any
/// future caller.
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Render `absolute` as an `action_dir`-relative, `/`-separated path so the
/// string can be pasted straight into a `file_read` call on any platform.
///
/// Falls back to the lossy display form when `absolute` is somehow not under
/// `action_dir` (unreachable via [`resolve_artifact_path`], which validates
/// containment first).
pub fn relative_to_action_dir(action_dir: &Path, absolute: &Path) -> String {
    match absolute.strip_prefix(action_dir) {
        Ok(rel) => rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => absolute.to_string_lossy().to_string(),
    }
}

/// Resolve `relative` into an absolute offload target under
/// `action_dir/<kind.subdir()>`, or explain why it is refused.
///
/// When `policy` is supplied the resolved path is additionally checked against
/// the core's internal state root:
///
/// * anything under `policy.workspace_dir` is refused outright, because the
///   convention is that offload targets live under `action_dir` and never
///   `workspace_dir`;
/// * anything [`SecurityPolicy::is_workspace_internal_path`] flags is refused
///   with the more specific error, which keeps the message useful when
///   `action_dir` has been configured inside the workspace root.
///
/// Passing `policy: None` skips only the workspace checks; the traversal and
/// containment checks always run.
pub fn resolve_artifact_path(
    action_dir: &Path,
    policy: Option<&SecurityPolicy>,
    kind: ArtifactKind,
    relative: &str,
) -> Result<PathBuf, OffloadError> {
    let trimmed = relative.trim();
    if trimmed.is_empty() {
        return Err(OffloadError::EmptyName);
    }

    let requested = Path::new(trimmed);
    let root = action_dir.join(kind.subdir());
    for component in requested.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(OffloadError::PathEscape {
                    root: root.display().to_string(),
                    path: trimmed.to_string(),
                });
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(OffloadError::AbsolutePath {
                    path: trimmed.to_string(),
                });
            }
        }
    }

    let candidate = normalize_lexically(&root.join(requested));
    // `normalize_lexically` cannot climb above `root` given the `..` rejection
    // above, but assert containment anyway so a future relaxation of that
    // rejection cannot silently widen the write surface.
    if !candidate.starts_with(&root) {
        return Err(OffloadError::PathEscape {
            root: root.display().to_string(),
            path: candidate.display().to_string(),
        });
    }

    if let Some(policy) = policy {
        if policy.is_workspace_internal_path(&candidate) {
            return Err(OffloadError::WorkspaceInternal {
                path: candidate.display().to_string(),
            });
        }
        if candidate.starts_with(&policy.workspace_dir) {
            return Err(OffloadError::WorkspaceTarget {
                path: candidate.display().to_string(),
            });
        }
    }

    Ok(candidate)
}
