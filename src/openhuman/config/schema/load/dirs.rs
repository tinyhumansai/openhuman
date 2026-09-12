use super::env::{EnvLookup, ProcessEnv};
use anyhow::{Context, Result};
use directories::UserDirs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;

pub use load_user_state::{
    active_user_marker_path, clear_active_user, pre_login_user_dir, read_active_user_id,
    read_active_user_id_checked_async, user_openhuman_dir, write_active_user_id, PRE_LOGIN_USER_ID,
};
// Sync variant is consumed only by the `read_active_user_id` wrapper (in
// load_user_state) and the load_tests module; the async resolver below uses
// `read_active_user_id_checked_async`.
#[cfg(test)]
pub use load_user_state::read_active_user_id_checked;

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

    // Before the write, not after: this marker is one of the inputs the
    // resolver reads, so from here until the next resolve the cached answer
    // is no longer known to be current — including if the write below fails
    // partway. Clearing early costs one resolve; clearing late leaves a
    // window in which a stale workspace reads as active.
    super::active_workspace::invalidate_active_workspace();

    if config_dir == default_config_dir {
        if state_path.exists() {
            fs::remove_file(&state_path).await.with_context(|| {
                format!(
                    "Failed to clear active workspace marker: {}",
                    state_path.display()
                )
            })?;
            // Again, now that the marker is actually gone — same race as the
            // write branch below.
            super::active_workspace::invalidate_active_workspace();
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

    // Again, now that the marker on disk actually says the new workspace. The
    // pre-write clear alone leaves a window in which a racing resolver reads
    // the *old* marker and refills the cache with the workspace being switched
    // away from. Before the directory sync, not after: the rename is what
    // changed the answer, and a sync failure must not skip this.
    super::active_workspace::invalidate_active_workspace();
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

    let has_workspace_basename = workspace_dir
        .file_name()
        .is_some_and(|name| name == std::ffi::OsStr::new("workspace"));
    let parent = workspace_dir.parent();

    // Modern default layout: the workspace lives *inside* the `.openhuman`
    // config dir (`~/.openhuman/workspace`), so the parent IS the config dir.
    // `default_config_and_workspace_dirs` produces exactly this shape. This is
    // a pure path-structure decision — no `.exists()` probe — so pointing
    // `OPENHUMAN_WORKSPACE` at `~/.openhuman/workspace` resolves to
    // `~/.openhuman` and loads the real `~/.openhuman/config.toml` instead of
    // the doubled, non-existent `~/.openhuman/.openhuman` (#6079).
    if has_workspace_basename {
        if let Some(parent) = parent {
            if parent
                .file_name()
                .is_some_and(|name| name == std::ffi::OsStr::new(default_root_dir_name()))
            {
                return (parent.to_path_buf(), workspace_config_dir);
            }
        }
    }

    let legacy_config_dir = parent.map(|parent| parent.join(".openhuman"));
    if let Some(legacy_dir) = legacy_config_dir {
        // Legacy sibling layout: `<proj>/workspace` with config living in a
        // sibling `<proj>/.openhuman`. Return the sibling unconditionally,
        // including on a fresh volume where `<proj>/.openhuman` does not exist
        // yet — that is where `config::load` writes and reads config for this
        // layout, so nesting the workspace inside itself (the fall-through
        // below) would strand it.
        //
        // This arm can no longer reintroduce the #6079 doubling: the modern
        // layout above already intercepts `~/.openhuman/workspace` (parent IS
        // the config dir) before control reaches here, so a `workspace`
        // basename whose parent is the config dir never falls into this arm.
        if legacy_dir.join("config.toml").exists() || has_workspace_basename {
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

/// The workspace this process is serving *right now*.
///
/// Resolved through the same path [`Config::load_or_init`] takes — the
/// `OPENHUMAN_WORKSPACE` override, then `active_user.toml`, then the
/// workspace marker, then the pre-login directory — so a caller cannot
/// disagree with the loader about which workspace is active.
///
/// The answer is never pinned at boot. One process serves more than one
/// workspace over its life and the switch is a change to an on-disk marker,
/// so anything a subscriber captured once goes stale exactly when it
/// matters. The cost is one small marker read, which is why callers should
/// reach for this per *decision*, not per event: `desktop::notifications`
/// asks only for the handful of workspace-bound events a supervisor tick
/// produces, and returns before calling it for everything else.
///
/// A caller that cannot afford even that — the Event Log stamps every domain
/// event the process publishes, from a synchronous stream closure — reads
/// [`active_workspace_dir_cached`](super::active_workspace::active_workspace_dir_cached)
/// instead. Both arms below refill that cache, so the two cannot disagree
/// about a workspace either of them has seen, and the embedder arm is
/// included deliberately: an embedding host's chosen workspace is the
/// authoritative answer for its process too.
///
/// The revision is the one the resolved workspace is current under, taken
/// from the same lock acquisition that committed it. Anything sending the
/// pair to a client — the connect-time seed, the notification stamp — must
/// take it from here rather than resolving the workspace and reading the
/// revision separately: a switch between those two reads pairs workspace A
/// with B's revision, and a receiver comparing revisions then ranks the stale
/// A above the B it should yield to.
pub async fn active_workspace_snapshot() -> Result<(PathBuf, u64)> {
    // An embedding host that supplied its own `Config` is authoritative, and
    // `config::ops::load_config_with_timeout` already short-circuits on it for
    // exactly this reason. Resolving from disk/env here instead would answer
    // with the process-global workspace, which for an embedder is a directory
    // it never chose — and the one caller of this function compares the answer
    // against an event's `workspace_dir` to decide whether to announce. A
    // mismatch there is not a wrong banner, it is *no* banner, permanently,
    // with only a `debug!` line to say so. See AGENTS.md, "CoreBuilder::config
    // alone configures boot and nothing else".
    if let Some(config) = crate::core::runtime::context::CoreContext::current_embedder_config() {
        let revision = super::active_workspace::publish_active_workspace(&config.workspace_dir);
        return Ok((config.workspace_dir, revision));
    }
    let (default_openhuman_dir, default_workspace_dir) = default_config_and_workspace_dirs()?;
    let (_, workspace_dir, source) =
        resolve_runtime_config_dirs(&default_openhuman_dir, &default_workspace_dir).await?;
    tracing::trace!(
        source = source.as_str(),
        "active workspace resolved for a workspace-bound decision"
    );
    let revision = super::active_workspace::publish_active_workspace(&workspace_dir);
    Ok((workspace_dir, revision))
}

/// [`active_workspace_snapshot`] without the revision, for callers that only
/// need to know which workspace is active.
pub async fn active_workspace_dir() -> Result<PathBuf> {
    active_workspace_snapshot().await.map(|(dir, _)| dir)
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
            // A misresolved config dir (e.g. `OPENHUMAN_WORKSPACE` pointing
            // inside `.openhuman`, #6079) silently reverts every setting to
            // schema defaults, and the resolved workspace_dir stays correct so
            // the mistake is otherwise invisible. Name the chosen config dir and
            // whether it holds a `config.toml`: warn when it does not (the
            // fall-to-defaults case), debug on the happy path.
            let config_found = openhuman_dir.join("config.toml").exists();
            if config_found {
                tracing::debug!(
                    config_dir = %openhuman_dir.display(),
                    workspace_dir = %workspace_dir.display(),
                    "OPENHUMAN_WORKSPACE resolved config dir; config.toml present"
                );
            } else {
                tracing::warn!(
                    config_dir = %openhuman_dir.display(),
                    workspace_dir = %workspace_dir.display(),
                    "OPENHUMAN_WORKSPACE resolved to a config dir with no config.toml; \
                     settings will fall back to schema defaults (see #6079)"
                );
            }
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
    // `read_active_user_id_checked_async` (not the lossy `read_active_user_id`):
    // a transient read fault on an *existing* marker propagates as an error here
    // instead of masquerading as "no active user". Falling through to the
    // pre-login directory in that case boots a signed-in user into a fresh,
    // empty `users/local` profile and orphans their real data under
    // `users/<id>` — the "app reset itself" symptom (#5334). Failing the boot
    // loudly (and retrying on the next launch, once the file lock clears) keeps
    // the data intact. The async variant is used so the transient-lock backoff
    // does not block a tokio worker with `std::thread::sleep`.
    if let Some(user_id) = read_active_user_id_checked_async(default_openhuman_dir).await? {
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
