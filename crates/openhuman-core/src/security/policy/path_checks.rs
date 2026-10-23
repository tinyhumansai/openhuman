use std::path::{Path, PathBuf};

#[cfg(not(feature = "security-module"))]
use tinytools::status::POLICY_BLOCKED_MARKER;

use super::types::SecurityPolicy;

/// Credential-store path segments, matched case-insensitively at any depth.
pub(crate) const SENSITIVE_COMPONENTS: &[&str] =
    &[".ssh", ".gnupg", ".aws", ".azure", ".kube", "keychains"];
/// Single-file credential stores have no protected parent directory.
pub(crate) const SENSITIVE_FILES: &[&str] = &[".netrc", ".git-credentials", ".pgpass", ".npmrc"];
/// Consecutive segments used by Docker, Cargo, and Windows DPAPI stores.
pub(crate) const SENSITIVE_PAIRS: &[[&str; 2]] = &[
    [".docker", "config.json"],
    [".cargo", "credentials.toml"],
    ["microsoft", "protect"],
    ["microsoft", "credentials"],
    ["microsoft", "crypto"],
    ["microsoft", "vault"],
];

/// Credential stores under a user's home, for jail backends that deny paths.
#[cfg(any(feature = "modules", test))]
pub(crate) fn home_credential_stores(home: &Path) -> Vec<PathBuf> {
    let components = SENSITIVE_COMPONENTS.iter().map(|segment| match *segment {
        "keychains" => home.join("Library").join("Keychains"),
        segment => home.join(segment),
    });
    let files = SENSITIVE_FILES.iter().map(|file| home.join(file));
    let pairs = SENSITIVE_PAIRS
        .iter()
        .filter(|[dir, _]| dir.starts_with('.'))
        .map(|[dir, file]| home.join(dir).join(file));
    components.chain(files).chain(pairs).collect()
}

impl SecurityPolicy {
    /// Validate a path for file I/O: string checks, canonicalize, workspace containment,
    /// and forbidden-path check on the resolved path.
    /// Returns the canonical `PathBuf` on success.
    #[cfg(feature = "security-module")]
    pub async fn validate_path(&self, path: &str) -> Result<PathBuf, String> {
        self.validate_native_path(path, false).await
    }

    /// Without the module there is nothing to ask: file access is denied.
    #[cfg(not(feature = "security-module"))]
    pub async fn validate_path(&self, path: &str) -> Result<PathBuf, String> {
        log::debug!(
            "[security][path] validate_path denied: module feature disabled path_len={}",
            path.len()
        );
        Err(format!(
            "{POLICY_BLOCKED_MARKER} TinySecurity path authorization unavailable; file access denied"
        ))
    }

    /// Like `validate_path` but canonicalizes the parent directory.
    /// Use for write operations where the target file may not yet exist.
    /// Does NOT require the parent directory to exist — walks up to the deepest
    /// existing ancestor and checks that for symlink escapes.
    /// Returns the canonical full path (parent resolved + filename appended).
    #[cfg(feature = "security-module")]
    pub async fn validate_parent_path(&self, path: &str) -> Result<PathBuf, String> {
        self.validate_native_path(path, true).await
    }

    /// Without the module there is nothing to ask: file access is denied.
    #[cfg(not(feature = "security-module"))]
    pub async fn validate_parent_path(&self, path: &str) -> Result<PathBuf, String> {
        log::debug!(
            "[security][path] validate_parent_path denied: module feature disabled path_len={}",
            path.len()
        );
        Err(format!(
            "{POLICY_BLOCKED_MARKER} TinySecurity path authorization unavailable; file access denied"
        ))
    }
}

#[cfg(all(test, feature = "security-module"))]
#[path = "path_checks_native_tests.rs"]
mod native_tests;
