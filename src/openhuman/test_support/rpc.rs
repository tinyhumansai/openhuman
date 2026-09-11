//! Implementation of `openhuman.test_reset` — wipes persistent state in-place.
//!
//! The reset deliberately mirrors what the user sees on a fresh install:
//!   - no authenticated user (active_user.toml removed, api_key cleared)
//!   - onboarding not yet completed (onboarding_completed=false, chat_onboarding_completed=false)
//!   - no cron jobs (so the post-onboarding seed re-creates `morning_briefing`)
//!   - no memory-tree chunks, summaries, content dirs, or sync cursors
//!
//! It is intentionally in-process: the sidecar keeps running. Specs reload
//! the webview after this call so the renderer also starts from a blank slate.

use serde::Serialize;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::config::{clear_active_user, default_root_openhuman_dir};
use crate::openhuman::cron;
use crate::openhuman::memory::read_rpc;
use crate::rpc::RpcOutcome;

const E2E_MODE_ENV_VAR: &str = "OPENHUMAN_E2E_MODE";

/// Wipe summary returned to the caller for debug visibility.
#[derive(Debug, Serialize)]
pub struct ResetSummary {
    pub cron_jobs_removed: usize,
    pub memory_tree_rows_deleted: u64,
    pub memory_tree_dirs_removed: Vec<String>,
    pub memory_tree_sync_state_cleared: u64,
    pub onboarding_was_completed: bool,
    pub api_key_was_set: bool,
    pub active_user_cleared: bool,
}

#[derive(Debug, Serialize)]
struct MemoryTreeResetSummary {
    rows_deleted: u64,
    dirs_removed: Vec<String>,
    sync_state_cleared: u64,
}

fn ensure_e2e_mode_enabled() -> Result<(), String> {
    ensure_e2e_mode_value(std::env::var(E2E_MODE_ENV_VAR).ok().as_deref())
}

fn ensure_e2e_mode_value(raw: Option<&str>) -> Result<(), String> {
    match raw.map(str::trim) {
        Some("1" | "true" | "TRUE" | "yes" | "YES") => Ok(()),
        _ => Err(format!(
            "test_reset is disabled unless {E2E_MODE_ENV_VAR} is set to one of: 1, true, TRUE, yes, YES"
        )),
    }
}

/// Reset persistent state to the "fresh install" baseline.
///
/// Errors at any individual wipe step short-circuit and surface back to the
/// caller — partial resets are worse than a clear failure, because they let
/// downstream tests pass on contaminated state.
pub async fn reset() -> Result<RpcOutcome<ResetSummary>, String> {
    log::debug!("[test_reset] entry");
    ensure_e2e_mode_enabled().map_err(|e| {
        log::debug!("[test_reset] rejected: {e}");
        e
    })?;

    let mut config = Config::load_or_init()
        .await
        .map_err(|e| format!("test_reset: failed to load config: {e}"))?;
    log::trace!(
        "[test_reset] config loaded — onboarding_completed={} chat_onboarding_completed={}, api_key_set={}",
        config.onboarding_completed,
        config.chat_onboarding_completed,
        config.api_key.is_some()
    );

    let onboarding_was_completed = config.chat_onboarding_completed || config.onboarding_completed;
    let api_key_was_set = config.api_key.is_some();

    log::debug!("[test_reset] step=wipe_cron start");
    let cron_jobs_removed = cron::clear_all_jobs(&config)
        .map_err(|e| format!("test_reset: cron wipe failed: {e:#}"))?;
    log::debug!("[test_reset] step=wipe_cron ok removed={cron_jobs_removed}");

    log::debug!("[test_reset] step=wipe_memory_tree start");
    let memory_tree = wipe_memory_tree(&config).await?;
    log::debug!(
        "[test_reset] step=wipe_memory_tree ok rows={} dirs={:?} sync_state={}",
        memory_tree.rows_deleted,
        memory_tree.dirs_removed,
        memory_tree.sync_state_cleared
    );

    log::debug!("[test_reset] step=clear_config_fields start");
    config.onboarding_completed = false;
    config.chat_onboarding_completed = false;
    config.api_key = None;
    config
        .save()
        .await
        .map_err(|e| format!("test_reset: failed to save config: {e:#}"))?;
    log::debug!("[test_reset] step=clear_config_fields ok");

    log::debug!("[test_reset] step=clear_active_user start");
    let root = default_root_openhuman_dir()
        .map_err(|e| format!("test_reset: failed to resolve default root dir: {e:#}"))?;
    clear_active_user(&root)
        .map_err(|e| format!("test_reset: failed to clear active user: {e:#}"))?;
    log::debug!(
        "[test_reset] step=clear_active_user ok root={}",
        root.display()
    );

    let memory_tree_log = format!(
        "memory_tree wiped rows={} dirs={:?} sync_state={}",
        memory_tree.rows_deleted, memory_tree.dirs_removed, memory_tree.sync_state_cleared
    );

    let summary = ResetSummary {
        cron_jobs_removed,
        memory_tree_rows_deleted: memory_tree.rows_deleted,
        memory_tree_dirs_removed: memory_tree.dirs_removed,
        memory_tree_sync_state_cleared: memory_tree.sync_state_cleared,
        onboarding_was_completed,
        api_key_was_set,
        active_user_cleared: true,
    };

    log::info!(
        "[test_reset] wiped sidecar state: {}",
        serde_json::to_string(&summary).unwrap_or_default()
    );

    Ok(RpcOutcome::new(
        summary,
        vec![
            format!("removed {cron_jobs_removed} cron jobs"),
            memory_tree_log,
            format!("onboarding_completed + chat_onboarding_completed: {onboarding_was_completed} → false"),
            format!("api_key cleared (was set: {api_key_was_set})"),
            "active_user.toml removed".to_string(),
        ],
    ))
}

async fn wipe_memory_tree(config: &Config) -> Result<MemoryTreeResetSummary, String> {
    let outcome = read_rpc::wipe_all_rpc(config)
        .await
        .map_err(|e| format!("test_reset: memory_tree wipe failed: {e}"))?;
    let value = outcome.value;
    Ok(MemoryTreeResetSummary {
        rows_deleted: value.rows_deleted,
        dirs_removed: value.dirs_removed,
        sync_state_cleared: value.sync_state_cleared,
    })
}

/// Convenience helper for handlers that prefer a raw JSON envelope.
#[allow(dead_code)]
pub async fn reset_json() -> Result<serde_json::Value, String> {
    let outcome = reset().await?;
    Ok(json!({
        "removed_cron_jobs": outcome.value.cron_jobs_removed,
        "memory_tree_rows_deleted": outcome.value.memory_tree_rows_deleted,
        "memory_tree_dirs_removed": outcome.value.memory_tree_dirs_removed,
        "memory_tree_sync_state_cleared": outcome.value.memory_tree_sync_state_cleared,
        "previously_onboarded": outcome.value.onboarding_was_completed,
        "previously_authenticated": outcome.value.api_key_was_set,
    }))
}

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;
