//! The host's path and budget questions, answered by TinySecurity.
//!
//! Every method here asks the module (`CheckPaths2`, `SpendAction2`,
//! `PeekBudget2`) and projects its stable answer into the host's vocabulary. None
//! decides anything: there is no local path check and no local counter to fall back
//! on, so a module that is missing, untrusted, faulted or slow refuses the call.
//!
//! The synchronous variants exist for the tool traits (`FsGate`, `NetGate`) that
//! cannot be `async`; they go through the shared sync-to-async seam
//! (`modules::security::sync_seam`).
//!
//! Unit tests of what a caller does with an answer install a scripted port
//! (`modules::security::gap::stub`); production always asks the module.

use std::path::{Path, PathBuf};

use super::types::SecurityPolicy;

/// Which filesystem question is asked of the module, mirroring its `PathCheckKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKind {
    /// Lexical and existing-ancestor checks: may this path be used at all.
    StringAllowed,
    /// The resolved path is readable under the policy.
    ResolvedRead,
    /// The resolved path is writable under the policy.
    ResolvedWrite,
    /// The path is host internal state the agent never reaches.
    Internal,
}

/// The command class of a host action, from its permission level and whether it
/// has an external effect. Informational (the gate decides what may run), and the
/// module's: the host keeps no mapping of its own. `Err` when the module cannot
/// answer.
pub async fn host_action_class(
    permission: tinytools::PermissionLevel,
    external_effect: bool,
) -> Result<tinysecurity_bus::CommandClass, String> {
    native::host_action_class(permission, external_effect).await
}

impl SecurityPolicy {
    /// One answer per path for `kind`, in order, from a single module call per
    /// batch. `Err` means the module could not answer; callers treat that as a
    /// refusal of every path.
    pub async fn check_paths(
        &self,
        kind: PathKind,
        paths: &[PathBuf],
    ) -> Result<Vec<bool>, String> {
        self.native_check_paths(kind, paths).await
    }

    /// Whether `path` passes `kind`. A path the module could not be asked about is
    /// refused.
    pub async fn path_allowed(&self, kind: PathKind, path: &Path) -> bool {
        match self.check_paths(kind, &[path.to_path_buf()]).await {
            Ok(answers) => answers.first().copied().unwrap_or(false),
            Err(error) => {
                log::debug!("[security][path] {kind:?} unavailable; refused: {error}");
                false
            }
        }
    }

    /// [`Self::check_paths`] for a synchronous caller.
    pub fn check_paths_blocking(&self, kind: PathKind, paths: &[PathBuf]) -> Vec<bool> {
        match self.native_check_paths_blocking(kind, paths) {
            Ok(answers) if answers.len() == paths.len() => answers,
            Ok(_) => {
                log::warn!("[security][path] answer length mismatch; every path refused");
                vec![false; paths.len()]
            }
            Err(error) => {
                log::debug!("[security][path] {kind:?} unavailable; refused: {error}");
                vec![false; paths.len()]
            }
        }
    }

    /// [`Self::path_allowed`] for a synchronous caller.
    pub fn path_allowed_blocking(&self, kind: PathKind, path: &Path) -> bool {
        self.check_paths_blocking(kind, &[path.to_path_buf()])
            .first()
            .copied()
            .unwrap_or(false)
    }

    /// Whether `path` is host internal state the agent never reaches (the module's
    /// `Internal` reservation). Fails closed: a path the module could not be asked
    /// about is internal.
    pub async fn is_internal(&self, path: &Path) -> bool {
        match self
            .check_paths(PathKind::Internal, &[path.to_path_buf()])
            .await
        {
            Ok(answers) => answers.first().copied().unwrap_or(true),
            Err(error) => {
                log::debug!("[security][path] internal-state check unavailable; treated as internal: {error}");
                true
            }
        }
    }

    /// [`Self::is_internal`] for a synchronous caller.
    pub fn is_internal_blocking(&self, path: &Path) -> bool {
        match self.native_check_paths_blocking(PathKind::Internal, &[path.to_path_buf()]) {
            Ok(answers) if answers.len() == 1 => answers[0],
            Ok(_) | Err(_) => {
                log::debug!(
                    "[security][path] internal-state check unavailable; treated as internal"
                );
                true
            }
        }
    }

    /// Whether the autonomy tier lets the agent change anything right now. A module
    /// that cannot answer says no.
    pub fn tier_allows_acting_blocking(&self) -> bool {
        if !self.enabled {
            return true;
        }
        matches!(
            self.budget_snapshot
                .lock()
                .ok()
                .and_then(|snapshot| snapshot.clone()),
            Some(
                tinysecurity_bus::BudgetDecision2::Allow
                    | tinysecurity_bus::BudgetDecision2::RateLimited { .. }
            )
        )
    }
}

#[cfg(feature = "security-module")]
mod native {
    use super::*;
    use crate::modules::security::gap;
    use tinysecurity_bus::PathCheckKind;

    fn bus_kind(kind: PathKind) -> PathCheckKind {
        match kind {
            PathKind::StringAllowed => PathCheckKind::StringAllowed,
            PathKind::ResolvedRead => PathCheckKind::ResolvedRead,
            PathKind::ResolvedWrite => PathCheckKind::ResolvedWrite,
            PathKind::Internal => PathCheckKind::Internal,
        }
    }

    pub(super) async fn host_action_class(
        permission: tinytools::PermissionLevel,
        external_effect: bool,
    ) -> Result<tinysecurity_bus::CommandClass, String> {
        use tinysecurity_bus::{HostActionClassRequest2, ToolPermission};
        use tinytools::PermissionLevel as Level;
        let permission = match permission {
            Level::None => ToolPermission::None,
            Level::ReadOnly => ToolPermission::ReadOnly,
            Level::Write => ToolPermission::Write,
            Level::Execute => ToolPermission::Execute,
            Level::Dangerous => ToolPermission::Dangerous,
        };
        gap::host_action_class(HostActionClassRequest2 {
            permission,
            external_effect,
        })
        .await
    }

    impl SecurityPolicy {
        pub(super) async fn native_check_paths(
            &self,
            kind: PathKind,
            paths: &[PathBuf],
        ) -> Result<Vec<bool>, String> {
            gap::check_paths(self.native_path_policy(), bus_kind(kind), paths).await
        }

        pub(super) fn native_check_paths_blocking(
            &self,
            kind: PathKind,
            paths: &[PathBuf],
        ) -> Result<Vec<bool>, String> {
            let scope = self.native_path_policy();
            let paths = paths.to_vec();
            gap::blocking(async move { gap::check_paths(scope, bus_kind(kind), &paths).await })
        }
    }
}

#[cfg(not(feature = "security-module"))]
mod native {
    use super::*;

    const NO_MODULE: &str = "TinySecurity is not built into this host";

    pub(super) async fn host_action_class(
        _permission: tinytools::PermissionLevel,
        _external_effect: bool,
    ) -> Result<tinysecurity_bus::CommandClass, String> {
        Err(NO_MODULE.into())
    }

    impl SecurityPolicy {
        pub(super) async fn native_check_paths(
            &self,
            _kind: PathKind,
            _paths: &[PathBuf],
        ) -> Result<Vec<bool>, String> {
            Err(NO_MODULE.into())
        }

        pub(super) fn native_check_paths_blocking(
            &self,
            _kind: PathKind,
            _paths: &[PathBuf],
        ) -> Result<Vec<bool>, String> {
            Err(NO_MODULE.into())
        }
    }
}
