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

/// Reads the active user id, distinguishing a legitimately signed-out state
/// (`Ok(None)`) from a transient/unexpected failure to read an *existing*
/// marker (`Err`).
///
/// - `Ok(None)` — the marker is absent, empty, or holds a whitespace-only or
///   unparseable id. The user is signed out; callers resolve to the pre-login
///   directory.
/// - `Ok(Some(id))` — the active user is `id`.
/// - `Err(_)` — the marker could not be read for a reason other than "not
///   found" (e.g. a Windows `ERROR_SHARING_VIOLATION`/`ERROR_ACCESS_DENIED`
///   from an antivirus, backup, or search-indexer briefly locking the file, or
///   a permission fault). The read is retried with backoff first; a persistent
///   failure is surfaced rather than laundered into `None`.
///
/// Laundering a transient read fault into `None` is the root cause of the
/// "app reset itself" data-loss reports (#5334): the directory resolver would
/// treat "can't read the marker" as "signed out" and boot the user into
/// `users/local` — a fresh, empty profile — while their real avatar, settings,
/// and memory sit intact under `users/<id>`. The `config.toml` read is already
/// retry-guarded (`read_config_with_recovery_or_default`); the marker that
/// decides *which* config to load must be at least as resilient.
pub fn read_active_user_id_checked(default_openhuman_dir: &Path) -> Result<Option<String>> {
    let path = active_user_marker_path(default_openhuman_dir);
    // `retry_with_backoff` bails immediately on a non-transient error (incl.
    // `NotFound`), and only re-attempts the Windows file-lock class of faults —
    // exactly the resilience the config-file read already has. This variant is
    // synchronous for the best-effort sync callers reached via
    // [`read_active_user_id`]; async callers must use
    // [`read_active_user_id_checked_async`] to avoid blocking sleeps.
    let read = crate::openhuman::util::retry_with_backoff("read active_user.toml", 4, 20, || {
        std::fs::read_to_string(&path)
            .with_context(|| format!("read active user marker: {}", path.display()))
    });
    interpret_active_user_marker(&path, read)
}

/// Async counterpart of [`read_active_user_id_checked`] for callers already on
/// the tokio executor (the config-directory resolver).
///
/// It retries with `retry_with_backoff_async` so a transient-lock backoff never
/// blocks a worker thread with `std::thread::sleep` — the same reason the
/// `config.toml` read uses the async helper
/// (`read_config_with_recovery_or_default`). The outcome classification
/// (missing → `Ok(None)`, other read fault → `Err`, unparseable → `Ok(None)`)
/// is shared with the sync variant so the two cannot drift.
pub async fn read_active_user_id_checked_async(
    default_openhuman_dir: &Path,
) -> Result<Option<String>> {
    let path = active_user_marker_path(default_openhuman_dir);
    let read =
        crate::openhuman::util::retry_with_backoff_async("read active_user.toml", 4, 20, || {
            let path = path.clone();
            async move {
                tokio::fs::read_to_string(&path)
                    .await
                    .with_context(|| format!("read active user marker: {}", path.display()))
            }
        })
        .await;
    interpret_active_user_marker(&path, read)
}

/// Shared classification of an (already retried) active-user marker read, so the
/// sync and async readers above cannot drift. Maps the raw read result to the
/// active user id, applying the missing/unparseable → signed-out and
/// unexpected-fault → error policy documented on
/// [`read_active_user_id_checked`].
fn interpret_active_user_marker(path: &Path, read: Result<String>) -> Result<Option<String>> {
    let contents = match read {
        Ok(contents) => contents,
        Err(err) => {
            // A genuinely missing marker is the expected signed-out state, not a
            // fault: fall through to the pre-login profile exactly as before.
            let missing = err.chain().any(|cause| {
                cause
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
            });
            if missing {
                return Ok(None);
            }
            tracing::warn!(
                path = %path.display(),
                error = %format!("{err:#}"),
                "[config] active_user.toml exists but could not be read; refusing to \
                 downgrade to the pre-login profile so a signed-in user's data under \
                 users/<id> is not orphaned"
            );
            return Err(err);
        }
    };

    // A corrupt (unparseable) marker cannot identify the active user. Treat it
    // as signed-out rather than an error: the id is re-established atomically on
    // the next login, and failing the whole boot on a malformed tiny file would
    // be worse than resolving to the pre-login profile.
    let state: ActiveUserState = match toml::from_str(&contents) {
        Ok(state) => state,
        Err(parse_err) => {
            tracing::warn!(
                path = %path.display(),
                error = %parse_err,
                "[config] active_user.toml is unparseable; treating as signed out"
            );
            return Ok(None);
        }
    };
    let id = state.user_id.trim().to_string();
    Ok(if id.is_empty() { None } else { Some(id) })
}

/// Best-effort variant of [`read_active_user_id_checked`]: a transient or
/// unexpected read failure collapses to `None`.
///
/// Retained for callers that only need a hint (session-present guards,
/// informational logging) and can tolerate a false "signed out" on a transient
/// fault. The **directory-resolution** path must use
/// [`read_active_user_id_checked`] instead so it never silently orphans a
/// signed-in user's data (#5334).
pub fn read_active_user_id(default_openhuman_dir: &Path) -> Option<String> {
    read_active_user_id_checked(default_openhuman_dir)
        .ok()
        .flatten()
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
