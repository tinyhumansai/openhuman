//! The migration's `layout_migration` value in the memory state KV store.
//! The first read imports the legacy `<workspace>/memory/layout_migration.json`.
//!
//! Written whole after every page, so a crash leaves the previous state or
//! the new one, never a torn value.
//! The state itself is `tinymemory_tools::layout_migration::MigrationState`.

use std::path::{Path, PathBuf};

use tinymemory_tools::layout_migration::{Error as MigrationError, Result as MigrationResult};

use super::{MigrationState, MigrationStateStore};
use tinymemory_api::{Error, Result};

const STATE: &str = "layout_migration";

/// Whether a migration state exists in `workspace_dir`: the migration has
/// begun there (a state that cannot be read counts: it was begun).
#[must_use]
pub fn exists(workspace_dir: &Path) -> bool {
    crate::memory::state_store::load_strict::<serde_json::Value>(workspace_dir, STATE)
        .map(|state| state.is_some())
        .unwrap_or(true)
}

/// The migration state kept in one workspace.
#[derive(Debug, Clone)]
pub struct StateKvMigrationStore {
    workspace_dir: PathBuf,
}

/// Compatibility name for callers that used the former file adapter.
pub type FileStateStore = StateKvMigrationStore;

impl StateKvMigrationStore {
    /// The store of `workspace_dir`.
    #[must_use]
    pub fn new(workspace_dir: &Path) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
        }
    }
}

impl MigrationStateStore for StateKvMigrationStore {
    fn load(&self) -> MigrationResult<MigrationState> {
        load(&self.workspace_dir).map_err(|error| MigrationError::State(detail(error)))
    }

    fn save(&self, state: &MigrationState) -> MigrationResult<()> {
        save(&self.workspace_dir, state).map_err(|error| MigrationError::State(detail(error)))
    }
}

fn detail(error: Error) -> String {
    match error {
        Error::Engine(message) => message,
        other => other.to_string(),
    }
}

/// The saved state, or a fresh one when none was saved.
///
/// # Errors
///
/// A state file that cannot be read or parsed: the migration must not start
/// over and copy everything again on a damaged file.
pub fn load(workspace_dir: &Path) -> Result<MigrationState> {
    crate::memory::state_store::load_strict(workspace_dir, STATE)
        .map(|state| state.unwrap_or_default())
        .map_err(|error| Error::Engine(format!("layout migration state unreadable: {error}")))
}

/// Saves `state` whole (one storage write, so a crash leaves the previous
/// state or the new one).
///
/// # Errors
///
/// When the file cannot be written.
pub fn save(workspace_dir: &Path, state: &MigrationState) -> Result<()> {
    crate::memory::state_store::save(workspace_dir, STATE, state)
        .map_err(|error| Error::Engine(format!("layout migration state not saved: {error}")))
}

/// Legacy file path used by one-shot state-store import and compatibility
/// tests.
#[must_use]
pub fn path(workspace_dir: &Path) -> PathBuf {
    crate::memory::state_store::legacy_path(workspace_dir, STATE)
}

// These read and write the migration state, and every state read adopts the
// legacy file under an import claim that needs conditional writes. The
// in-memory test backend has none; only `storage-sqlite` gives tests a
// backend that does (`StateKv::open`), so without it they cannot run.
#[cfg(all(test, feature = "storage-sqlite"))]
#[path = "state_tests.rs"]
mod tests;
