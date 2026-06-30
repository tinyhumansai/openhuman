//! Config load/save and environment variable overrides.

mod dirs;
mod env;
mod env_overlay;
mod impl_load;
mod migrate;
mod secrets;

pub(crate) use env::EnvLookup;
pub(crate) use env::ProcessEnv;

pub use dirs::{
    action_dir_env_override, active_user_marker_path, clear_active_user, default_action_dir,
    default_projects_dir, default_root_openhuman_dir, pre_login_user_dir, read_active_user_id,
    resolve_action_dir, user_openhuman_dir, write_active_user_id, ACTION_DIR_ENV_VAR,
    MEMORY_SYNC_INTERVAL_SECS_ENV_VAR, PRE_LOGIN_USER_ID, PROJECTS_DIR_ENV_VAR,
};

pub(crate) use dirs::persist_active_workspace_config_dir;

// redact_url_for_log is pub(super) for the schema module; tests inside load
// can access it because they are a submodule and use `use super::*`.
pub(super) use migrate::redact_url_for_log;

// Items needed by load_tests.rs (loaded as `mod tests` below).
// Tests are a submodule of `load`, so `super::*` == this module's namespace.
#[cfg(test)]
pub(crate) use dirs::default_root_dir_name_pub as default_root_dir_name;
#[cfg(test)]
pub(crate) use dirs::{
    resolve_config_dir_for_workspace, resolve_runtime_config_dirs,
    resolve_runtime_config_dirs_with, ConfigResolutionSource,
};
// PathBuf and Config were in scope via `use super::*` in the original load.rs.
#[cfg(test)]
pub(crate) use super::Config;
#[cfg(test)]
pub(crate) use dirs::ACTIVE_USER_STATE_FILE;
#[cfg(test)]
pub(crate) use env::ProcessEnvWithoutWorkspace;
#[cfg(test)]
pub(crate) use impl_load::parse_config_with_recovery;
#[cfg(test)]
pub(crate) use migrate::{migrate_cloud_provider_slugs, migrate_legacy_inference_url};
#[cfg(test)]
pub(crate) use std::path::PathBuf;

#[cfg(unix)]
pub(super) async fn sync_directory(path: &std::path::Path) -> anyhow::Result<()> {
    use anyhow::Context;
    use tokio::fs::File;
    let dir = File::open(path)
        .await
        .with_context(|| format!("Failed to open directory for fsync: {}", path.display()))?;
    dir.sync_all()
        .await
        .with_context(|| format!("Failed to fsync directory metadata: {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
pub(super) async fn sync_directory(_path: &std::path::Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
#[path = "../load_tests.rs"]
mod tests;
