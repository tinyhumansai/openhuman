//! The unconditional protected-path floor, answered by TinySecurity.
//!
//! Credential stores (`~/.ssh`, `~/.gnupg`, `~/.aws`, ...), system roots, `..`
//! traversal and NUL bytes are unreachable whether or not the discretionary
//! autonomy policy is enabled. The module owns the list and the match; this file
//! only asks. A path the module could not be asked about is **protected**: every
//! failure (no module, faulted, slow, malformed answer) refuses.
//!
//! Callers here hold no [`SecurityPolicy`](super::SecurityPolicy) (a working-folder
//! picker, an artifact escape guard, a jail-grant filter), so the question names
//! only paths.

use std::path::Path;

/// The floor's vocabulary, for components that apply it themselves (a jail grant
/// resolver) rather than asking per path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Floor {
    /// Credential-store directory names, matched at any depth.
    pub components: Vec<String>,
    /// Single-file credential stores, matched at any depth.
    pub files: Vec<String>,
    /// Consecutive segment pairs that mark a credential store.
    pub pairs: Vec<[String; 2]>,
    /// Lower-cased absolute system prefixes.
    pub prefixes: Vec<String>,
}

/// Whether `path` is inside the floor. Fails closed.
pub async fn is_protected(path: &Path) -> bool {
    native::is_protected(path).await
}

/// Whether any of `paths` is inside the floor, in one module call. Fails closed.
pub async fn any_protected(paths: &[&Path]) -> bool {
    native::any_protected(paths).await
}

/// One answer per path, in order, from a single module call. `Err` when the module
/// cannot answer; callers treat that as every path protected.
pub async fn protected_paths(paths: &[std::path::PathBuf]) -> Result<Vec<bool>, String> {
    native::protected_paths(paths).await
}

/// [`is_protected`] for a synchronous caller. Fails closed.
pub fn is_protected_blocking(path: &Path) -> bool {
    native::is_protected_blocking(path)
}

/// The floor's vocabulary. `Err` when the module cannot answer.
pub async fn floor() -> Result<Floor, String> {
    native::floor().await
}

#[cfg(feature = "security-module")]
mod native {
    use super::*;
    use crate::modules::security::gap;

    pub(super) async fn is_protected(path: &Path) -> bool {
        gap::is_protected(path).await
    }

    pub(super) async fn any_protected(paths: &[&Path]) -> bool {
        gap::any_protected(paths).await
    }

    pub(super) async fn protected_paths(paths: &[std::path::PathBuf]) -> Result<Vec<bool>, String> {
        gap::protected_paths(paths).await
    }

    pub(super) fn is_protected_blocking(path: &Path) -> bool {
        gap::is_protected_blocking(path)
    }

    pub(super) async fn floor() -> Result<Floor, String> {
        let floor = gap::protected_floor().await?;
        Ok(Floor {
            components: floor.components,
            files: floor.files,
            pairs: floor.pairs,
            prefixes: floor.prefixes,
        })
    }
}

#[cfg(not(feature = "security-module"))]
mod native {
    use super::*;

    pub(super) async fn is_protected(_path: &Path) -> bool {
        true
    }

    pub(super) async fn any_protected(_paths: &[&Path]) -> bool {
        true
    }

    pub(super) async fn protected_paths(
        _paths: &[std::path::PathBuf],
    ) -> Result<Vec<bool>, String> {
        Err("TinySecurity is not built into this host".into())
    }

    pub(super) fn is_protected_blocking(_path: &Path) -> bool {
        true
    }

    pub(super) async fn floor() -> Result<Floor, String> {
        Err("TinySecurity is not built into this host".into())
    }
}
