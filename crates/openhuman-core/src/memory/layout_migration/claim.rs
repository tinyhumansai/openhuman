//! Which account on this machine takes a shared legacy tree.
//!
//! With a self-hosted CortexDB every account on the machine reaches the same
//! `app:tinymemory/…` tree, so only one of them may move it into its own
//! per-user tree. The first to start a consented move claims it with a marker
//! in the app directory shared by all accounts,
//! `<app>/memory/legacy_claims/<digest of the endpoint>.json`; other accounts
//! then have nothing to move. The marker is created with a hard link, which
//! fails when it exists, so two accounts racing cannot both claim the tree.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::Digest;

use tinymemory_tools::layout_migration::{LegacyClaim, Result as ModuleResult};

use super::app_host::to_module;
use crate::memory::error::{MemoryError, MemoryResult};

/// Where a legacy tree's marker lives and who would own it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimKey {
    /// The marker file.
    pub marker: PathBuf,
    /// The claiming account's actor (`user:<id>`).
    pub owner: String,
    /// The engine endpoint the tree lives on, normalised.
    pub endpoint: String,
}

impl ClaimKey {
    /// The key for the tree at `endpoint`, below the shared app directory.
    #[must_use]
    pub fn new(app_dir: &Path, endpoint: &str, owner: &str) -> Self {
        let endpoint = normalize(endpoint);
        let digest: String = sha2::Sha256::digest(endpoint.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Self {
            marker: app_dir
                .join("memory")
                .join("legacy_claims")
                .join(format!("{digest}.json")),
            owner: owner.to_string(),
            endpoint,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Marker {
    endpoint: String,
    owner: String,
}

/// One spelling per endpoint: trimmed, scheme and host lowercased, default
/// port and trailing `/` dropped.
fn normalize(endpoint: &str) -> String {
    let endpoint = endpoint.trim();
    match url::Url::parse(endpoint) {
        Ok(url) => url.as_str().trim_end_matches('/').to_string(),
        Err(_) => endpoint.trim_end_matches('/').to_ascii_lowercase(),
    }
}

/// The account that claimed the tree, if any.
///
/// # Errors
///
/// When the marker exists but cannot be read.
fn owner(key: &ClaimKey) -> MemoryResult<Option<String>> {
    match std::fs::read(&key.marker) {
        Ok(bytes) => serde_json::from_slice::<Marker>(&bytes)
            .map(|marker| Some(marker.owner))
            .map_err(|error| unreadable(&error)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(unreadable(&error)),
    }
}

/// Whether another account claimed the tree.
///
/// # Errors
///
/// When the marker exists but cannot be read.
pub fn held_by_other(key: &ClaimKey) -> MemoryResult<bool> {
    Ok(owner(key)?.is_some_and(|owner| owner != key.owner))
}

/// Claims the tree for `key.owner`: `true` when it is now (or already was)
/// theirs, `false` when another account holds it.
///
/// # Errors
///
/// When the marker cannot be read or written.
pub fn take(key: &ClaimKey) -> MemoryResult<bool> {
    if let Some(owner) = owner(key)? {
        return Ok(owner == key.owner);
    }
    let write = || -> std::io::Result<()> {
        if let Some(dir) = key.marker.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temp = tempfile::NamedTempFile::new_in(key.marker.parent().unwrap_or(Path::new(".")))?;
        let marker = Marker {
            endpoint: key.endpoint.clone(),
            owner: key.owner.clone(),
        };
        serde_json::to_writer(temp.as_file(), &marker).map_err(std::io::Error::other)?;
        temp.as_file().sync_all()?;
        // Fails when the marker exists: never replaces another's claim.
        std::fs::hard_link(temp.path(), &key.marker)
    };
    match write() {
        Ok(()) => {
            tracing::info!("[memory:layout_migration] legacy tree claimed");
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Ok(owner(key)?.is_some_and(|owner| owner == key.owner))
        }
        Err(error) => Err(MemoryError::Engine(format!(
            "legacy tree claim not saved: {error}"
        ))),
    }
}

impl LegacyClaim for ClaimKey {
    fn held_by_other(&self) -> ModuleResult<bool> {
        held_by_other(self).map_err(to_module)
    }

    fn take(&self) -> ModuleResult<bool> {
        take(self).map_err(to_module)
    }
}

fn unreadable(error: &dyn std::fmt::Display) -> MemoryError {
    MemoryError::Engine(format!("legacy tree claim unreadable: {error}"))
}

#[cfg(test)]
#[path = "claim_tests.rs"]
mod tests;
