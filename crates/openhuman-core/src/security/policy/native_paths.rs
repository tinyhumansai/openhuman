//! Product configuration translation for native TinySecurity filesystem scopes.
//!
//! Only async I/O validators migrate here. Synchronous lexical helpers remain
//! host preflights; they cannot grant I/O permission in place of native validation.

use std::path::PathBuf;

use tinysecurity_bus::{
    PathAccess, PathErrorCategory, PathPolicy, PathReservation, PathReservedPrefix,
    PathTrustedRoot, PathValidationResult,
};

use super::types::{
    SecurityPolicy, TrustedAccess, ACCOUNT_CONFIG_FILE, ARTIFACTS_DIR, ARTIFACT_TOOL_RESULTS_DIR,
    WORKSPACE_INTERNAL_DIRS, WORKSPACE_INTERNAL_FILES, WORKSPACE_INTERNAL_PREFIXES,
};
use tinytools::status::POLICY_BLOCKED_MARKER;

impl SecurityPolicy {
    /// Capture host-owned settings and the task-local turn grant before any await.
    /// Registration deduplicates the complete immutable value, so changed roots
    /// or policy settings cannot reuse a stale scope from a cloned policy.
    pub(crate) fn native_path_policy(&self) -> PathPolicy {
        let mut trusted_roots: Vec<_> = self
            .trusted_roots
            .iter()
            .map(|root| PathTrustedRoot {
                // Policy and config expand `~/` byte-for-byte identically (#3353).
                path: PathBuf::from(crate::config::expand_tilde(&root.path)),
                access: match root.access {
                    TrustedAccess::Read => PathAccess::ReadOnly,
                    TrustedAccess::ReadWrite => PathAccess::ReadWrite,
                },
            })
            .collect();
        let media_paths = self
            .media_artifact_paths
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let artifacts_dir = self.workspace_dir.join(ARTIFACTS_DIR);
        // TinySecurity admits a reservation exception only when it resolves to
        // a strict descendant of the reservation, and refuses the whole policy
        // otherwise. A ready media artifact filed in the Files folder lives
        // outside `<workspace>/artifacts`, so it is granted as a read-only
        // trusted root of that exact file instead.
        let (media_exceptions, media_outside): (Vec<_>, Vec<_>) = media_paths
            .into_iter()
            .partition(|path| is_strict_descendant(path, &artifacts_dir));
        trusted_roots.extend(media_outside.into_iter().map(|path| PathTrustedRoot {
            path,
            access: PathAccess::ReadOnly,
        }));
        let mut reserved_paths: Vec<_> = WORKSPACE_INTERNAL_DIRS
            .iter()
            .chain(WORKSPACE_INTERNAL_FILES.iter())
            .map(|name| PathReservation {
                path: self.workspace_dir.join(name),
                exceptions: Vec::new(),
                children_only: false,
            })
            .collect();
        if let Some(account_dir) = self.workspace_dir.parent() {
            reserved_paths.push(PathReservation {
                path: account_dir.join(ACCOUNT_CONFIG_FILE),
                exceptions: Vec::new(),
                children_only: false,
            });
        }
        let tool_results = artifacts_dir.join(ARTIFACT_TOOL_RESULTS_DIR);
        reserved_paths.push(PathReservation {
            path: artifacts_dir,
            exceptions: std::iter::once(tool_results.clone())
                .chain(media_exceptions)
                .collect(),
            children_only: true,
        });
        PathPolicy {
            enabled: self.enabled,
            workspace_only: self.workspace_only,
            workspace_dir: self.workspace_dir.clone(),
            action_dir: self.action_dir.clone(),
            home_dir: dirs::home_dir(),
            forbidden_paths: self.forbidden_paths.clone(),
            trusted_roots,
            reserved_paths,
            reserved_prefixes: WORKSPACE_INTERNAL_PREFIXES
                .iter()
                .map(|prefix| PathReservedPrefix {
                    root: self.workspace_dir.clone(),
                    prefix: (*prefix).to_owned(),
                })
                .collect(),
            readonly_paths: vec![tool_results],
            reserved_names: WORKSPACE_INTERNAL_FILES
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
        }
    }

    pub(super) async fn validate_native_path(
        &self,
        path: &str,
        parent: bool,
    ) -> Result<PathBuf, String> {
        if crate::security::switch::disabled().is_some() {
            return self.resolve_unchecked(path, parent).await;
        }
        let scope = self.native_path_policy();
        #[cfg(feature = "security-module")]
        let result = crate::modules::security::validate_host_path(scope, path, parent).await;
        #[cfg(not(feature = "security-module"))]
        let result = {
            let _ = (scope, parent);
            Err("TinySecurity is not built into this host".to_owned())
        };
        project_native_result(path, result)
    }
}

impl SecurityPolicy {
    /// Resolve `path` without asking TinySecurity, for security disabled
    /// (`crate::security::switch`). Resolution follows the module's own shape so
    /// callers see the same paths: `~/` expanded, relative paths joined onto
    /// `action_dir`, a read resolved through symlinks (it must exist), a write
    /// target resolved through its nearest existing ancestor. No root, floor or
    /// reservation is applied.
    async fn resolve_unchecked(&self, path: &str, parent: bool) -> Result<PathBuf, String> {
        let expanded = PathBuf::from(self.expand_tilde(path));
        let absolute = if expanded.is_absolute() {
            expanded
        } else {
            self.action_dir.join(expanded)
        };
        let unresolved = || format!("Failed to resolve path '{path}'");
        if !parent {
            return tokio::fs::canonicalize(&absolute)
                .await
                .map_err(|_| unresolved());
        }
        let Some(filename) = absolute.file_name().map(ToOwned::to_owned) else {
            return Err(format!("Invalid path: {path}"));
        };
        if tokio::fs::symlink_metadata(&absolute).await.is_ok() {
            return tokio::fs::canonicalize(&absolute)
                .await
                .map_err(|_| unresolved());
        }
        let mut missing = vec![filename];
        let mut ancestor = absolute.parent().map(std::path::Path::to_path_buf);
        while let Some(dir) = ancestor {
            if let Ok(resolved) = tokio::fs::canonicalize(&dir).await {
                return Ok(missing
                    .iter()
                    .rev()
                    .fold(resolved, |at, part| at.join(part)));
            }
            match dir.file_name() {
                Some(name) => missing.push(name.to_owned()),
                None => break,
            }
            ancestor = dir.parent().map(std::path::Path::to_path_buf);
        }
        Err(unresolved())
    }
}

/// Whether `path` sits strictly inside `root`, compared both lexically and
/// through the root's canonical form (media paths are recorded canonical, a
/// configured workspace may not be, e.g. macOS `/var` -> `/private/var`).
fn is_strict_descendant(path: &std::path::Path, root: &std::path::Path) -> bool {
    let inside = |root: &std::path::Path| path != root && path.starts_with(root);
    inside(root) || std::fs::canonicalize(root).is_ok_and(|canonical| inside(&canonical))
}

/// Project stable native categories into the existing host tool error contract.
fn project_native_result(
    path: &str,
    result: Result<PathValidationResult, String>,
) -> Result<PathBuf, String> {
    match result {
        Ok(PathValidationResult::Allowed { resolved }) if resolved.is_absolute() => Ok(resolved),
        Ok(PathValidationResult::Allowed { .. }) => Err(format!(
            "{POLICY_BLOCKED_MARKER} TinySecurity returned an invalid resolved path"
        )),
        Ok(PathValidationResult::Denied { category }) => match category {
            PathErrorCategory::Protected | PathErrorCategory::PolicyDenied => Err(format!(
                "{POLICY_BLOCKED_MARKER} Path not allowed by security policy: {path}. Do not \
                 retry this path; use an allowed location (the workspace or a granted folder)."
            )),
            PathErrorCategory::Resolution => Err(format!("Failed to resolve path '{path}'")),
            PathErrorCategory::InvalidPath => Err(format!("Invalid path: {path}")),
        },
        Err(error) => {
            // The cause stays out of the tool error (a model reads that) but is
            // the first thing needed when a read is refused for no visible reason.
            log::debug!("[security][path] native path validation unavailable: {error}");
            Err(format!(
                "{POLICY_BLOCKED_MARKER} TinySecurity path authorization unavailable; file access denied"
            ))
        }
    }
}
