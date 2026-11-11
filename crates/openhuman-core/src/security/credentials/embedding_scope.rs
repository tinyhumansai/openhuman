use std::path::{Path, PathBuf};

use crate::config::Config;

/// Resolve the managed embedder's credential directory and encryption policy.
/// Callers with a config use its own state directory and encryption setting;
/// callers without one use the process's active user scope and default to
/// encrypted credentials.
pub(crate) fn credential_scope(config: Option<&Config>) -> (PathBuf, bool) {
    match config {
        Some(config) => (super::state_dir_from_config(config), config.secrets.encrypt),
        None => (default_state_dir(), true),
    }
}

/// Resolve the process credential scope for call sites without a `Config`.
/// Workspace overrides follow the same config-dir mapping as config loading;
/// otherwise credentials live below the active user (or the pre-login user).
fn default_state_dir() -> PathBuf {
    log::debug!("[credentials] default managed credential scope: resolving");
    if let Some(workspace) = std::env::var_os("OPENHUMAN_WORKSPACE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        log::debug!(
            "[credentials] default managed credential scope uses OPENHUMAN_WORKSPACE-derived config dir"
        );
        return env_workspace_state_dir(&workspace);
    }

    let root = crate::config::default_root_openhuman_dir().unwrap_or_else(|error| {
        log::warn!(
            "[credentials] could not resolve the openhuman root dir ({error}); \
             falling back to a relative .openhuman path"
        );
        PathBuf::from(".openhuman")
    });

    let user_id = crate::config::read_active_user_id(&root);
    log::debug!(
        "[credentials] default managed credential scope resolved to user-scoped dir (active_user_present={})",
        user_id.is_some()
    );
    user_scoped_state_dir(&root, user_id.as_deref())
}

/// Resolve a workspace override through the same workspace→config-dir mapping
/// as config loading. In the legacy layout this is the sibling `.openhuman`
/// directory containing `auth-profiles.json`, rather than the workspace itself.
fn env_workspace_state_dir(workspace: &Path) -> PathBuf {
    let (config_dir, _workspace_dir) = crate::config::resolve_config_dir_for_workspace(workspace);
    config_dir
}

/// Resolve the active user's credential directory, using the pre-login user
/// when no account is active.
fn user_scoped_state_dir(root: &Path, active_user_id: Option<&str>) -> PathBuf {
    crate::config::user_openhuman_dir(
        root,
        active_user_id.unwrap_or(crate::config::PRE_LOGIN_USER_ID),
    )
}

#[cfg(test)]
#[path = "embedding_scope_tests.rs"]
mod tests;
