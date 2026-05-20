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

const E2E_MODE_ENV_VAR: &str = "OPENHUMAN_E2E_MODE";

/// Wipe summary returned to the caller for debug visibility.
#[derive(Debug, Serialize)]
pub struct ResetSummary {
    pub cron_jobs_removed: usize,
    pub onboarding_was_completed: bool,
    pub api_key_was_set: bool,
    pub active_user_cleared: bool,
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

#[cfg(test)]
mod tests {
    use super::{ensure_e2e_mode_value, reset, E2E_MODE_ENV_VAR};
    use std::sync::{Mutex, OnceLock};

    static E2E_MODE_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        E2E_MODE_ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[tokio::test]
    async fn reset_rejects_when_e2e_mode_unset() {
        let _guard = env_lock();
        let prior = std::env::var(E2E_MODE_ENV_VAR).ok();
        std::env::remove_var(E2E_MODE_ENV_VAR);

        let err = reset()
            .await
            .expect_err("unset E2E mode must reject test_reset");

        match prior {
            Some(value) => std::env::set_var(E2E_MODE_ENV_VAR, value),
            None => std::env::remove_var(E2E_MODE_ENV_VAR),
        }

        assert!(
            err.contains("OPENHUMAN_E2E_MODE") && err.contains("is set to one of"),
            "unexpected guard error: {err}"
        );
    }

    #[test]
    fn reset_guard_accepts_explicit_e2e_mode() {
        ensure_e2e_mode_value(Some("1")).expect("1 enables E2E mode");
        ensure_e2e_mode_value(Some("true")).expect("true enables E2E mode");
        ensure_e2e_mode_value(Some("yes")).expect("yes enables E2E mode");
    }
}
