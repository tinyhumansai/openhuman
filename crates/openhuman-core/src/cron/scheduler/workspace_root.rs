//! The directory a scheduled agent run is rooted at.
//!
//! An agent definition may name one (`AgentExt::scheduled_workspace`). The run
//! then carries it as its workspace descriptor, so the shell and file tools
//! work there and a sandboxed shell asks TinySecurity for that root read-write
//! instead of the process-wide `action_dir`. A deployment whose action dir
//! holds the core workspace (a checkout with the brain inside it) can only run
//! a sandboxed shell from cron this way: TinySecurity refuses any grant that
//! overlaps the core workspace.

use std::path::{Path, PathBuf};

use crate::agent_registry::host_definition::AgentDefinition;
use crate::config::Config;

/// The root a cron run of `definition` works in, or `None` for the
/// configured `action_dir`.
///
/// # Errors
/// A message naming the agent and the reason when the configured root does
/// not exist, is not a directory, overlaps the core workspace or holds the
/// core's credentials. The run must not start rooted anywhere else.
pub(super) fn scheduled_workspace(
    config: &Config,
    definition: &AgentDefinition,
) -> Result<Option<PathBuf>, String> {
    let Some(raw) = definition.extensions.scheduled_workspace.as_deref() else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let agent = &definition.id;
    let named = Path::new(raw);
    let path = if named.is_absolute() {
        named.to_path_buf()
    } else {
        config.action_dir.join(named)
    };
    let root = std::fs::canonicalize(&path).map_err(|error| {
        format!("agent `{agent}`: scheduled_workspace '{raw}' is not accessible: {error}")
    })?;
    if !root.is_dir() {
        return Err(format!(
            "agent `{agent}`: scheduled_workspace '{raw}' is not a directory"
        ));
    }
    let workspace = canonical_or_original(&config.workspace_dir);
    if root.starts_with(&workspace) || workspace.starts_with(&root) {
        return Err(format!(
            "agent `{agent}`: scheduled_workspace {} overlaps the core workspace {}; \
             a scheduled run must be rooted beside it, never over the core's own state",
            root.display(),
            workspace.display()
        ));
    }
    if let Some(store) = credential_stores(config)
        .into_iter()
        .find(|store| store.starts_with(&root))
    {
        return Err(format!(
            "agent `{agent}`: scheduled_workspace {} holds the core credential store {}",
            root.display(),
            store.display()
        ));
    }
    tracing::debug!(
        agent_id = %agent,
        root = %root.display(),
        "[cron] scheduled run rooted at the agent's scheduled_workspace"
    );
    Ok(Some(root))
}

/// Every file holding the core's credentials, resolved where it exists: the
/// stores the sandbox already denies to a jail, plus the operator's keyring
/// master key file when one is configured.
/// `config` scoped to a scheduled run's `root`: stub for the red step.
pub(super) fn rooted_config(config: &Config, root: &Path) -> Config {
    let _ = root;
    config.clone()
}

fn credential_stores_with(config: &Config, master_key: Option<std::ffi::OsString>) -> Vec<PathBuf> {
    let _ = master_key;
    credential_stores(config)
}

fn credential_stores(config: &Config) -> Vec<PathBuf> {
    let mut stores = crate::credentials::credential_files(config);
    if let Some(master_key) = std::env::var_os(
        crate::security::secret_storage::encrypted_file_backend::MASTER_KEY_FILE_ENV,
    )
    .filter(|value| !value.is_empty())
    {
        stores.push(PathBuf::from(master_key));
    }
    stores
        .iter()
        .map(|store| canonical_or_original(store))
        .collect()
}

fn canonical_or_original(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
#[path = "workspace_root_tests.rs"]
mod tests;
