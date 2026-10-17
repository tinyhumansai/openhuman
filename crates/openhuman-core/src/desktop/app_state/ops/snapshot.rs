//! The `app_state_snapshot` and `update_local_state` RPC handlers: the
//! polled entry point that assembles auth, runtime and local on-disk state
//! into one response for the frontend.
//!
//! The core reports the credential it holds and the user payload the host
//! handed over with `auth.set_credential` (the host's own `/auth/me`
//! answer). The *live* current user is the host's business — the desktop
//! shell's `openhuman_tinyhumans::session` cache — so this snapshot never talks to the
//! backend.

use super::runtime_snapshot::{
    build_runtime_snapshot, degraded_runtime_snapshot, RUNTIME_SNAPSHOT_TIMEOUT,
};
use super::state_file::{
    load_stored_app_state, load_stored_app_state_unlocked, save_stored_app_state_unlocked,
    APP_STATE_FILE_LOCK,
};
use super::types::{AppStateSnapshot, StoredAppState, StoredAppStatePatch};
use super::LOG_PREFIX;
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;
use crate::security::credentials::session_support::{
    load_app_session_profile, session_state_from_profile,
};
use log::{debug, warn};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Correlates a snapshot call's own timing/debug log lines across the poll.
pub(super) static SNAPSHOT_REQ_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) fn sanitize_snapshot_user(user: Option<Value>) -> Option<Value> {
    match user {
        Some(Value::Object(map)) if map.is_empty() => None,
        Some(Value::Null) => None,
        other => other,
    }
}

pub async fn snapshot(ctx: &DispatchCtx) -> Result<Outcome<AppStateSnapshot>, String> {
    let req_id = SNAPSHOT_REQ_COUNTER.fetch_add(1, Ordering::Relaxed);
    let t_total = Instant::now();

    let t_config = Instant::now();
    let config = ctx.load_config().await?;
    // Latch corruption recovery from *this* poll's load, not only from boot.
    // `load_config_with_timeout` re-reads config.toml on every snapshot, so a
    // config that becomes corrupt after boot is healed here — carrying a fresh
    // `recovered_from_corruption`. Without this, that mid-session recovery would
    // be dropped (the boot latch never saw it) and the notice never surfaces
    // (#5167). No-op when the load was clean; idempotent once latched.
    crate::desktop::app_state::latch_from_config(&config);
    let config_ms = t_config.elapsed().as_millis();

    let t_auth = Instant::now();
    // Load the `app-session` auth profile exactly once and derive both the
    // session-state view and the raw token from it — one auth-profile file
    // lock per snapshot (the "Timed out waiting for auth profile lock" Sentry
    // failure on Windows came from taking it twice).
    //
    // `load_app_session_profile` busy-waits with `thread::sleep` for up to
    // ~35s when the lock is contended, so it runs on the blocking pool rather
    // than a tokio worker thread.
    let config_for_profile = config.clone();
    let session_profile =
        tokio::task::spawn_blocking(move || load_app_session_profile(&config_for_profile))
            .await
            .unwrap_or_else(|e| Err(format!("[app_state] auth profile load task panicked: {e}")))?;
    let mut auth = session_state_from_profile(session_profile.as_ref());
    let session_token = crate::security::credentials::session_support::session_token_for_config(
        &config,
        session_profile.as_ref(),
    )
    .ok()
    .flatten();
    let current_user = sanitize_snapshot_user(auth.user.clone());
    auth.user = current_user.clone();
    let auth_ms = t_auth.elapsed().as_millis();

    let t_runtime = Instant::now();
    let runtime = match tokio::time::timeout(
        RUNTIME_SNAPSHOT_TIMEOUT,
        build_runtime_snapshot(&config, req_id),
    )
    .await
    {
        Ok(snapshot) => snapshot,
        Err(_) => {
            warn!(
                "{LOG_PREFIX} build_runtime_snapshot timed out after {}s req_id={}; returning degraded runtime snapshot",
                RUNTIME_SNAPSHOT_TIMEOUT.as_secs(),
                req_id
            );
            degraded_runtime_snapshot(&config)
        }
    };
    let runtime_ms = t_runtime.elapsed().as_millis();

    let t_local_state = Instant::now();
    let local_state = load_stored_app_state(&config)?;
    crate::security::keyring_consent::policy::initialize(local_state.keyring_consent.clone());
    let local_state_ms = t_local_state.elapsed().as_millis();

    let total_ms = t_total.elapsed().as_millis();
    debug!(
        "{LOG_PREFIX} snapshot timings req_id={} config_ms={} auth_ms={} runtime_ms={} local_state_ms={} total_ms={}",
        req_id, config_ms, auth_ms, runtime_ms, local_state_ms, total_ms
    );

    debug!(
        "{LOG_PREFIX} snapshot req_id={} auth={} onboarding={} chat_onboarding={} analytics={} local_ai_state={} service_state={:?}",
        req_id,
        auth.is_authenticated,
        config.onboarding_completed,
        config.chat_onboarding_completed,
        config.observability.analytics_enabled,
        runtime.local_ai.state,
        runtime.service.state
    );

    let keyring_status = crate::security::keyring_consent::policy::current_status();
    let health = crate::platform::health::snapshot();

    Ok(Outcome::new(
        AppStateSnapshot {
            auth,
            session_token,
            current_user,
            onboarding_completed: config.onboarding_completed,
            chat_onboarding_completed: config.chat_onboarding_completed,
            analytics_enabled: config.observability.analytics_enabled,
            local_state,
            keyring_status,
            runtime,
            health,
            config_recovered: crate::desktop::app_state::config_recovered_this_session(),
        },
        vec!["core app state snapshot fetched".to_string()],
    ))
}

pub async fn update_local_state(
    ctx: &DispatchCtx,
    patch: StoredAppStatePatch,
) -> Result<Outcome<StoredAppState>, String> {
    let config = ctx.load_config().await?;
    let _guard = APP_STATE_FILE_LOCK.lock();
    let mut current = load_stored_app_state_unlocked(&config)?;

    if let Some(encryption_key) = patch.encryption_key {
        current.encryption_key = encryption_key.and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        });
    }

    if let Some(onboarding_tasks) = patch.onboarding_tasks {
        current.onboarding_tasks = onboarding_tasks;
    }

    if let Some(keyring_consent) = patch.keyring_consent {
        current.keyring_consent = keyring_consent;
    }

    save_stored_app_state_unlocked(&config, &current)?;

    debug!(
        "{LOG_PREFIX} local state updated encryption_key={} onboarding_tasks={} keyring_consent={}",
        current.encryption_key.is_some(),
        current.onboarding_tasks.is_some(),
        current.keyring_consent.is_some(),
    );

    Ok(Outcome::new(
        current,
        vec!["core local app state updated".to_string()],
    ))
}
