//! Global singleton for the SubconsciousEngine.
//!
//! Shared between the heartbeat background loop and RPC handlers
//! so both see the same state and counters.

use super::engine::SubconsciousEngine;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

static ENGINE: OnceLock<Arc<Mutex<Option<SubconsciousEngine>>>> = OnceLock::new();
static BOOTSTRAPPED: AtomicBool = AtomicBool::new(false);
static HEARTBEAT_HANDLE: OnceLock<Mutex<Option<JoinHandle<()>>>> = OnceLock::new();

fn engine_lock() -> &'static Arc<Mutex<Option<SubconsciousEngine>>> {
    ENGINE.get_or_init(|| Arc::new(Mutex::new(None)))
}

fn heartbeat_slot() -> &'static Mutex<Option<JoinHandle<()>>> {
    HEARTBEAT_HANDLE.get_or_init(|| Mutex::new(None))
}

pub async fn get_or_init_engine() -> Result<Arc<Mutex<Option<SubconsciousEngine>>>, String> {
    let lock = engine_lock();
    {
        let guard = lock.lock().await;
        if guard.is_some() {
            return Ok(Arc::clone(lock));
        }
    }

    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e}"))?;

    let memory = crate::openhuman::memory_store::MemoryClient::from_workspace_dir(
        config.workspace_dir.clone(),
    )
    .ok()
    .map(Arc::new);

    let engine = SubconsciousEngine::new(&config, memory);

    let mut guard = lock.lock().await;
    if guard.is_none() {
        *guard = Some(engine);
    }

    Ok(Arc::clone(lock))
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

    if !config.heartbeat.enabled {
        tracing::info!("[subconscious] heartbeat disabled in config — bootstrap skipped");
        BOOTSTRAPPED.store(false, Ordering::SeqCst);
        return Ok(());
    }

    get_or_init_engine().await.inspect_err(|_e| {
        BOOTSTRAPPED.store(false, Ordering::SeqCst);
    })?;
    tracing::info!(
        workspace = %config.workspace_dir.display(),
        "[subconscious] engine initialized against per-user workspace"
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

    Ok(())
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

    BOOTSTRAPPED.store(false, Ordering::SeqCst);
}

pub async fn reset_engine_for_user_switch() {
    stop_heartbeat_loop().await;

    let lock = engine_lock();
    let mut guard = lock.lock().await;
    *guard = None;

    tracing::info!("[subconscious] engine reset for user switch");
}
