//! The per-user directory and the process globals that follow it.
//!
//! A credential names a user; the core keeps that user's state under
//! `~/.openhuman/users/<id>/` and points every process-global store
//! (config, core context / memory binding, conversation persistence, cron
//! seeds) at it. These helpers move those pointers when a credential is
//! installed or removed. They know nothing about *how* the credential was
//! obtained — that is the host's business.

use crate::config::{
    default_root_openhuman_dir, pre_login_user_dir, read_active_user_id, user_openhuman_dir,
    write_active_user_id, Config,
};
use crate::threads::store as conversations;

use super::gated_services::is_embedder_host;

const LOG_PREFIX: &str = "[credentials][user-scope]";

/// Whether this process may touch the operator's global
/// `~/.openhuman/active_user.toml` / `users/` tree. An embedder host (the
/// library `Harness`) keeps session state under its own `config_path` scope
/// and must never change which user the operator's real install believes is
/// active purely by virtue of running a library call.
pub(super) fn operator_user_activation_allowed() -> bool {
    if is_embedder_host() {
        return false;
    }
    let Some(workspace) = std::env::var_os("OPENHUMAN_WORKSPACE").filter(|value| !value.is_empty())
    else {
        return true;
    };
    let Ok(root) = default_root_openhuman_dir() else {
        return false;
    };
    let workspace = std::path::PathBuf::from(workspace);
    let workspace = workspace.canonicalize().unwrap_or(workspace);
    let root = root.canonicalize().unwrap_or(root);
    // An explicit scratch workspace must never switch the operator's real
    // ~/.openhuman active user. Its credential still lives at its own config_path.
    workspace.starts_with(root)
}

#[cfg(test)]
#[path = "user_scope_tests.rs"]
mod tests;

/// Activate `~/.openhuman/users/<user_id>/` as the current user directory.
///
/// On the very first activation (no previous `active_user.toml`) the
/// anonymous pre-login conversation store is purged so a fresh account never
/// inherits demo or scratch threads from the pre-login bucket (#1157).
/// Returns human-readable log lines for the RPC outcome.
pub(super) fn activate_user_scope(user_id: &str) -> Result<Vec<String>, String> {
    let mut logs = Vec::new();
    if !operator_user_activation_allowed() {
        log::debug!("{LOG_PREFIX} embedder host; skipping global user activation for {user_id}");
        return Ok(logs);
    }
    let root_dir = default_root_openhuman_dir().map_err(|error| error.to_string())?;
    // Snapshot before overwriting `active_user.toml` so first activation can be
    // told apart from an in-place account switch.
    let previous_active = read_active_user_id(&root_dir);
    let user_dir = user_openhuman_dir(&root_dir, user_id);
    std::fs::create_dir_all(&user_dir).map_err(|error| error.to_string())?;
    write_active_user_id(&root_dir, user_id).map_err(|error| error.to_string())?;
    logs.push(format!("user directory activated for {user_id}"));
    tracing::info!(user_id = %user_id, user_dir = %user_dir.display(), "{LOG_PREFIX} user-scoped directory activated");

    if previous_active.is_none() {
        // Shares `threads::store`' process-wide mutex with
        // `list_threads` / `purge_threads` on any workspace, so purge and
        // concurrent thread RPC in this process cannot interleave.
        let pre_ws = pre_login_user_dir(&root_dir).join("workspace");
        let pre_ws_log = pre_ws.display().to_string();
        match conversations::purge_threads(pre_ws) {
            Ok(stats) => {
                tracing::info!(
                    pre_login_workspace = %pre_ws_log,
                    threads = stats.thread_count,
                    messages = stats.message_count,
                    "{LOG_PREFIX} purged pre-login conversation threads after first activation"
                );
                logs.push(format!(
                    "purged pre-login conversation history (threads={}, messages={})",
                    stats.thread_count, stats.message_count
                ));
            }
            Err(e) => tracing::debug!(
                error = %e,
                pre_login_workspace = %pre_ws_log,
                "{LOG_PREFIX} pre-login conversation purge skipped (non-fatal)"
            ),
        }
    }
    Ok(logs)
}

/// Clear `active_user.toml` so subsequent config loads fall back to the
/// default (unauthenticated) openhuman directory.
pub(super) fn deactivate_user_scope() -> Result<(), String> {
    if !operator_user_activation_allowed() {
        return Ok(());
    }
    let root_dir = default_root_openhuman_dir().map_err(|error| error.to_string())?;
    crate::config::clear_active_user(&root_dir).map_err(|error| error.to_string())
}

/// The config that reflects the current `active_user.toml` — reloaded after an
/// activation so credentials, keys and workspaces land in the user-scoped
/// location — or `fallback` when the reload fails.
pub(super) async fn reload_config_or(_fallback: &Config) -> Result<Config, String> {
    crate::config::load_config_with_timeout()
        .await
        .map_err(|error| error.to_string())
}

/// Point every process-global store at `config`'s workspace after a
/// credential change: cron seeds (including memory's cron jobs), the core
/// context, conversation persistence, and the process-global cost tracker. Returns log lines for
/// the RPC outcome.
pub(super) fn rebind_after_credential_change(
    config: &Config,
    _reason: &str,
) -> Result<Vec<String>, String> {
    let mut logs = Vec::new();
    crate::cron::seed::prune_retired_jobs(config).map_err(|error| error.to_string())?;
    crate::core::runtime::context::CoreContext::rebind_default_workspace(&config.workspace_dir)
        .map_err(|error| error.to_string())?;
    if let Err(error) = crate::cron::system_jobs::ensure_memory_jobs(config) {
        logs.push(format!("memory cron jobs not seeded: {error}"));
    }
    logs.push(format!(
        "core context bound to workspace {}",
        config.workspace_dir.display()
    ));
    conversations::register_conversation_persistence_subscriber(config.workspace_dir.clone());
    logs.push("conversation persistence bound to active workspace".to_string());
    crate::platform::cost::rebind_global(config.cost.clone(), &config.workspace_dir);
    logs.push("cost tracker bound to active workspace".to_string());
    Ok(logs)
}
