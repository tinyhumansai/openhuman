//! Local-data reset: removing the active user's directory and the shared
//! root markers, plus the path-discovery RPCs the Tauri shell uses to do the
//! same from outside the core process.

use crate::core::runtime::DispatchCtx;
use std::path::Path;

use serde_json::json;

use super::paths::{active_workspace_marker_path, config_openhuman_dir, default_openhuman_dir};
use crate::core::Outcome;

pub(crate) fn is_windows_file_lock_error(error: &std::io::Error) -> bool {
    cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33))
}

pub(crate) fn reset_local_data_remove_error(path: &Path, error: &std::io::Error) -> String {
    if is_windows_file_lock_error(error) {
        tracing::warn!(
            path = %path.display(),
            error = %error,
            "[config] reset_local_data: Windows file lock blocked local data deletion"
        );
        return format!(
            "Failed to remove {} because it is locked by another OpenHuman window or process. Close all OpenHuman windows and try again. ({error})",
            path.display()
        );
    }

    format!("Failed to remove {}: {error}", path.display())
}

pub(crate) fn reset_local_data_marker_remove_error(path: &Path, error: &std::io::Error) -> String {
    // This is called for every root-level marker (active_workspace.toml,
    // active_user.toml, …), so the wording is derived from the actual file
    // name rather than hardcoded to one marker.
    let marker_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("marker");

    if is_windows_file_lock_error(error) {
        tracing::warn!(
            marker = %path.display(),
            error = %error,
            "[config] reset_local_data: Windows file lock blocked marker deletion"
        );
        return format!(
            "Failed to remove marker {} ({marker_name}) because it is locked by another OpenHuman window or process. Close all OpenHuman windows and try again. ({error})",
            path.display()
        );
    }

    format!(
        "Failed to remove marker {} ({marker_name}): {error}",
        path.display()
    )
}

/// Internal helper to reset local data for the **active user only**.
///
/// Removes the current user's data directory (`~/.openhuman/users/<id>`) plus
/// the two shared marker files at the root — `active_workspace.toml` and
/// `active_user.toml` — so the next launch boots signed-out into the
/// pre-login (`users/local`) scope.
///
/// It deliberately does **not** delete the shared root `~/.openhuman`
/// directory: that root holds every user's `users/<other>` subtree, and
/// wiping it during a single user's "Clear App Data" destroyed sibling
/// accounts' data (the scoping bug this replaces). The root is left in place;
/// only the current user's slice and the active markers are removed.
pub(crate) async fn reset_local_data_for_paths(
    current_openhuman_dir: &Path,
    default_openhuman_dir: &Path,
) -> Result<Outcome<serde_json::Value>, String> {
    let active_workspace_marker = active_workspace_marker_path(default_openhuman_dir);
    let active_user_marker = crate::config::active_user_marker_path(default_openhuman_dir);
    tracing::debug!(
        current_dir = %current_openhuman_dir.display(),
        default_dir = %default_openhuman_dir.display(),
        workspace_marker = %active_workspace_marker.display(),
        user_marker = %active_user_marker.display(),
        "[config] reset_local_data: starting (user-scoped)"
    );

    let mut removed_paths = Vec::new();

    // Remove the two shared root-level markers so the current user is signed
    // out and any non-default workspace pointer is dropped. Each is a single
    // file under the root; the root itself is preserved for sibling users.
    for marker in [&active_workspace_marker, &active_user_marker] {
        if marker.exists() {
            if let Err(error) = tokio::fs::remove_file(marker).await {
                return Err(reset_local_data_marker_remove_error(marker, &error));
            }
            tracing::debug!(
                marker = %marker.display(),
                "[config] reset_local_data: removed marker"
            );
            removed_paths.push(marker.display().to_string());
        }
    }

    // Remove only the active user's directory — NOT the shared root, which
    // contains other users' `users/<id>` subtrees.
    if current_openhuman_dir.exists() {
        if let Err(error) = tokio::fs::remove_dir_all(current_openhuman_dir).await {
            return Err(reset_local_data_remove_error(current_openhuman_dir, &error));
        }
        tracing::debug!(
            dir = %current_openhuman_dir.display(),
            "[config] reset_local_data: removed current user directory"
        );
        removed_paths.push(current_openhuman_dir.display().to_string());
    } else {
        tracing::debug!(
            dir = %current_openhuman_dir.display(),
            "[config] reset_local_data: current user directory already absent"
        );
    }

    Ok(Outcome::new(
        json!({
            "removed_paths": removed_paths,
            "current_openhuman_dir": current_openhuman_dir.display().to_string(),
            "default_openhuman_dir": default_openhuman_dir.display().to_string(),
        }),
        vec![format!(
            "reset local data for active user dir {} (shared root {} preserved)",
            current_openhuman_dir.display(),
            default_openhuman_dir.display()
        )],
    ))
}

/// Deletes all local data directories and workspace markers.
///
/// Runs **inside the core's tokio task**, which means the running core
/// holds open handles to SQLite databases, log files, the Sentry session
/// store, etc. On Windows, `remove_dir_all` therefore fails with
/// `ERROR_SHARING_VIOLATION` (os error 32) — see OPENHUMAN-TAURI-AF.
///
/// GUI callers must use the Tauri-side `reset_local_data` command instead:
/// it stops the embedded core via `CoreProcessHandle::shutdown` (dropping
/// the file handles), removes the directories from the Tauri host process,
/// and restarts the core. This JSON-RPC method is kept for headless / CLI
/// callers where in-process removal is acceptable (POSIX file semantics
/// tolerate unlinking open files; on Windows the CLI invocation runs
/// without the core attached, so no handle is in the way).
pub async fn reset_local_data(ctx: &DispatchCtx) -> Result<Outcome<serde_json::Value>, String> {
    let config = ctx.load_config().await?;
    let current_openhuman_dir = config_openhuman_dir(&config);
    let default_openhuman_dir = default_openhuman_dir();
    reset_local_data_for_paths(&current_openhuman_dir, &default_openhuman_dir).await
}

/// Reports the resolved paths that `reset_local_data` would remove, without
/// performing any filesystem changes.
///
/// Lets the Tauri-side `reset_local_data` command discover the active
/// workspace dir, the default `~/.openhuman` dir (which can differ when
/// `OPENHUMAN_WORKSPACE` is set or a staging build is in use), and the
/// active workspace marker file **before** the core sidecar is shut down —
/// after which the Tauri shell removes them while no process holds open
/// handles. See OPENHUMAN-TAURI-AF for the Windows file-locking failure
/// that motivated the split.
pub async fn get_data_paths(ctx: &DispatchCtx) -> Result<Outcome<serde_json::Value>, String> {
    let config = ctx.load_config().await?;
    let current_openhuman_dir = config_openhuman_dir(&config);
    let default_openhuman_dir = default_openhuman_dir();
    let active_workspace_marker = active_workspace_marker_path(&default_openhuman_dir);
    // The active-user marker lives at the *shared* root `~/.openhuman`, not
    // inside the per-user dir. A clear removes it (to sign the current user
    // out) but must leave the sibling `users/<other>` dirs and the root
    // itself intact — see `reset_local_data_for_paths`.
    let active_user_marker = crate::config::active_user_marker_path(&default_openhuman_dir);
    Ok(Outcome::new(
        json!({
            "current_openhuman_dir": current_openhuman_dir.display().to_string(),
            "default_openhuman_dir": default_openhuman_dir.display().to_string(),
            "active_workspace_marker_path": active_workspace_marker.display().to_string(),
            "active_user_marker_path": active_user_marker.display().to_string(),
        }),
        vec![format!(
            "data paths resolved (current={}, default={})",
            current_openhuman_dir.display(),
            default_openhuman_dir.display()
        )],
    ))
}

/// Like [`get_data_paths`], but resolves the current data dir directly from an
/// explicit `user_id` (`~/.openhuman/users/<user_id>`) instead of the
/// active-user marker.
///
/// Root cause of #4950 ("Clear App Data does nothing"): the GUI clear flow
/// signs the user out *before* it asks the Tauri shell which directory to
/// delete. Signing out (`auth_clear_session`) removes `active_user.toml`, so a
/// marker-based resolution here falls back to the pre-login `users/local` dir —
/// the reset then deletes an empty directory and leaves the signed-in user's
/// memory / conversations / cron / thread history under `users/<id>` fully
/// intact. Passing the id the UI already holds pins the deletion to the correct
/// user regardless of marker state, so the clear actually clears.
///
/// `user_id` is expected to be non-empty and pre-trimmed (the controller
/// enforces this); an empty id would resolve to the bare `users/` parent, which
/// the caller must never delete.
///
/// **Security:** `user_id` is caller-controlled (it arrives over `/rpc` and via
/// the Tauri `reset_local_data` command, whose renderer runs untrusted webview
/// content), and the returned `current_openhuman_dir` is handed straight to
/// `remove_dir_all`. An absolute id (`/etc`) or one with `..` / separators would
/// let `Path::join` resolve a delete target OUTSIDE `<root>/users/<id>`. We
/// therefore reject anything that isn't a single plain path segment and, as
/// defense in depth, verify the resolved dir is a direct child of `users/`.
pub async fn get_data_paths_for_user(user_id: &str) -> Result<Outcome<serde_json::Value>, String> {
    if !is_plain_user_id(user_id) {
        return Err(format!(
            "refusing to resolve data paths for unsafe user id {user_id:?}: must be a single path segment with no separators, `.` or `..`"
        ));
    }
    let default_openhuman_dir = default_openhuman_dir();
    let current_openhuman_dir = crate::config::user_openhuman_dir(&default_openhuman_dir, user_id);
    // Defense in depth: the resolved user dir MUST be a direct child of
    // `<root>/users`. Catches any platform-specific `join` quirk (e.g. a
    // Windows drive-relative id) that slipped past the string check above,
    // before the path reaches `remove_dir_all`.
    let users_root = default_openhuman_dir.join("users");
    if current_openhuman_dir.parent() != Some(users_root.as_path()) {
        return Err(format!(
            "refusing to resolve data paths: resolved dir {} is not a direct child of {}",
            current_openhuman_dir.display(),
            users_root.display()
        ));
    }
    let active_workspace_marker = active_workspace_marker_path(&default_openhuman_dir);
    let active_user_marker = crate::config::active_user_marker_path(&default_openhuman_dir);
    // Content-free logging only: the user id and the user-scoped paths are PII
    // (AGENTS.md: never log secrets/PII), so emit a boolean indicator instead of
    // the id or the resolved dirs. The paths are still returned in the JSON
    // result below for the caller that actually needs them.
    log::debug!("[config] get_data_paths_for_user: explicit_user_id=true");
    Ok(Outcome::new(
        json!({
            "current_openhuman_dir": current_openhuman_dir.display().to_string(),
            "default_openhuman_dir": default_openhuman_dir.display().to_string(),
            "active_workspace_marker_path": active_workspace_marker.display().to_string(),
            "active_user_marker_path": active_user_marker.display().to_string(),
        }),
        vec!["data paths resolved (explicit_user_id=true)".to_string()],
    ))
}

/// True when `user_id` is a single plain path segment safe to join onto the
/// `users/` root: non-empty, not `.`/`..`, and free of path separators or NUL.
/// Rejecting everything else keeps [`get_data_paths_for_user`] (and the
/// `remove_dir_all` it feeds) from escaping `<root>/users/<id>`.
fn is_plain_user_id(user_id: &str) -> bool {
    !user_id.is_empty() && user_id != "." && user_id != ".." && !user_id.contains(['/', '\\', '\0'])
}
