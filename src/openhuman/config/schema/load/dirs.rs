use super::env::{EnvLookup, ProcessEnv};
use anyhow::{Context, Result};
use directories::UserDirs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;

pub use load_user_state::{
    active_user_marker_path, clear_active_user, pre_login_user_dir, read_active_user_id,
    user_openhuman_dir, write_active_user_id, PRE_LOGIN_USER_ID,
};

#[path = "../load_user_state.rs"]
mod load_user_state;
#[cfg(test)]
pub(crate) use load_user_state::ACTIVE_USER_STATE_FILE;

const ACTIVE_WORKSPACE_STATE_FILE: &str = "active_workspace.toml";

#[derive(Debug, Serialize, Deserialize)]
struct ActiveWorkspaceState {
    config_dir: String,
}

/// Environment override for the agent's default projects directory.
pub const PROJECTS_DIR_ENV_VAR: &str = "OPENHUMAN_PROJECTS_DIR";

/// Environment override for the agent action sandbox directory.
pub const ACTION_DIR_ENV_VAR: &str = "OPENHUMAN_ACTION_DIR";

/// Environment override for the global memory-sync cadence (seconds).
/// `0` means "Manual only". See issue #3302 and
/// [`Config::memory_sync_interval_secs`].
pub const MEMORY_SYNC_INTERVAL_SECS_ENV_VAR: &str = "OPENHUMAN_MEMORY_SYNC_INTERVAL_SECS";

fn default_root_dir_name() -> &'static str {
    if crate::api::config::is_staging_app_env(crate::api::config::app_env_from_env().as_deref()) {
        ".openhuman-staging"
    } else {
        ".openhuman"
    }
}

#[cfg(test)]
pub(crate) fn default_root_dir_name_pub() -> &'static str {
    default_root_dir_name()
}

/// Returns the root openhuman directory (`~/.openhuman`), independent of any
/// per-user scoping.  Used to locate `active_user.toml` and the shared
/// `users/` tree.
pub fn default_root_openhuman_dir() -> Result<PathBuf> {
    let home = UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .context("Could not find home directory")?;
    Ok(home.join(default_root_dir_name()))
}

pub(super) fn default_config_dir() -> Result<PathBuf> {
    default_root_openhuman_dir()
}

pub(super) fn default_config_and_workspace_dirs() -> Result<(PathBuf, PathBuf)> {
    let config_dir = default_config_dir()?;
    Ok((config_dir.clone(), config_dir.join("workspace")))
}

/// The agent's default **projects home** — a visible, read-write directory
/// (`~/OpenHuman/projects`) where the coding agent creates and saves projects,
/// kept distinct from the hidden internal state dir (`~/.openhuman/workspace`,
/// which also holds `memory_tree` etc.). Overridable via `OPENHUMAN_PROJECTS_DIR`;
/// falls back to `./OpenHuman/projects` only when the home dir can't be resolved.
pub fn default_projects_dir() -> PathBuf {
    if let Ok(p) = std::env::var(PROJECTS_DIR_ENV_VAR) {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("OpenHuman")
        .join("projects")
}

/// The `OPENHUMAN_ACTION_DIR` env override, when set to a non-empty value.
///
/// Returns `None` when the variable is unset or blank (a common shape from
/// shells that pass through a declared-but-unset variable). The trim mirrors
/// [`default_action_dir`] so an empty env var never pins `action_dir`.
pub fn action_dir_env_override() -> Option<PathBuf> {
    let raw = std::env::var(ACTION_DIR_ENV_VAR).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// Resolve the effective `action_dir` from the precedence chain:
/// env `OPENHUMAN_ACTION_DIR` > persisted `action_dir_override` > default
/// projects dir. Keeping the env var first means existing env-driven
/// deployments are unaffected by a UI-set override.
pub fn resolve_action_dir(action_dir_override: &Option<PathBuf>) -> PathBuf {
    if let Some(env_dir) = action_dir_env_override() {
        return env_dir;
    }
    if let Some(over) = action_dir_override {
        if !over.as_os_str().is_empty() && over.is_absolute() {
            return over.clone();
        }
        tracing::warn!(
            value = %over.display(),
            "[config] ignoring invalid action_dir_override; expected non-empty absolute path"
        );
    }
    default_projects_dir()
}

pub fn default_action_dir() -> PathBuf {
    if let Ok(p) = std::env::var(ACTION_DIR_ENV_VAR) {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    default_projects_dir()
}

fn active_workspace_state_path(default_dir: &Path) -> PathBuf {
    default_dir.join(ACTIVE_WORKSPACE_STATE_FILE)
}

async fn load_persisted_workspace_dirs(
    default_config_dir: &Path,
) -> Result<Option<(PathBuf, PathBuf)>> {
    let state_path = active_workspace_state_path(default_config_dir);
    if !state_path.exists() {
        return Ok(None);
    }

    let contents = match fs::read_to_string(&state_path).await {
        Ok(contents) => contents,
        Err(error) => {
            tracing::warn!(
                "Failed to read active workspace marker {}: {error}",
                state_path.display()
            );
            return Ok(None);
        }
    };

    let state: ActiveWorkspaceState = match toml::from_str(&contents) {
        Ok(state) => state,
        Err(error) => {
            tracing::warn!(
                "Failed to parse active workspace marker {}: {error}",
                state_path.display()
            );
            return Ok(None);
        }
    };

    let raw_config_dir = state.config_dir.trim();
    if raw_config_dir.is_empty() {
        tracing::warn!(
            "Ignoring active workspace marker {} because config_dir is empty",
            state_path.display()
        );
        return Ok(None);
    }

    let parsed_dir = PathBuf::from(raw_config_dir);
    let config_dir = if parsed_dir.is_absolute() {
        parsed_dir
    } else {
        default_config_dir.join(parsed_dir)
    };
    Ok(Some((config_dir.clone(), config_dir.join("workspace"))))
}

pub(crate) async fn persist_active_workspace_config_dir(config_dir: &Path) -> Result<()> {
    let default_config_dir = default_config_dir()?;
    let state_path = active_workspace_state_path(&default_config_dir);

    if config_dir == default_config_dir {
        if state_path.exists() {
            fs::remove_file(&state_path).await.with_context(|| {
                format!(
                    "Failed to clear active workspace marker: {}",
                    state_path.display()
                )
            })?;
        }
        return Ok(());
    }

    fs::create_dir_all(&default_config_dir)
        .await
        .with_context(|| {
            format!(
                "Failed to create default config directory: {}",
                default_config_dir.display()
            )
        })?;

    let state = ActiveWorkspaceState {
        config_dir: config_dir.to_string_lossy().into_owned(),
    };
    let serialized =
        toml::to_string_pretty(&state).context("Failed to serialize active workspace marker")?;

    let temp_path = default_config_dir.join(format!(
        ".{ACTIVE_WORKSPACE_STATE_FILE}.tmp-{}",
        uuid::Uuid::new_v4()
    ));
    fs::write(&temp_path, serialized).await.with_context(|| {
        format!(
            "Failed to write temporary active workspace marker: {}",
            temp_path.display()
        )
    })?;

    if let Err(error) = fs::rename(&temp_path, &state_path).await {
        let _ = fs::remove_file(&temp_path).await;
        anyhow::bail!(
            "Failed to atomically persist active workspace marker {}: {error}",
            state_path.display()
        );
    }

    super::sync_directory(&default_config_dir).await?;
    Ok(())
}

pub(crate) fn resolve_config_dir_for_workspace(workspace_dir: &Path) -> (PathBuf, PathBuf) {
    let workspace_config_dir = workspace_dir.to_path_buf();
    if workspace_config_dir.join("config.toml").exists() {
        return (
            workspace_config_dir.clone(),
            workspace_config_dir.join("workspace"),
        );
    }

    let legacy_config_dir = workspace_dir
        .parent()
        .map(|parent| parent.join(".openhuman"));
    if let Some(legacy_dir) = legacy_config_dir {
        if legacy_dir.join("config.toml").exists() {
            return (legacy_dir, workspace_config_dir);
        }

        if workspace_dir
            .file_name()
            .is_some_and(|name| name == std::ffi::OsStr::new("workspace"))
        {
            return (legacy_dir, workspace_config_dir);
        }
    }

    (
        workspace_config_dir.clone(),
        workspace_config_dir.join("workspace"),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConfigResolutionSource {
    EnvWorkspace,
    ActiveWorkspaceMarker,
    ActiveUser,
    DefaultConfigDir,
}

impl ConfigResolutionSource {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::EnvWorkspace => "OPENHUMAN_WORKSPACE",
            Self::ActiveWorkspaceMarker => "active_workspace.toml",
            Self::ActiveUser => "active_user.toml",
            Self::DefaultConfigDir => "default",
        }
    }
}

pub(crate) async fn resolve_runtime_config_dirs(
    default_openhuman_dir: &Path,
    default_workspace_dir: &Path,
) -> Result<(PathBuf, PathBuf, ConfigResolutionSource)> {
    resolve_runtime_config_dirs_with(default_openhuman_dir, default_workspace_dir, &ProcessEnv)
        .await
}

/// Env-injectable variant of [`resolve_runtime_config_dirs`]. Accepts any
/// [`EnvLookup`] so unit tests can exercise the `OPENHUMAN_WORKSPACE`
/// override path without mutating the process environment.
pub(crate) async fn resolve_runtime_config_dirs_with(
    default_openhuman_dir: &Path,
    default_workspace_dir: &Path,
    env: &(dyn EnvLookup + Send + Sync),
) -> Result<(PathBuf, PathBuf, ConfigResolutionSource)> {
    if let Some(custom_workspace) = env.get("OPENHUMAN_WORKSPACE") {
        if !custom_workspace.is_empty() {
            let (openhuman_dir, workspace_dir) =
                resolve_config_dir_for_workspace(&PathBuf::from(custom_workspace));
            return Ok((
                openhuman_dir,
                workspace_dir,
                ConfigResolutionSource::EnvWorkspace,
            ));
        }
    }

    resolve_config_dirs_ignoring_env(default_openhuman_dir, default_workspace_dir).await
}

/// Same as [`resolve_runtime_config_dirs`] but skips the
/// `OPENHUMAN_WORKSPACE` env var override. Used by
/// [`Config::load_from_default_paths`] so callers can reliably load
/// the real user config without mutating the process environment.
pub(super) async fn resolve_config_dirs_ignoring_env(
    default_openhuman_dir: &Path,
    default_workspace_dir: &Path,
) -> Result<(PathBuf, PathBuf, ConfigResolutionSource)> {
    if let Some(user_id) = read_active_user_id(default_openhuman_dir) {
        let user_dir = user_openhuman_dir(default_openhuman_dir, &user_id);
        let user_workspace = user_dir.join("workspace");
        tracing::debug!(
            user_id = %user_id,
            user_dir = %user_dir.display(),
            "Config dirs resolved via active_user.toml"
        );
        return Ok((user_dir, user_workspace, ConfigResolutionSource::ActiveUser));
    }

    if let Some((openhuman_dir, workspace_dir)) =
        load_persisted_workspace_dirs(default_openhuman_dir).await?
    {
        return Ok((
            openhuman_dir,
            workspace_dir,
            ConfigResolutionSource::ActiveWorkspaceMarker,
        ));
    }

    let user_dir = pre_login_user_dir(default_openhuman_dir);
    let user_workspace = user_dir.join("workspace");
    tracing::debug!(
        user_id = %PRE_LOGIN_USER_ID,
        user_dir = %user_dir.display(),
        default_workspace_dir = %default_workspace_dir.display(),
        "Config dirs resolved to pre-login user directory (no active user, no workspace marker)"
    );
    Ok((
        user_dir,
        user_workspace,
        ConfigResolutionSource::DefaultConfigDir,
    ))
}
