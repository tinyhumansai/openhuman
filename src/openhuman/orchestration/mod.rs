//! Orchestration domain — the device-side client of the **hosted** orchestration
//! brain.
//!
//! The reasoning/wake graph runs server-side (`tinyhumansai/backend`). On the
//! device this domain is a pure trigger + effect-executor + renderer:
//!
//! - [`types`]: the harness `SessionEnvelopeV1` mirror + persisted session/message model.
//! - [`store`]: SQLite render cache at `<workspace>/orchestration/orchestration.db`.
//! - [`ingest`]: decrypt-once → dedupe → persist(cache) → forward to the hosted brain.
//! - [`cloud`]: the hosted uplink (`POST events`/`world-diff`) + read surface (`GET …`).
//! - [`effect_executor`]: runs `send_dm` / `evict` / `tool_call` effects the brain pushes.
//! - [`world_diff_uploader`] / [`world_model`]: device world-observations → subconscious tier.
//! - [`sync`]: hosted reachability + steering cache for the status/offline surface.
//! - [`migrate_history`]: one-shot first-login import of local history to the brain.

pub mod attention;
pub mod bus;
pub mod cloud;
pub mod effect_executor;
pub mod exec_gate;
pub mod ingest;
pub mod migrate_history;
pub mod ops;
pub mod presence;
pub mod schemas;
pub mod store;
pub mod sync;
pub mod tools;
pub mod types;
pub mod wire;
pub mod world_diff_uploader;
pub mod world_model;

pub use bus::{
    notify_orchestration_message, register_orchestration_ingest_subscriber,
    subscribe_orchestration_socket,
};
pub use ops::start_message_drain_supervisor;
pub use schemas::{all_controller_schemas, all_registered_controllers};

// ── Hosted-client background services (login-gated) ──────────────────────────

use std::sync::Mutex;

use tokio::task::JoinHandle;

use crate::openhuman::config::Config;

/// Join handles for the per-login hosted-client loops (read-sync + world-diff
/// uploader), so a logout→login aborts the old session's loops before starting
/// fresh ones — no duplicates, and never a loop bound to a previous session's
/// config/workspace.
static HOSTED_CLIENT_TASKS: Mutex<Vec<JoinHandle<()>>> = Mutex::new(Vec::new());

fn abort_hosted_client_tasks() {
    let mut guard = HOSTED_CLIENT_TASKS
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    for handle in guard.drain(..) {
        handle.abort();
    }
}

/// Start (or restart) the device-side hosted-orchestration client for the active
/// login: the read-sync loop (hosted read surface → render cache + reachability)
/// and the world-diff uploader, plus the one-shot idempotent history migration.
///
/// Called from `credentials::ops::start_login_gated_services`, so it runs on both
/// startup (already logged in) and a fresh login. Idempotent: aborts any loops
/// from a previous session first. No-op (and stops any running loops) when
/// orchestration is disabled.
pub async fn start_hosted_client_services(config: &Config) {
    if !config.orchestration.enabled {
        abort_hosted_client_tasks();
        return;
    }
    {
        let mut guard = HOSTED_CLIENT_TASKS
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        for handle in guard.drain(..) {
            handle.abort();
        }
        let sync_cfg = config.clone();
        guard.push(tokio::spawn(async move {
            sync::run_sync_loop(sync_cfg, sync::DEFAULT_SYNC_INTERVAL).await;
        }));
        let flush_cfg = config.clone();
        guard.push(tokio::spawn(async move {
            world_diff_uploader::run_flush_loop(
                flush_cfg,
                world_diff_uploader::DEFAULT_FLUSH_INTERVAL,
            )
            .await;
        }));
        // Fire-and-forget the one-shot history migration so a slow/offline
        // network can't block login-gated startup. Idempotent; the flag stays
        // unset on failure so it retries next login.
        let migrate_cfg = config.clone();
        guard.push(tokio::spawn(async move {
            migrate_history::migrate_if_needed(&migrate_cfg).await;
        }));
    }
    log::info!(target: "orchestration", "[orchestration] hosted-client services started");
}

/// Stop the hosted-client loops (logout). Symmetric with
/// [`start_hosted_client_services`].
pub fn stop_hosted_client_services() {
    abort_hosted_client_tasks();
}
