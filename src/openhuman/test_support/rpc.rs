//! Implementation of `openhuman.test_reset` — wipes persistent state in-place.
//!
//! The reset deliberately mirrors what the user sees on a fresh install:
//!   - no authenticated user (active_user.toml removed, api_key cleared)
//!   - onboarding not yet completed (chat_onboarding_completed=false)
//!   - no cron jobs (so the post-onboarding seed re-creates `morning_briefing`)
//!
//! It is intentionally in-process: the sidecar keeps running. Specs reload
//! the webview after this call so the renderer also starts from a blank slate.

use serde::Serialize;
use serde_json::json;

use crate::openhuman::config::Config;
use crate::openhuman::config::{clear_active_user, default_root_openhuman_dir};
use crate::openhuman::cron;
use crate::rpc::RpcOutcome;

/// Wipe summary returned to the caller for debug visibility.
#[derive(Debug, Serialize)]
pub struct ResetSummary {
    pub cron_jobs_removed: usize,
    pub onboarding_was_completed: bool,
    pub api_key_was_set: bool,
    pub active_user_cleared: bool,
}

/// Reset persistent state to the "fresh install" baseline.
///
/// Errors at any individual wipe step short-circuit and surface back to the
/// caller — partial resets are worse than a clear failure, because they let
/// downstream tests pass on contaminated state.
pub async fn reset() -> Result<RpcOutcome<ResetSummary>, String> {
    log::debug!("[test_reset] entry");
    let mut config = Config::load_or_init()
        .await
        .map_err(|e| format!("test_reset: failed to load config: {e}"))?;
    log::trace!(
        "[test_reset] config loaded — onboarding_completed={}, api_key_set={}",
        config.chat_onboarding_completed,
        config.api_key.is_some()
    );

    let onboarding_was_completed = config.chat_onboarding_completed;
    let api_key_was_set = config.api_key.is_some();

    log::debug!("[test_reset] step=wipe_cron start");
    let cron_jobs_removed = cron::clear_all_jobs(&config)
        .map_err(|e| format!("test_reset: cron wipe failed: {e:#}"))?;
    log::debug!("[test_reset] step=wipe_cron ok removed={cron_jobs_removed}");

    log::debug!("[test_reset] step=clear_config_fields start");
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

    let summary = ResetSummary {
        cron_jobs_removed,
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
            format!("chat_onboarding_completed: {onboarding_was_completed} → false"),
            format!("api_key cleared (was set: {api_key_was_set})"),
            "active_user.toml removed".to_string(),
        ],
    ))
}

/// Convenience helper for handlers that prefer a raw JSON envelope.
#[allow(dead_code)]
pub async fn reset_json() -> Result<serde_json::Value, String> {
    let outcome = reset().await?;
    Ok(json!({
        "removed_cron_jobs": outcome.value.cron_jobs_removed,
        "previously_onboarded": outcome.value.onboarding_was_completed,
        "previously_authenticated": outcome.value.api_key_was_set,
    }))
}
