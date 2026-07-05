//! Registry of live subconscious instances — one per enabled [`SubconsciousKind`].
//!
//! Shared between the heartbeat fan-out and the RPC handlers so both see the
//! same instances and counters. Instances are held as plain `Arc` (not
//! `Mutex<Option<..>>`): each instance's own `tick_lock`/`state` mutexes
//! serialize what needs serializing, and the status path must stay lock-free
//! (it never takes `tick_lock`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use super::factory::{make_subconscious, SubconsciousKind};
use super::instance::SubconsciousInstance;

static REGISTRY: OnceLock<Mutex<HashMap<SubconsciousKind, Arc<SubconsciousInstance>>>> =
    OnceLock::new();
static BOOTSTRAPPED: AtomicBool = AtomicBool::new(false);
static HEARTBEAT_HANDLE: OnceLock<Mutex<Option<JoinHandle<()>>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<SubconsciousKind, Arc<SubconsciousInstance>>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn heartbeat_slot() -> &'static Mutex<Option<JoinHandle<()>>> {
    HEARTBEAT_HANDLE.get_or_init(|| Mutex::new(None))
}

/// Lazily construct (or fetch) the instance for `kind`, keyed in the registry.
pub async fn get_or_init_instance(
    kind: SubconsciousKind,
) -> Result<Arc<SubconsciousInstance>, String> {
    {
        let map = registry().lock().await;
        if let Some(inst) = map.get(&kind) {
            return Ok(Arc::clone(inst));
        }
    }

    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e}"))?;
    let inst = Arc::new(make_subconscious(kind, &config));

    let mut map = registry().lock().await;
    Ok(Arc::clone(map.entry(kind).or_insert(inst)))
}

/// Every currently-registered instance (the bootstrap set), in a stable order
/// (memory first). Used by the heartbeat fan-out and `subconscious.status`.
pub async fn registered_instances() -> Vec<Arc<SubconsciousInstance>> {
    let map = registry().lock().await;
    SubconsciousKind::ALL
        .iter()
        .filter_map(|k| map.get(k).map(Arc::clone))
        .collect()
}

pub async fn bootstrap_after_login() -> Result<(), String> {
    if BOOTSTRAPPED.swap(true, Ordering::SeqCst) {
        tracing::debug!("[subconscious] bootstrap already ran — skipping");
        return Ok(());
    }

    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| {
            BOOTSTRAPPED.store(false, Ordering::SeqCst);
            format!("load config: {e}")
        })?;

    // The heartbeat loop is the clock that drives every instance's tick (plus
    // the event-planner duties). If it is disabled, no world ticks — same gate
    // as the pre-factory build.
    if !config.heartbeat.enabled {
        tracing::info!("[subconscious] heartbeat disabled in config — bootstrap skipped");
        BOOTSTRAPPED.store(false, Ordering::SeqCst);
        return Ok(());
    }

    // Initialize every enabled world against the per-user workspace.
    let kinds = SubconsciousKind::enabled_kinds(&config);
    for kind in &kinds {
        get_or_init_instance(*kind).await.inspect_err(|_e| {
            BOOTSTRAPPED.store(false, Ordering::SeqCst);
        })?;
    }
    tracing::info!(
        workspace = %config.workspace_dir.display(),
        kinds = ?kinds.iter().map(|k| k.id()).collect::<Vec<_>>(),
        "[subconscious] instances initialized against per-user workspace"
    );

    let heartbeat = crate::openhuman::heartbeat::engine::HeartbeatEngine::new(
        config.heartbeat.clone(),
        config.workspace_dir.clone(),
    );
    let handle = tokio::spawn(async move {
        if let Err(e) = heartbeat.run().await {
            tracing::warn!("[heartbeat] loop exited with error: {e}");
        }
    });
    *heartbeat_slot().lock().await = Some(handle);
    tracing::info!(
        "[heartbeat] periodic loop spawned ({}min interval)",
        config.heartbeat.interval_minutes
    );

    // Opt-in event-driven trigger pipeline. Require BOTH the flag and an
    // event-driven effective mode: if the user enables triggers and later
    // switches the subconscious mode to Off/Simple/Aggressive via the mode
    // selector (which doesn't clear `triggers_enabled`), the stale flag must
    // not silently reactivate background trigger processing.
    if config.heartbeat.triggers_enabled
        && config
            .heartbeat
            .effective_subconscious_mode()
            .is_event_driven()
    {
        bootstrap_trigger_orchestrator(&config);
    }

    Ok(())
}

/// Spawn the background trigger orchestrator (event loop) and register its
/// bus subscriber. Idempotent via the orchestrator's process-global slot.
fn bootstrap_trigger_orchestrator(config: &crate::openhuman::config::Config) {
    use crate::openhuman::subconscious_triggers::{
        init_orchestrator, register_subconscious_triggers_subscriber, OrchestratorConfig,
        TriggerOrchestrator,
    };

    let mode = config.heartbeat.effective_subconscious_mode();
    let session = Arc::new(super::LongLivedSession::new(
        config.workspace_dir.clone(),
        mode,
    ));
    let orch_config = OrchestratorConfig {
        max_promotions_per_hour: config.heartbeat.max_promotions_per_hour,
        ..OrchestratorConfig::default()
    };
    let orchestrator = init_orchestrator(Arc::new(TriggerOrchestrator::new(session, orch_config)));
    register_subconscious_triggers_subscriber(orchestrator);
    tracing::info!(
        workspace = %config.workspace_dir.display(),
        mode = %mode.as_str(),
        max_promotions_per_hour = config.heartbeat.max_promotions_per_hour,
        "[subconscious_triggers] event-driven orchestrator bootstrapped"
    );
}

pub async fn stop_heartbeat_loop() {
    if let Some(handle) = heartbeat_slot().lock().await.take() {
        handle.abort();
        match handle.await {
            Ok(()) => {
                tracing::debug!("[heartbeat] loop exited before abort completed");
            }
            Err(join_err) if join_err.is_cancelled() => {
                tracing::info!("[heartbeat] loop aborted");
            }
            Err(join_err) => {
                tracing::warn!(error = %join_err, "[heartbeat] loop abort join failed");
            }
        }
    }

    // Tear down the event-driven trigger orchestrator + its bus subscriber on
    // every stop (disable, mode change, user switch) so a stale session/loop
    // never keeps routing trigger work after the pipeline is turned off or the
    // workspace changes. A subsequent bootstrap re-creates them when enabled.
    crate::openhuman::subconscious_triggers::shutdown_orchestrator();
    crate::openhuman::subconscious_triggers::unregister_subconscious_triggers_subscriber();

    BOOTSTRAPPED.store(false, Ordering::SeqCst);
}

/// Reset for a user switch: stop the heartbeat + triggers, then clear the whole
/// registry so a subsequent bootstrap rebuilds every instance against the new
/// workspace.
pub async fn reset_engine_for_user_switch() {
    stop_heartbeat_loop().await;
    registry().lock().await.clear();
    tracing::info!("[subconscious] registry reset for user switch");
}
