//! Where one user profile's state lives: the layout the desktop and a SaaS
//! core share.
//!
//! Two layouts exist on disk ([`LayoutVersion`]). V1, every install before
//! storage schema v2:
//!
//! ```text
//! <root>/users/<profile-id>/
//!   profile.toml      SaaS profile metadata (the desktop does not write it)
//!   config.toml       the profile's config_path
//!   workspace/        sessions, memory, threads, cron, cost — internal state
//!   sandbox/          a SaaS profile's action_dir
//! ```
//!
//! V2, a brand-new desktop profile (epic #7973): all state that used to be
//! loose files lives in one database, and `Config.workspace_dir` is the
//! profile's user-visible `files/` directory:
//!
//! ```text
//! <root>/users/<profile-id>/
//!   .layout           marker: `2`
//!   config.toml       the profile's config_path
//!   storage.db        every store (sessions, memory, threads, cron, ...)
//!   files/            workspace_dir: files the user and agent work with
//! ```
//!
//! The version is decided from disk ([`LayoutVersion::detect`]): a `.layout`
//! marker reading `2` is V2, an empty or missing profile directory starts as
//! V2, anything else is V1 and behaves exactly as it always has. Moving a V1
//! profile to V2 is the migration of #7997, not this module.
//!
//! On the desktop `<root>` is `~/.openhuman` and the profile id is the
//! signed-in user's id (or `local` before login); the config loader resolves
//! `config.toml` and `workspace/` from here
//! (`load::dirs::resolve_config_dirs_ignoring_env`). A SaaS core roots it at
//! its operator-chosen directory (`profiles::layout`). Both build every path
//! through [`ProfileLayout::new`], so the two cannot drift apart.

use std::path::{Path, PathBuf};

use super::user_openhuman_dir;

/// The profile metadata file, beside `config.toml`.
pub const PROFILE_META_FILE: &str = "profile.toml";
/// The profile's config file.
pub const PROFILE_CONFIG_FILE: &str = "config.toml";
/// The profile's internal-state directory.
pub const PROFILE_WORKSPACE_DIR: &str = "workspace";
/// The profile's sandbox (a SaaS profile's `action_dir`).
pub const PROFILE_SANDBOX_DIR: &str = "sandbox";
/// The V2 profile database, beside `config.toml`.
pub const PROFILE_STORAGE_FILE: &str = "storage.db";
/// The V2 workspace: the profile's files directory.
pub const PROFILE_FILES_DIR: &str = "files";
/// The marker file recording a profile's layout version.
pub const PROFILE_LAYOUT_MARKER: &str = ".layout";

/// Which on-disk layout a profile directory uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutVersion {
    /// `workspace/` holds the internal state; no marker.
    #[default]
    V1,
    /// `storage.db` holds the state; `files/` is the workspace.
    V2,
}

impl LayoutVersion {
    /// The version `dir` is, from disk and without writing anything.
    ///
    /// A `.layout` marker reading `2` is V2. A missing or empty directory is a
    /// brand-new profile and starts as V2. Anything else (an install that
    /// predates the marker) is V1. An unreadable directory is V1: the
    /// conservative reading never moves data.
    pub fn detect(dir: &Path) -> Self {
        match std::fs::read_to_string(dir.join(PROFILE_LAYOUT_MARKER)) {
            Ok(marker) => {
                return if marker.trim() == "2" {
                    Self::V2
                } else {
                    Self::V1
                };
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Self::V1,
            Err(_) => {}
        }
        match std::fs::read_dir(dir) {
            Ok(mut entries) => {
                if entries.next().is_none() {
                    Self::V2
                } else {
                    Self::V1
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::V2,
            Err(_) => Self::V1,
        }
    }

    /// Whether `dir` carries the V2 marker (no fresh-directory rule).
    pub fn is_marked_v2(dir: &Path) -> bool {
        std::fs::read_to_string(dir.join(PROFILE_LAYOUT_MARKER))
            .map(|marker| marker.trim() == "2")
            .unwrap_or(false)
    }

    /// Writes the V2 marker into `dir`, creating the directory.
    ///
    /// # Errors
    ///
    /// When the directory or the marker cannot be written.
    pub fn mark_v2(dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join(PROFILE_LAYOUT_MARKER), "2\n")
    }
}

/// Resolved paths of one profile under a root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileLayout {
    /// Which layout this is.
    pub version: LayoutVersion,
    /// `<root>/users/<id>`.
    pub dir: PathBuf,
    /// `<dir>/profile.toml`.
    pub meta_path: PathBuf,
    /// `<dir>/config.toml`.
    pub config_path: PathBuf,
    /// `Config.workspace_dir`: `<dir>/workspace` (V1) or `<dir>/files` (V2).
    pub workspace_dir: PathBuf,
    /// `<dir>/storage.db`: the V2 database (V1 profiles do not have one).
    pub storage_db: PathBuf,
    /// `<dir>/files`: the V2 files directory, the workspace under V2.
    pub files_dir: PathBuf,
    /// `<dir>/sandbox`.
    pub sandbox_dir: PathBuf,
}

impl ProfileLayout {
    /// The V1 layout of profile `id` under `root`, with no disk access. The id
    /// is used as one path segment; callers pass a validated id
    /// (`profiles::ProfileId`, or the desktop's active user id).
    pub fn new(root: &Path, id: impl AsRef<str>) -> Self {
        Self::at(user_openhuman_dir(root, id.as_ref()))
    }

    /// The V1 layout of the profile whose directory is `dir`.
    pub fn at(dir: PathBuf) -> Self {
        Self::with_version(dir, LayoutVersion::V1)
    }

    /// The layout of `dir` as version `version`, with no disk access.
    pub fn with_version(dir: PathBuf, version: LayoutVersion) -> Self {
        let files_dir = dir.join(PROFILE_FILES_DIR);
        let workspace_dir = match version {
            LayoutVersion::V1 => dir.join(PROFILE_WORKSPACE_DIR),
            LayoutVersion::V2 => files_dir.clone(),
        };
        Self {
            version,
            meta_path: dir.join(PROFILE_META_FILE),
            config_path: dir.join(PROFILE_CONFIG_FILE),
            storage_db: dir.join(PROFILE_STORAGE_FILE),
            files_dir,
            workspace_dir,
            sandbox_dir: dir.join(PROFILE_SANDBOX_DIR),
            dir,
        }
    }

    /// The layout of profile `id` under `root` as it is on disk
    /// ([`LayoutVersion::detect`]). A brand-new profile is V2 and gets its
    /// marker written here, so it stays V2 once files appear beside it.
    pub fn resolve(root: &Path, id: impl AsRef<str>) -> Self {
        Self::resolve_at(user_openhuman_dir(root, id.as_ref()))
    }

    /// [`Self::resolve`] for the profile whose directory is `dir`.
    pub fn resolve_at(dir: PathBuf) -> Self {
        let version = LayoutVersion::detect(&dir);
        if version == LayoutVersion::V2 && !LayoutVersion::is_marked_v2(&dir) {
            match LayoutVersion::mark_v2(&dir) {
                Ok(()) => tracing::debug!(
                    target: "openhuman::storage",
                    dir = %dir.display(),
                    "[layout] new profile directory starts as layout v2"
                ),
                Err(error) => tracing::warn!(
                    target: "openhuman::storage",
                    dir = %dir.display(),
                    %error,
                    "[layout] could not write the layout v2 marker; the profile will be read as v1 next launch"
                ),
            }
        }
        tracing::debug!(
            target: "openhuman::storage",
            dir = %dir.display(),
            version = ?version,
            "[layout] profile layout resolved"
        );
        Self::with_version(dir, version)
    }

    /// The V2 layout whose files directory is `workspace_dir`, when it is one:
    /// the directory is named `files` and its parent carries the V2 marker.
    /// `None` for every V1 workspace.
    pub fn of_workspace(workspace_dir: &Path) -> Option<Self> {
        if workspace_dir.file_name()? != std::ffi::OsStr::new(PROFILE_FILES_DIR) {
            return None;
        }
        let dir = workspace_dir.parent()?;
        LayoutVersion::is_marked_v2(dir)
            .then(|| Self::with_version(dir.to_path_buf(), LayoutVersion::V2))
    }
}

/// The directory under a root that holds every profile.
pub const USERS_DIR: &str = "users";

/// `<root>/users`: the parent of every profile directory. The desktop's
/// [`user_openhuman_dir`] is built on it.
pub fn users_dir(root: &Path) -> PathBuf {
    root.join(USERS_DIR)
}

#[cfg(test)]
#[path = "profile_layout_tests.rs"]
mod tests;
