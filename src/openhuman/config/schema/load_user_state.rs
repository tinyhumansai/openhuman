use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

pub(crate) const ACTIVE_USER_STATE_FILE: &str = "active_user.toml";

#[derive(Debug, Serialize, Deserialize)]
struct ActiveUserState {
    user_id: String,
}

/// Returns the path to the active-user marker:
/// `{default_openhuman_dir}/active_user.toml`.
///
/// This marker is **shared across all users** — it lives at the root
/// `~/.openhuman` dir, not inside any per-user directory — so it records
/// *which* user is currently active. Clearing one user's data must remove
/// this marker (to sign that user out) without touching the sibling
/// `users/<other>` directories.
pub fn active_user_marker_path(default_openhuman_dir: &Path) -> PathBuf {
    default_openhuman_dir.join(ACTIVE_USER_STATE_FILE)
}

/// Reads the active user id from `{default_openhuman_dir}/active_user.toml`.
/// Returns `None` when the file does not exist, is empty, or cannot be parsed.
pub fn read_active_user_id(default_openhuman_dir: &Path) -> Option<String> {
    let path = active_user_marker_path(default_openhuman_dir);
    let contents = std::fs::read_to_string(&path).ok()?;
    let state: ActiveUserState = toml::from_str(&contents).ok()?;
    let id = state.user_id.trim().to_string();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

/// Writes the active user id to `{default_openhuman_dir}/active_user.toml`.
pub fn write_active_user_id(default_openhuman_dir: &Path, user_id: &str) -> Result<()> {
    std::fs::create_dir_all(default_openhuman_dir).with_context(|| {
        format!(
            "Failed to create active user state directory: {}",
            default_openhuman_dir.display()
        )
    })?;

    let path = default_openhuman_dir.join(ACTIVE_USER_STATE_FILE);
    let state = ActiveUserState {
        user_id: user_id.to_string(),
    };
    let toml_str = toml::to_string_pretty(&state).context("serialize active_user.toml")?;
    let temp_path = default_openhuman_dir.join(format!(
        ".{ACTIVE_USER_STATE_FILE}.tmp-{}",
        uuid::Uuid::new_v4()
    ));

    let mut temp_file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_path)
        .with_context(|| {
            format!(
                "Failed to create temporary active user state: {}",
                temp_path.display()
            )
        })?;
    temp_file
        .write_all(toml_str.as_bytes())
        .context("Failed to write temporary active user state")?;
    temp_file
        .sync_all()
        .context("Failed to fsync temporary active user state")?;
    drop(temp_file);

    if let Err(error) = std::fs::rename(&temp_path, &path) {
        let _ = std::fs::remove_file(&temp_path);
        anyhow::bail!(
            "Failed to atomically persist active user state {}: {error}",
            path.display()
        );
    }

    sync_directory(default_openhuman_dir)?;
    tracing::debug!(user_id = %user_id, path = %path.display(), "active user written");
    Ok(())
}

/// Removes the active user marker.  After this, the next config load will
/// use the default (unauthenticated) openhuman directory.
pub fn clear_active_user(default_openhuman_dir: &Path) -> Result<()> {
    let path = active_user_marker_path(default_openhuman_dir);
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("Failed to remove active user state: {}", path.display()))?;
        tracing::debug!(path = %path.display(), "active user cleared");
    }
    Ok(())
}

/// Returns the user-scoped openhuman directory for the given user id:
/// `{default_openhuman_dir}/users/{user_id}`.
pub fn user_openhuman_dir(default_openhuman_dir: &Path, user_id: &str) -> PathBuf {
    default_openhuman_dir.join("users").join(user_id)
}

/// Stable id used to scope the openhuman directory before any user has
/// logged in.  All memory, state, config, sessions and workspace files
/// created on first init land under `{root}/users/{PRE_LOGIN_USER_ID}`
/// so nothing is ever written directly at the root `.openhuman` path.
///
/// On first successful login, this directory is migrated into the real
/// user-scoped directory (see `credentials::ops::store_session`).
pub const PRE_LOGIN_USER_ID: &str = "local";

/// Returns the pre-login (unauthenticated) user directory:
/// `{default_openhuman_dir}/users/local`.
pub fn pre_login_user_dir(default_openhuman_dir: &Path) -> PathBuf {
    user_openhuman_dir(default_openhuman_dir, PRE_LOGIN_USER_ID)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    let dir = std::fs::File::open(path)
        .with_context(|| format!("Failed to open directory for fsync: {}", path.display()))?;
    dir.sync_all()
        .with_context(|| format!("Failed to fsync directory metadata: {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}
