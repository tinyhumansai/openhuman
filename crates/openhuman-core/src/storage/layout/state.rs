//! The migration's lock and its progress record.
//!
//! - **Lock**: an OS advisory lock on `users/<id>/.layout.lock`
//!   ([`Lock`]). A second migrator for the same profile, in this or another
//!   process, is refused with [`LayoutError::Locked`]; the lock dies with its
//!   holder, so a crash never leaves a stale one behind.
//! - **Progress**: a small JSON file, `users/<id>/.layout.state`
//!   ([`LayoutState`]), written atomically (temp file, fsync, rename) after
//!   every finished step. Its existence means "a migration started and did not
//!   finish": the next run resumes from it even when the `.layout` marker is
//!   already written. It is removed as the very last act of a migration. A step
//!   is recorded only after its work finished and every step is idempotent, so
//!   a crash anywhere re-runs at most one step from its start.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use super::plan::Plan;

/// The lock file, beside `config.toml`.
pub const LOCK_FILE: &str = ".layout.lock";
/// The progress file, beside `config.toml`.
pub const STATE_FILE: &str = ".layout.state";

/// A migration step, in the order they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// Legacy importers write the v1 files into the v1 database.
    Import,
    /// The schema migrations run over the imported database; the plan is
    /// classified.
    Migrate,
    /// The database is copied to `storage.db` and verified.
    CopyDb,
    /// User files move into `files/`.
    MoveFiles,
    /// The archive is written and verified.
    Archive,
    /// The marker is written and the archived originals are deleted.
    Finish,
}

impl Step {
    /// Every step, in order.
    pub const ALL: [Step; 6] = [
        Step::Import,
        Step::Migrate,
        Step::CopyDb,
        Step::MoveFiles,
        Step::Archive,
        Step::Finish,
    ];
}

/// Why a migration did not complete.
#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    /// Another migrator holds the profile's lock.
    #[error("LAYOUT_LOCKED: another migration holds {0}")]
    Locked(PathBuf),
    /// The running core serves this profile's v1 database.
    #[error("LAYOUT_LIVE: this process serves {0}; stop the core and migrate offline, or set [storage] auto_migrate_layout and restart")]
    Live(String),
    /// A test stopped the migration after `Step`'s work, before recording it.
    #[error("LAYOUT_CRASH: injected crash after {0:?}")]
    InjectedCrash(Step),
    /// A moved entry's destination already holds something else.
    #[error("LAYOUT_CONFLICT: {0}")]
    Conflict(String),
    /// A verification failed (counts, archive).
    #[error("LAYOUT_VERIFY: {0}")]
    Verify(String),
    /// Anything else.
    #[error("{0:#}")]
    Other(#[from] anyhow::Error),
}

impl From<std::io::Error> for LayoutError {
    fn from(error: std::io::Error) -> Self {
        Self::Other(error.into())
    }
}

/// The held lock. Dropping it releases it.
#[derive(Debug)]
pub struct Lock {
    file: File,
}

impl Lock {
    /// Takes the lock of `profile_dir`, creating the lock file.
    ///
    /// # Errors
    ///
    /// [`LayoutError::Locked`] when another migrator holds it, or an I/O error.
    pub fn acquire(profile_dir: &Path) -> Result<Self, LayoutError> {
        let path = profile_dir.join(LOCK_FILE);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)?;
        match file.try_lock_exclusive() {
            Ok(()) => {
                tracing::debug!(target: "openhuman::storage", path = %path.display(), "[layout] lock taken");
                Ok(Self { file })
            }
            Err(error) if error.kind() == fs2::lock_contended_error().kind() => {
                Err(LayoutError::Locked(path))
            }
            Err(error) => Err(error.into()),
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

/// What a migration has done so far.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutState {
    /// Format version of this file.
    pub version: u32,
    /// When the migration started (names the archive).
    pub started_ms: i64,
    /// The archive's file name under `<root>/backups`.
    pub archive: String,
    /// Finished steps.
    pub done: Vec<Step>,
    /// The classified workspace, taken once the schema step finished and kept
    /// for every later step, so a resume agrees with the first run even after
    /// entries have moved.
    #[serde(default)]
    pub plan: Option<Plan>,
}

impl LayoutState {
    /// A fresh state started at `started_ms` for `user`.
    pub fn new(user: &str, started_ms: i64) -> Self {
        let stamp = chrono::DateTime::from_timestamp_millis(started_ms)
            .unwrap_or_default()
            .format("%Y%m%dT%H%M%S");
        Self {
            version: 1,
            started_ms,
            archive: format!("{}-{stamp}.tar.zst", sanitize(user)),
            done: Vec::new(),
            plan: None,
        }
    }

    /// Whether `step` finished.
    pub fn has(&self, step: Step) -> bool {
        self.done.contains(&step)
    }

    /// The state in `profile_dir`, `None` when no migration is in progress.
    ///
    /// # Errors
    ///
    /// An unreadable or unparsable file: the migration must not guess.
    pub fn load(profile_dir: &Path) -> Result<Option<Self>, LayoutError> {
        match std::fs::read(profile_dir.join(STATE_FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| {
                LayoutError::Other(anyhow::anyhow!("the layout state file is damaged: {e}"))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Writes the state atomically.
    ///
    /// # Errors
    ///
    /// An I/O error.
    pub fn save(&self, profile_dir: &Path) -> Result<(), LayoutError> {
        let tmp = profile_dir.join(format!("{STATE_FILE}.tmp"));
        let bytes = serde_json::to_vec(self).map_err(|e| LayoutError::Other(e.into()))?;
        let mut file = File::create(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, profile_dir.join(STATE_FILE))?;
        Ok(())
    }

    /// Records `step` as finished and saves.
    ///
    /// # Errors
    ///
    /// An I/O error.
    pub fn finish(&mut self, step: Step, profile_dir: &Path) -> Result<(), LayoutError> {
        if !self.has(step) {
            self.done.push(step);
        }
        self.save(profile_dir)
    }

    /// Removes the state file (the last act of a migration).
    ///
    /// # Errors
    ///
    /// An I/O error other than "not found".
    pub fn clear(profile_dir: &Path) -> Result<(), LayoutError> {
        match std::fs::remove_file(profile_dir.join(STATE_FILE)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// A user id as one safe file-name segment.
fn sanitize(user: &str) -> String {
    let clean: String = user
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if clean.is_empty() {
        "user".to_owned()
    } else {
        clean
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
