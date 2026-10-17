//! The migration's state on disk, `<workspace>/memory/layout_migration.json`.
//!
//! Written whole after every page, through a temporary file and a rename,
//! so a crash leaves the previous state or the new one, never a torn file.
//! The state itself is `tinymemory_tools::layout_migration::MigrationState`.

use std::path::{Path, PathBuf};

use tinymemory_tools::layout_migration::{Error, Result};

use super::{MigrationState, MigrationStateStore};
use crate::memory::error::{MemoryError, MemoryResult};

/// The state file of `workspace_dir`.
#[must_use]
pub fn path(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join("memory").join("layout_migration.json")
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
/// A state file that cannot be read or parsed: the migration must not start
/// over and copy everything again on a damaged file.
pub fn load(workspace_dir: &Path) -> MemoryResult<MigrationState> {
    let file = path(workspace_dir);
    match std::fs::read(&file) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
            MemoryError::Engine(format!("layout migration state unreadable: {error}"))
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(MigrationState::default()),
        Err(error) => Err(MemoryError::Engine(format!(
            "layout migration state unreadable: {error}"
        ))),
    }
}

/// Saves `state` whole: a temporary file, synced, then renamed over the old.
///
/// # Errors
///
/// When the file cannot be written.
pub fn save(workspace_dir: &Path, state: &MigrationState) -> MemoryResult<()> {
    let file = path(workspace_dir);
    let write = || -> std::io::Result<()> {
        if let Some(dir) = file.parent() {
            crate::memory::files::create_private_dir_all(dir)?;
        }
        let temp = file.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(state).map_err(std::io::Error::other)?;
        let mut out = crate::memory::files::create_private(&temp)?;
        std::io::Write::write_all(&mut out, &bytes)?;
        out.sync_all()?;
        std::fs::rename(&temp, &file)
    };
    write()
        .map_err(|error| MemoryError::Engine(format!("layout migration state not saved: {error}")))
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
