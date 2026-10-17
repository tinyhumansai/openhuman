//! Moving a v1 profile to layout v2 (epic #7973, sub-issue #7997).
//!
//! A v1 profile keeps its state in `users/<id>/workspace/` (loose files plus a
//! SQLite directory backend at `workspace/storage/`). Layout v2 keeps it in one
//! `users/<id>/storage.db` and uses `users/<id>/files/` as the workspace
//! ([`crate::config::schema::profile_layout`]). [`migrate`] moves a profile
//! from one to the other without losing anything:
//!
//! 1. **Lock** `users/<id>/.layout.lock` ([`state::Lock`]).
//! 2. **Import** the legacy files into the v1 database ([`legacy::import`]).
//! 3. **Migrate** the schema ([`legacy::migrate`]) and classify the workspace
//!    ([`plan`]).
//! 4. **Copy the database** with `VACUUM INTO` and verify its counts
//!    ([`db::copy`]).
//! 5. **Move user files** into `files/`.
//! 6. **Archive** every moved and deleted entry to
//!    `<root>/backups/<user>-<timestamp>.tar.zst` and verify it
//!    ([`archive`]).
//! 7. **Finish**: write the `.layout` marker `2`, delete the archived
//!    originals, remove the progress file. Entries nobody classified are left
//!    where they are and listed.
//!
//! Progress is kept in `users/<id>/.layout.state` ([`state::LayoutState`]);
//! each step is idempotent and recorded after it finished, so a crash at any
//! point leaves a state the next [`migrate`] resumes. Nothing is deleted before
//! the archive verified and the marker is written.
//!
//! It does nothing for SaaS mode, for an explicit storage URL
//! (`OPENHUMAN_STORAGE_URL`, `[storage] url`: only the schema migrations run,
//! which opening the URL does) and for a profile already on v2. The controllers
//! `storage.migrate`, `storage.status` and `storage.restore` ([`schemas`]) are
//! the surface; `[storage] auto_migrate_layout = true` runs [`migrate`] at
//! boot ([`auto_migrate`]).
//!
//! **Offline only.** A running core writes to the v1 database; a migration
//! snapshots it. [`migrate`] refuses when this process serves the profile's v1
//! database ([`LayoutError::Live`]); it cannot see another process, so stop the
//! core before migrating from the CLI.

pub mod archive;
pub mod db;
pub mod legacy;
pub mod plan;
mod run;
pub mod schemas;
pub mod state;
mod status;

use std::path::{Path, PathBuf};

use serde::Serialize;

pub use run::{auto_migrate, migrate, restore, Options};
pub use state::{LayoutError, Step};
pub use status::{status, Status};

use crate::config::schema::profile_layout::{
    PROFILE_FILES_DIR, PROFILE_STORAGE_FILE, PROFILE_WORKSPACE_DIR,
};
use crate::config::Config;

/// Directory under the root that holds the archives.
pub const BACKUPS_DIR: &str = "backups";

/// One profile: its directory and the root it lives under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// `~/.openhuman` (the directory holding `users/`).
    pub root: PathBuf,
    /// `<root>/users/<id>`.
    pub profile_dir: PathBuf,
    /// The profile id (the signed-in user's id, or `local`).
    pub user_id: String,
}

impl Target {
    /// The profile `user_id` under `root`.
    pub fn new(root: &Path, user_id: &str) -> Self {
        Self {
            root: root.to_path_buf(),
            profile_dir: crate::config::schema::user_openhuman_dir(root, user_id),
            user_id: user_id.to_owned(),
        }
    }

    /// The profile `config` belongs to, when its `config.toml` sits in a
    /// `users/<id>/` directory (the desktop and SaaS layout). `None` for a
    /// config elsewhere (`OPENHUMAN_WORKSPACE`, an embed with its own paths):
    /// those have no layout to migrate.
    pub fn from_config(config: &Config) -> Option<Self> {
        let profile_dir = config.config_path.parent()?;
        let users = profile_dir.parent()?;
        if users.file_name()?
            != std::ffi::OsStr::new(crate::config::schema::profile_layout::USERS_DIR)
        {
            return None;
        }
        let user_id = profile_dir.file_name()?.to_str()?.to_owned();
        Some(Self {
            root: users.parent()?.to_path_buf(),
            profile_dir: profile_dir.to_path_buf(),
            user_id,
        })
    }

    /// `<profile>/workspace`, the v1 workspace.
    pub fn workspace(&self) -> PathBuf {
        self.profile_dir.join(PROFILE_WORKSPACE_DIR)
    }

    /// `<profile>/files`, the v2 workspace.
    pub fn files(&self) -> PathBuf {
        self.profile_dir.join(PROFILE_FILES_DIR)
    }

    /// `<profile>/storage.db`, the v2 database.
    pub fn storage_db(&self) -> PathBuf {
        self.profile_dir.join(PROFILE_STORAGE_FILE)
    }

    /// `<root>/backups`.
    pub fn backups(&self) -> PathBuf {
        self.root.join(BACKUPS_DIR)
    }
}

/// Why a migration did nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Noop {
    /// SaaS mode: profiles are not migrated in place.
    Saas,
    /// An explicit storage URL names the backend (including MongoDB); only
    /// the schema migrations apply to it.
    ExplicitUrl,
    /// The profile is already on layout v2 (or is brand new).
    AlreadyV2,
}

/// The dry run: what a migration would do. Nothing was written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DryRun {
    /// Files and bytes to archive (the moved and the deleted entries).
    pub archive: plan::Totals,
    /// Entries to move into `files/`.
    pub move_entries: Vec<plan::Entry>,
    /// Entries to delete after archiving.
    pub delete_entries: Vec<plan::Entry>,
    /// Entries left alone and listed.
    pub unknown_entries: Vec<plan::Entry>,
    /// Totals of the moved entries.
    pub move_totals: plan::Totals,
    /// Totals of the deleted entries.
    pub delete_totals: plan::Totals,
    /// Totals of the unknown entries.
    pub unknown_totals: plan::Totals,
    /// Size of the v1 database, when there is one.
    pub database_bytes: Option<u64>,
    /// Where the archive would go.
    pub archive_dir: PathBuf,
    /// Whether an interrupted migration would be resumed.
    pub resuming: bool,
}

/// A finished migration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    /// The archive written.
    pub archive: PathBuf,
    /// Files in the archive.
    pub archive_files: u64,
    /// Entries moved into `files/`.
    pub moved: Vec<String>,
    /// Entries deleted after archiving.
    pub deleted: Vec<String>,
    /// Entries left in place (unknown).
    pub unknown: Vec<String>,
    /// Row count of the copied database, when one was copied.
    pub database_rows: Option<u64>,
    /// Steps this run executed (the rest had finished in an earlier run).
    pub steps_run: Vec<Step>,
    /// A running core still serves the v1 layout until it restarts.
    pub restart_required: bool,
}

/// What [`migrate`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Outcome {
    /// Nothing to migrate.
    Noop {
        /// Why.
        reason: Noop,
    },
    /// A dry run.
    DryRun(Box<DryRun>),
    /// The migration ran to the end.
    Migrated(Box<Report>),
}

#[cfg(test)]
mod test_fixtures;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
