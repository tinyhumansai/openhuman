//! Which storage the process uses: the resolution rule behind
//! `OPENHUMAN_STORAGE_URL`, `[storage] url` and the default.
//!
//! In order:
//!
//! 1. `OPENHUMAN_STORAGE_URL`;
//! 2. `[storage] url` in `config.toml`;
//! 3. the default ([`default_url`]): `sqlite:<user dir>/storage.db` for a
//!    layout-v2 profile, `sqlite:<workspace_dir>/storage` for a v1 one.
//!
//! Every host opens the resolved URL once at startup and installs it
//! ([`super::install`]); every domain then reaches storage through it, and the
//! session store follows (`DriverSessionStores`). There is no second layout:
//! the per-domain `.db` files, JSONL transcripts and JSON stores older
//! installs wrote are read once, read-only, by [`super::import`] and never
//! consulted again.
//!
//! The default is rooted at the **workspace**, so a different workspace is a
//! different database. When the desktop rebinds its workspace (a different
//! user signs in) the host re-resolves the URL for the new workspace and
//! reinstalls the backend ([`super::rebind`]). An explicit URL does not follow
//! the workspace.
//!
//! A SaaS process has no workspace-local state to default to: with no explicit
//! URL it resolves none, and its host refuses to start without one.

use std::path::Path;

use crate::config::schema::profile_layout::{LayoutVersion, ProfileLayout};
use crate::config::Config;

/// The directory under the workspace the v1 default backend lives in.
pub const DEFAULT_DIR: &str = "storage";

/// The retired opt-out values (`classic`, `legacy`). They are no longer a mode:
/// a URL set to one of them is read as unset, with a warning.
const RETIRED_OPT_OUTS: [&str; 2] = ["classic", "legacy"];

/// Where the URL came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageMode {
    /// A backend named by `OPENHUMAN_STORAGE_URL` or `[storage] url`.
    Url(String),
    /// Nothing configured: the default for the workspace
    /// ([`default_url`]).
    Default,
}

/// The mode `env` and `config` ask for. Blank values count as unset, a blank
/// override falls through to the file, and a retired opt-out value
/// (`classic`) is skipped with a warning.
pub fn mode_from(env: Option<String>, config: &Config) -> StorageMode {
    let chosen = env
        .into_iter()
        .chain(config.storage.url.clone())
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
        .find(|url| {
            let retired = RETIRED_OPT_OUTS.iter().any(|a| url.eq_ignore_ascii_case(a));
            if retired {
                tracing::warn!(
                    target: "openhuman::storage",
                    "[storage] the `classic` storage mode was retired; using the default backend"
                );
            }
            !retired
        });
    match chosen {
        None => StorageMode::Default,
        Some(url) => StorageMode::Url(url),
    }
}

/// [`mode_from`] with the environment read.
pub fn mode(config: &Config) -> StorageMode {
    mode_from(std::env::var(super::STORAGE_URL_VAR).ok(), config)
}

/// The default URL for a profile of layout `layout`: `sqlite:<dir>/storage.db`
/// for V2 (one file for the whole profile), `sqlite:<workspace>/storage` for
/// V1 (SQLite in directory mode, one file per named database).
pub fn default_url_for(layout: &ProfileLayout) -> String {
    match layout.version {
        LayoutVersion::V2 => format!("sqlite:{}", layout.storage_db.display()),
        LayoutVersion::V1 => format!(
            "sqlite:{}",
            layout.workspace_dir.join(DEFAULT_DIR).display()
        ),
    }
}

/// The default URL for `workspace_dir`. A V2 profile's files directory
/// ([`ProfileLayout::of_workspace`]) gets its `storage.db`; any other
/// workspace is V1 and gets the directory-mode backend inside it.
pub fn default_url(workspace_dir: &Path) -> String {
    match ProfileLayout::of_workspace(workspace_dir) {
        Some(layout) => default_url_for(&layout),
        None => format!("sqlite:{}", workspace_dir.join(DEFAULT_DIR).display()),
    }
}

/// Creates what [`default_url`] opens into: the profile directory for a V2
/// `storage.db`, `<workspace>/storage` for V1.
///
/// # Errors
///
/// When the directory cannot be created.
pub fn prepare_default_dir(workspace_dir: &Path) -> std::io::Result<()> {
    match ProfileLayout::of_workspace(workspace_dir) {
        Some(layout) => std::fs::create_dir_all(&layout.dir),
        None => std::fs::create_dir_all(workspace_dir.join(DEFAULT_DIR)),
    }
}

/// The URL in effect for `mode`: the configured one, or the default under
/// `workspace_dir`.
pub fn effective_url(mode: &StorageMode, workspace_dir: &Path) -> String {
    match mode {
        StorageMode::Url(url) => url.clone(),
        StorageMode::Default => default_url(workspace_dir),
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
