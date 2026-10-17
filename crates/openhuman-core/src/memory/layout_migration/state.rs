//! The migration's state, the `memory`/`state`/`layout_migration` storage
//! value (once `<workspace>/memory/layout_migration.json`).
//!
//! Written whole after every page, so a crash leaves the previous state or
//! the new one, never a torn value.
//! The state itself is `tinymemory_tools::layout_migration::MigrationState`.

use std::path::{Path, PathBuf};

use tinymemory_tools::layout_migration::{Error, Result};

use super::{MigrationState, MigrationStateStore};
use crate::memory::error::{MemoryError, MemoryResult};

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
pub struct FileStateStore {
    workspace_dir: PathBuf,
}

impl FileStateStore {
    /// The store of `workspace_dir`.
    #[must_use]
    pub fn new(workspace_dir: &Path) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
        }
    }
}

impl MigrationStateStore for FileStateStore {
    fn load(&self) -> Result<MigrationState> {
        load(&self.workspace_dir).map_err(|error| Error::State(detail(error)))
    }

    fn save(&self, state: &MigrationState) -> Result<()> {
        save(&self.workspace_dir, state).map_err(|error| Error::State(detail(error)))
    }
}

fn detail(error: MemoryError) -> String {
    match error {
        MemoryError::Engine(message) => message,
        other => other.to_string(),
    }
}

/// The saved state, or a fresh one when none was saved.
///
/// # Errors
///
/// A state that cannot be read or parsed: the migration must not start
/// over and copy everything again on a damaged value.
pub fn load(workspace_dir: &Path) -> MemoryResult<MigrationState> {
    crate::memory::state_store::load_strict(workspace_dir, STATE)
        .map(Option::unwrap_or_default)
        .map_err(|error| {
            MemoryError::Engine(format!("layout migration state unreadable: {error:#}"))
        })
}

/// Saves `state` whole (one storage write, so a crash leaves the previous
/// state or the new one).
///
/// # Errors
///
/// When the state cannot be written.
pub fn save(workspace_dir: &Path, state: &MigrationState) -> MemoryResult<()> {
    crate::memory::state_store::save(workspace_dir, STATE, state).map_err(|error| {
        MemoryError::Engine(format!("layout migration state not saved: {error:#}"))
    })
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
