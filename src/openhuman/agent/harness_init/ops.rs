//! Harness-init orchestrator + RPC handlers.
//!
//! `run_harness_init` is spawned (non-blocking) from `bootstrap_core_runtime`
//! after the core is RPC-ready, so the frontend can connect and watch progress.
//! It walks the [`registry`] step list, marking each `Done` instantly when its
//! cheap `is_done` probe passes, otherwise running it and recording the result.

use serde_json::{json, Map, Value};

use super::registry::{self, HarnessInitStep};
use super::store;
use super::types::{HarnessInitSnapshot, OverallState, StepState};
use crate::core::all::ControllerFuture;
use crate::openhuman::config::Config;

/// Run every registered step once. `force` re-runs steps even when their
/// `is_done` probe passes (used by the retry RPC). Failures of non-required
/// steps are recorded as `Skipped`; the overall state is `Failed` only when a
/// *required* step fails.
pub async fn run_harness_init_with(config: Config, force: bool) -> HarnessInitSnapshot {
    log::info!("[harness_init] starting one-time init run (force={force})");

    let steps = registry::all_steps();

    // Warm-start guard (GH-5047): decide visibility *before* publishing any
    // `Running` state. The blocking overlay is justified only when a
    // provisioning step (download/install/repair) genuinely needs work. Routine
    // startup — e.g. re-launching an already-installed local Python server — is
    // NOT provisioning and must run silently in the background on every launch.
    // Probing on-disk readiness first (durably) keeps the overlay off warm
    // restarts entirely instead of flashing it while the run settles.
    let needs_provisioning = force || provisioning_required(&config, &steps).await;
    if needs_provisioning {
        log::info!("[harness_init] provisioning required — surfacing setup overlay");
        store::set_overall(OverallState::Running);
    } else {
        log::info!(
            "[harness_init] warm start — all provisioning satisfied; running remaining startup silently"
        );
    }

    let mut failed_required = false;

    for step in &steps {
        run_one(&config, step, force, &mut failed_required).await;
    }

    let overall = if failed_required {
        OverallState::Failed
    } else {
        OverallState::Done
    };
    store::set_overall(overall);
    store::publish_completed(overall, failed_required);
    log::info!("[harness_init] init run finished overall={overall:?}");
    store::snapshot()
}

/// Boot entrypoint: run all not-yet-done steps.
pub async fn run_harness_init(config: Config) {
    run_harness_init_with(config, false).await;
}

/// Whether any *provisioning* step (download/install/repair) still needs work.
///
/// Only provisioning steps gate the visible overlay; a non-provisioning step
/// (routine service startup) is ignored here so an already-installed host never
/// surfaces the blocking screen just to relaunch a background server. Each
/// probe is the step's durable, network-free `is_done` — see [`registry`].
async fn provisioning_required(config: &Config, steps: &[HarnessInitStep]) -> bool {
    for step in steps {
        if !step.provisioning {
            continue;
        }
        if !(step.is_done)(config).await {
            log::info!(
                "[harness_init] provisioning step {} not satisfied on disk — setup needed",
                step.id
            );
            return true;
        }
    }
    false
}

async fn run_one(config: &Config, step: &HarnessInitStep, force: bool, failed_required: &mut bool) {
    if !force && (step.is_done)(config).await {
        log::debug!("[harness_init] step {} already satisfied", step.id);
        store::update_step(
            step.id,
            StepState::Done,
            Some("already provisioned".to_string()),
            Some(100),
        );
        return;
    }

    store::update_step(step.id, StepState::Running, None, None);
    match (step.run)(config).await {
        Ok(()) => {
            store::update_step(step.id, StepState::Done, None, Some(100));
        }
        Err(msg) => {
            log::warn!("[harness_init] step {} failed: {msg}", step.id);
            if step.required {
                *failed_required = true;
                store::update_step(step.id, StepState::Failed, Some(msg), None);
            } else {
                // Non-fatal: the app proceeds with a fallback.
                store::update_step(step.id, StepState::Skipped, Some(msg), None);
            }
        }
    }
}

// ── RPC handlers ────────────────────────────────────────────────────────────

/// `openhuman.harness_init_status` — return the current snapshot.
pub fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { snapshot_response(store::snapshot()) })
}

/// `openhuman.harness_init_run` — re-run init (retry). `force` re-runs even
/// already-satisfied steps; defaults to false (only retries pending/failed).
pub fn handle_run(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let force = params
            .get("force")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let config = crate::openhuman::config::rpc::load_config_with_timeout().await?;
        let snapshot = run_harness_init_with(config, force).await;
        snapshot_response(snapshot)
    })
}

fn snapshot_response(snapshot: HarnessInitSnapshot) -> Result<Value, String> {
    let value = serde_json::to_value(snapshot)
        .map_err(|e| format!("failed to serialize harness-init snapshot: {e}"))?;
    Ok(json!({ "snapshot": value }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn status_response_wraps_snapshot_key() {
        let resp = handle_status(Map::new()).await.unwrap();
        assert!(resp.get("snapshot").is_some());
        let overall = resp["snapshot"]["overall"].as_str().unwrap();
        // Idle before any run, or a later state if a boot run already executed
        // in this process — both are valid, we only assert the shape.
        assert!(["idle", "running", "done", "failed"].contains(&overall));
        assert!(resp["snapshot"]["steps"].is_array());
    }
}
