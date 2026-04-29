use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const CEF_CACHE_PATH_ENV: &str = "OPENHUMAN_CEF_CACHE_PATH";
const ACTIVE_USER_STATE_FILE: &str = "active_user.toml";
/// Sibling of the OpenHuman data dir (not under it) so the marker survives
/// `reset_local_data` removing the whole `default_openhuman_dir` tree.
const PENDING_PURGE_STATE_FILE: &str = "openhuman_pending_cef_purge.toml";
/// Pre–sibling-layout marker (lived under the data root; `reset_local_data` removed it).
const LEGACY_PENDING_PURGE_IN_TREE: &str = "pending_cef_purge.toml";
const PRE_LOGIN_USER_ID: &str = "local";

#[derive(Debug, Deserialize)]
struct ActiveUserState {
    user_id: String,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct PendingCefPurgeState {
    #[serde(default)]
    paths: Vec<String>,
}

fn default_root_dir_name() -> &'static str {
    let app_env = std::env::var("OPENHUMAN_APP_ENV")
        .or_else(|_| std::env::var("VITE_OPENHUMAN_APP_ENV"))
        .ok()
        .map(|value| value.trim().to_ascii_lowercase());
    if matches!(app_env.as_deref(), Some("staging")) {
        ".openhuman-staging"
    } else {
        ".openhuman"
    }
}

pub fn default_root_openhuman_dir() -> Result<PathBuf, String> {
    if let Ok(workspace) = std::env::var("OPENHUMAN_WORKSPACE") {
        let trimmed = workspace.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    let home = directories::UserDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .ok_or_else(|| "Could not find home directory".to_string())?;
    Ok(home.join(default_root_dir_name()))
}

pub fn read_active_user_id(default_openhuman_dir: &Path) -> Option<String> {
    let path = default_openhuman_dir.join(ACTIVE_USER_STATE_FILE);
    let contents = std::fs::read_to_string(path).ok()?;
    let state: ActiveUserState = toml::from_str(&contents).ok()?;
    let trimmed = state.user_id.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Returns a single safe path segment for `users/<id>/…`. Rejects traversal, separators,
/// and other inputs that would escape the intended profile root.
fn validate_user_id_for_path(user_id: &str) -> Result<String, String> {
    let trimmed = user_id.trim();
    if trimmed.is_empty() {
        return Err("user_id is empty after trim".to_string());
    }
    if matches!(trimmed, "." | "..") {
        return Err("user_id must not be '.' or '..'".to_string());
    }
    if trimmed.contains("..")
        || trimmed
            .chars()
            .any(|c| matches!(c, '/' | '\\' | '\0' | char::REPLACEMENT_CHARACTER) || c.is_control())
    {
        return Err("user_id must not contain path components or control characters".to_string());
    }
    #[cfg(windows)]
    if trimmed.contains(':') {
        return Err("user_id must not contain ':' (Windows path roots)".to_string());
    }
    if !trimmed
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '@' || c == '.')
    {
        return Err("user_id must only use [A-Za-z0-9._@-] (after trim)".to_string());
    }
    Ok(trimmed.to_string())
}

fn user_openhuman_dir(default_openhuman_dir: &Path, user_id: &str) -> Result<PathBuf, String> {
    let id = validate_user_id_for_path(user_id)?;
    Ok(default_openhuman_dir.join("users").join(&id))
}

fn cache_dir_for_user(default_openhuman_dir: &Path, user_id: &str) -> Result<PathBuf, String> {
    Ok(user_openhuman_dir(default_openhuman_dir, user_id)?.join("cef"))
}

/// `remove_dir_all` is only safe for CEF profile dirs we queued ourselves (under
/// `.../users/<id>/cef`). Rejects absolute paths outside that tree, corrupted
/// TOML, or anything that `canonicalize` would not place under
/// `default_openhuman_dir/users/…/cef`.
fn is_trusted_queued_purge_path(default_openhuman_dir: &Path, target: &Path) -> bool {
    if !target.is_absolute() {
        log::warn!(
            "[cef-profile] refusing purge: path is not absolute (possible cwd-relative TOML injection) path={}",
            target.display()
        );
        return false;
    }
    let Ok(data_root) = std::fs::canonicalize(default_openhuman_dir) else {
        log::warn!(
            "[cef-profile] refusing purge: could not canonicalize data root path={} (cannot validate purge target) target={}",
            default_openhuman_dir.display(),
            target.display()
        );
        return false;
    };
    let users_dir = data_root.join("users");
    let Ok(users_canon) = std::fs::canonicalize(&users_dir) else {
        log::warn!(
            "[cef-profile] refusing purge: could not canonicalize `users` dir under {} (target={})",
            data_root.display(),
            target.display()
        );
        return false;
    };
    let Ok(canon) = std::fs::canonicalize(target) else {
        log::warn!(
            "[cef-profile] refusing purge: could not canonicalize target (symlink/permission?) path={}",
            target.display()
        );
        return false;
    };
    if !canon.starts_with(&users_canon) {
        log::warn!(
            "[cef-profile] refusing purge: canonical path is not under users tree (possible malicious queue entry) data_root={} target_canon={}",
            data_root.display(),
            canon.display()
        );
        return false;
    }
    if canon.file_name() != Some(OsStr::new("cef")) {
        log::warn!(
            "[cef-profile] refusing purge: expected a .../users/<id>/cef directory, got file_name={:?} path={}",
            canon.file_name(),
            canon.display()
        );
        return false;
    }
    true
}

/// Marker file lives in the **parent** of the OpenHuman data root so a full
/// `remove_dir_all(default_openhuman_dir)` (e.g. from core `reset_local_data`) does
/// not delete the pending-purge list before it is processed.
fn pending_purge_marker_path(default_openhuman_dir: &Path) -> Result<PathBuf, String> {
    let parent = default_openhuman_dir.parent().ok_or_else(|| {
        "default OpenHuman data dir has no parent; cannot place CEF purge marker outside the data tree"
            .to_string()
    })?;
    Ok(parent.join(PENDING_PURGE_STATE_FILE))
}

pub fn configured_cache_path_from_env() -> Option<PathBuf> {
    std::env::var(CEF_CACHE_PATH_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn load_pending_purge_state(default_openhuman_dir: &Path) -> Result<PendingCefPurgeState, String> {
    let path = pending_purge_marker_path(default_openhuman_dir)?;
    if path.exists() {
        let raw = std::fs::read_to_string(&path).map_err(|error| {
            format!("read pending CEF purge marker {}: {error}", path.display())
        })?;
        return toml::from_str(&raw).map_err(|error| {
            format!("parse pending CEF purge marker {}: {error}", path.display())
        });
    }

    // One-time read from the legacy in-tree file (older app versions).
    let legacy = default_openhuman_dir.join(LEGACY_PENDING_PURGE_IN_TREE);
    if !legacy.exists() {
        return Ok(PendingCefPurgeState::default());
    }
    let raw = std::fs::read_to_string(&legacy).map_err(|error| {
        format!(
            "read legacy pending CEF purge marker {}: {error}",
            legacy.display()
        )
    })?;
    let state: PendingCefPurgeState = toml::from_str(&raw).map_err(|error| {
        format!(
            "parse legacy pending CEF purge marker {}: {error}",
            legacy.display()
        )
    })?;
    match save_pending_purge_state(default_openhuman_dir, &state) {
        Ok(()) => {
            let _ = std::fs::remove_file(&legacy);
            log::info!(
                "[cef-profile] migrated pending CEF purge list from {} to {}",
                legacy.display(),
                path.display()
            );
        }
        Err(err) => log::warn!(
            "[cef-profile] could not write migrated pending CEF purge marker to {}: {err}",
            path.display()
        ),
    }
    Ok(state)
}

fn save_pending_purge_state(
    default_openhuman_dir: &Path,
    state: &PendingCefPurgeState,
) -> Result<(), String> {
    std::fs::create_dir_all(default_openhuman_dir).map_err(|error| {
        format!(
            "create OpenHuman root dir {}: {error}",
            default_openhuman_dir.display()
        )
    })?;

    let path = pending_purge_marker_path(default_openhuman_dir)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create parent of pending CEF purge marker {}: {error}",
                path.display()
            )
        })?;
    }
    let raw = toml::to_string_pretty(state)
        .map_err(|error| format!("serialize pending CEF purge marker: {error}"))?;
    std::fs::write(&path, raw)
        .map_err(|error| format!("write pending CEF purge marker {}: {error}", path.display()))
}

pub fn queue_profile_purge_for_user(user_id: Option<&str>) -> Result<PathBuf, String> {
    let default_openhuman_dir = default_root_openhuman_dir()?;
    let user_id = user_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(PRE_LOGIN_USER_ID);
    let purge_path = cache_dir_for_user(&default_openhuman_dir, user_id)?;

    let mut state = load_pending_purge_state(&default_openhuman_dir)?;
    let mut unique = BTreeSet::new();
    for path in state.paths {
        unique.insert(path);
    }
    unique.insert(purge_path.display().to_string());
    state = PendingCefPurgeState {
        paths: unique.into_iter().collect(),
    };
    save_pending_purge_state(&default_openhuman_dir, &state)?;
    log::info!(
        "[cef-profile] queued purge for user={} path={}",
        user_id,
        purge_path.display()
    );
    Ok(purge_path)
}

pub fn prepare_process_cache_path() -> Result<PathBuf, String> {
    let default_openhuman_dir = default_root_openhuman_dir()?;
    drain_pending_purges(&default_openhuman_dir)?;

    let user_id_raw = read_active_user_id(&default_openhuman_dir)
        .unwrap_or_else(|| PRE_LOGIN_USER_ID.to_string());
    let user_id = match validate_user_id_for_path(&user_id_raw) {
        Ok(id) => id,
        Err(why) => {
            log::warn!(
                "[cef-profile] invalid user_id in active user state: {why}; using {}",
                PRE_LOGIN_USER_ID
            );
            PRE_LOGIN_USER_ID.to_string()
        }
    };
    let cache_dir = cache_dir_for_user(&default_openhuman_dir, &user_id)?;
    std::fs::create_dir_all(&cache_dir)
        .map_err(|error| format!("create CEF cache dir {}: {error}", cache_dir.display()))?;
    std::env::set_var(CEF_CACHE_PATH_ENV, &cache_dir);
    log::info!(
        "[cef-profile] configured CEF cache user={} path={}",
        user_id,
        cache_dir.display()
    );

    // When a real user is active, the pre-login `users/local/cef` bucket is
    // stale third-party state captured during cold-bootstrap (before
    // `active_user.toml` existed) — e.g. a Slack/WhatsApp tile added on a
    // fresh install while the process was still running on the `local`
    // fallback path. If we don't sweep it, those cookies leak into the
    // first user's session via webview pre-warm and across users when the
    // pre-login bucket is reused on subsequent fresh installs. Drop it
    // synchronously here, before CEF init, so it's safe to delete. (#900)
    if user_id != PRE_LOGIN_USER_ID {
        if let Ok(local_cef) = cache_dir_for_user(&default_openhuman_dir, PRE_LOGIN_USER_ID) {
            if local_cef.exists() {
                match std::fs::remove_dir_all(&local_cef) {
                    Ok(()) => log::info!(
                        "[cef-profile] purged stale pre-login CEF cache path={}",
                        local_cef.display()
                    ),
                    Err(error) => log::warn!(
                        "[cef-profile] failed to purge stale pre-login CEF cache path={} error={}",
                        local_cef.display(),
                        error
                    ),
                }
            }
        }
    }

    Ok(cache_dir)
}

fn drain_pending_purges(default_openhuman_dir: &Path) -> Result<(), String> {
    let marker_path = pending_purge_marker_path(default_openhuman_dir)?;
    let mut state = load_pending_purge_state(default_openhuman_dir)?;
    if state.paths.is_empty() {
        if marker_path.exists() {
            let _ = std::fs::remove_file(&marker_path);
        }
        return Ok(());
    }

    let mut remaining: Vec<String> = Vec::new();
    for raw_path in &state.paths {
        let target = PathBuf::from(raw_path);
        if !target.exists() {
            log::debug!(
                "[cef-profile] pending purge target already absent path={}",
                target.display()
            );
            continue;
        }
        if !is_trusted_queued_purge_path(default_openhuman_dir, &target) {
            log::warn!(
                "[cef-profile] skipping unsafe purge and retaining queue entry (will not delete) path={} raw_toml={}",
                target.display(),
                raw_path
            );
            remaining.push(raw_path.clone());
            continue;
        }
        match std::fs::remove_dir_all(&target) {
            Ok(()) => {
                log::info!(
                    "[cef-profile] purged queued CEF cache path={}",
                    target.display()
                );
            }
            Err(error) => {
                log::warn!(
                    "[cef-profile] failed to purge queued CEF cache path={} error={}",
                    target.display(),
                    error
                );
                remaining.push(raw_path.clone());
            }
        }
    }

    if !remaining.is_empty() {
        state.paths = remaining;
        save_pending_purge_state(default_openhuman_dir, &state)?;
        log::warn!(
            "[cef-profile] not removing pending CEF purge marker: {} path(s) still fail purge (will retry) marker={}",
            state.paths.len(),
            marker_path.display()
        );
        return Ok(());
    }

    if marker_path.exists() {
        std::fs::remove_file(&marker_path).map_err(|error| {
            format!(
                "remove pending CEF purge marker {}: {error}",
                marker_path.display()
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_active_user_id_ignores_empty_values() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(ACTIVE_USER_STATE_FILE), "user_id = \"   \"").unwrap();
        assert_eq!(read_active_user_id(tmp.path()), None);
    }

    #[test]
    fn cache_dir_for_user_nests_under_users_tree() {
        let root = PathBuf::from("/tmp/openhuman");
        assert_eq!(
            cache_dir_for_user(&root, "u-123").unwrap(),
            PathBuf::from("/tmp/openhuman/users/u-123/cef")
        );
    }

    #[test]
    fn validate_user_id_rejects_path_traversal() {
        assert!(validate_user_id_for_path("..").is_err());
        assert!(validate_user_id_for_path("a/../b").is_err());
        assert!(validate_user_id_for_path("x/y").is_err());
    }

    #[test]
    fn validate_user_id_accepts_typical_ids() {
        assert_eq!(validate_user_id_for_path("u-123").unwrap(), "u-123");
        assert_eq!(
            validate_user_id_for_path("user@ex.com").unwrap(),
            "user@ex.com"
        );
    }

    /// `default_openhuman_dir` must have a parent (sibling marker uses `parent()`).
    fn test_data_hierarchy() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let data_root = tmp.path().join("oh_data");
        std::fs::create_dir_all(&data_root).unwrap();
        (tmp, data_root)
    }

    #[test]
    fn legacy_purge_marker_migrates_to_sibling_file() {
        let (_tmp, data_root) = test_data_hierarchy();
        let legacy = data_root.join(LEGACY_PENDING_PURGE_IN_TREE);
        let sibling = data_root.parent().unwrap().join(PENDING_PURGE_STATE_FILE);
        let body = r#"paths = []"#;
        std::fs::write(&legacy, body).unwrap();
        assert!(!sibling.exists());

        let _ = load_pending_purge_state(&data_root).unwrap();

        assert!(!legacy.exists());
        assert!(sibling.exists());
    }

    #[test]
    fn drain_removes_only_trusted_paths_and_clears_marker() {
        let (_tmp, data_root) = test_data_hierarchy();
        let cef = data_root.join("users").join("u1").join("cef");
        std::fs::create_dir_all(&cef).unwrap();
        std::fs::write(cef.join("x.txt"), b"x").unwrap();
        let cef_s = cef.to_string_lossy().to_string();

        let state = PendingCefPurgeState { paths: vec![cef_s] };
        save_pending_purge_state(&data_root, &state).unwrap();

        drain_pending_purges(&data_root).unwrap();

        assert!(!cef.exists());
        let marker = pending_purge_marker_path(&data_root).unwrap();
        assert!(!marker.exists());
    }

    #[test]
    fn drain_retains_malicious_queue_path_without_deleting() {
        let (tmp, data_root) = test_data_hierarchy();
        let outside = tmp.path().join("outside_sandbox");
        std::fs::create_dir_all(&outside).unwrap();
        let outside_s = outside.to_string_lossy().to_string();
        let state = PendingCefPurgeState {
            paths: vec![outside_s.clone()],
        };
        save_pending_purge_state(&data_root, &state).unwrap();

        drain_pending_purges(&data_root).unwrap();

        assert!(outside.exists());
        let rest = load_pending_purge_state(&data_root).unwrap();
        assert_eq!(rest.paths, vec![outside_s]);
        let marker = pending_purge_marker_path(&data_root).unwrap();
        assert!(marker.exists());
    }

    /// Path is under `users/…` but last component is not `cef` (reject, retain in queue).
    #[test]
    fn drain_does_not_remove_path_without_cef_final_segment() {
        let (_tmp, data_root) = test_data_hierarchy();
        let d = data_root.join("users").join("u1").join("data");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("f"), b"1").unwrap();
        save_pending_purge_state(
            &data_root,
            &PendingCefPurgeState {
                paths: vec![d.to_string_lossy().to_string()],
            },
        )
        .unwrap();

        drain_pending_purges(&data_root).unwrap();

        assert!(d.exists());
        let after = load_pending_purge_state(&data_root).unwrap();
        assert_eq!(after.paths.len(), 1);
    }
}
