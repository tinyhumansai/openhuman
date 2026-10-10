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
    POLICY_BLOCKED_MARKER, WORKSPACE_INTERNAL_DIRS, WORKSPACE_INTERNAL_FILES,
    WORKSPACE_INTERNAL_PREFIXES,
};

impl SecurityPolicy {
    /// Capture host-owned settings and the task-local turn grant before any await.
    /// Registration deduplicates the complete immutable value, so changed roots
    /// or policy settings cannot reuse a stale scope from a cloned policy.
    pub(crate) fn native_path_policy(&self) -> PathPolicy {
        let mut trusted_roots: Vec<_> = self
            .trusted_roots
            .iter()
            .map(|root| PathTrustedRoot {
                path: PathBuf::from(self.expand_tilde(&root.path)),
                access: match root.access {
                    TrustedAccess::Read => PathAccess::ReadOnly,
                    TrustedAccess::ReadWrite => PathAccess::ReadWrite,
                },
            })
            .collect();
        if let Some(path) = crate::agent::turn_workspace::current() {
            trusted_roots.push(PathTrustedRoot {
                path,
                access: PathAccess::ReadWrite,
            });
        }
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
        let tool_results = self
            .workspace_dir
            .join(ARTIFACTS_DIR)
            .join(ARTIFACT_TOOL_RESULTS_DIR);
        reserved_paths.push(PathReservation {
            path: self.workspace_dir.join(ARTIFACTS_DIR),
            exceptions: vec![tool_results.clone()],
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
        let scope = self.native_path_policy();
        let result = crate::modules::security::validate_host_path(scope, path, parent).await;
        project_native_result(path, result)
    }
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
        Err(_) => Err(format!(
            "{POLICY_BLOCKED_MARKER} TinySecurity path authorization unavailable; file access denied"
        )),
    }
}

#[cfg(test)]
#[path = "native_paths_tests.rs"]
mod tests;
