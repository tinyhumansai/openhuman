//! Global singleton for the SubconsciousEngine.
//!
//! Shared between the heartbeat background loop and RPC handlers
//! so both see the same decision log, counters, and last_tick_at.
//!
//! Lifecycle note: the engine is bootstrapped **post-login** via
//! [`bootstrap_after_login`] so that `seed_default_tasks` runs against the
//! per-user workspace (`~/.openhuman/users/<id>/workspace/`) instead of the
//! pre-login global default. See `load.rs::resolve_runtime_config_dirs` for
//! how `active_user.toml` drives `config.workspace_dir`.

use super::engine::SubconsciousEngine;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

static ENGINE: OnceLock<Arc<Mutex<Option<SubconsciousEngine>>>> = OnceLock::new();

/// True once [`bootstrap_after_login`] has successfully seeded the engine and
/// spawned the heartbeat loop for the current active user.
static BOOTSTRAPPED: AtomicBool = AtomicBool::new(false);

/// Heartbeat loop handle so logout / user switch can abort it cleanly.
static HEARTBEAT_HANDLE: OnceLock<Mutex<Option<JoinHandle<()>>>> = OnceLock::new();

fn engine_lock() -> &'static Arc<Mutex<Option<SubconsciousEngine>>> {
    ENGINE.get_or_init(|| Arc::new(Mutex::new(None)))
}

fn heartbeat_slot() -> &'static Mutex<Option<JoinHandle<()>>> {
    HEARTBEAT_HANDLE.get_or_init(|| Mutex::new(None))
}

/// Get or initialize the global engine. Both heartbeat loop and RPC use this.
pub async fn get_or_init_engine() -> Result<Arc<Mutex<Option<SubconsciousEngine>>>, String> {
    let lock = engine_lock();
    {
        let guard = lock.lock().await;
        if guard.is_some() {
            return Ok(Arc::clone(lock));
        }
    }

    // Initialize
    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e}"))?;

    let memory =
        crate::openhuman::memory::MemoryClient::from_workspace_dir(config.workspace_dir.clone())
            .ok()
            .map(Arc::new);

    let engine = SubconsciousEngine::new(&config, memory);

    let mut guard = lock.lock().await;
    if guard.is_none() {
        *guard = Some(engine);
    }

    Ok(Arc::clone(lock))
}

/// Construct the engine (which seeds defaults into the per-user workspace)
/// and spawn the heartbeat loop. Idempotent per-process via [`BOOTSTRAPPED`].
///
/// Call this:
/// - after a successful login writes `active_user.toml`, OR
/// - at sidecar startup **iff** `active_user.toml` already exists.
///
/// Calling before login would seed into the global pre-login workspace and
/// then silently diverge from the per-user workspace the UI reads from.
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
        return Ok(());
    }

    // Build the engine against the NOW-correct per-user workspace_dir.
    // SubconsciousEngine::new calls seed_default_tasks() inside the
    // constructor, so by the time this returns the 3 system defaults are
    // present in `<workspace>/subconscious/subconscious.db`.
    get_or_init_engine().await.inspect_err(|e| {
        BOOTSTRAPPED.store(false, Ordering::SeqCst);
    })?;
    tracing::info!(
        workspace = %config.workspace_dir.display(),
        "[subconscious] engine initialized against per-user workspace"
    );

    // Spawn the heartbeat loop and keep the JoinHandle so we can cancel it
    // on logout. Without this the task would leak: tokio::spawn returns a
    // detached task that drops on handle-drop but keeps running.
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

/// Tear down the engine + heartbeat loop so the next login rebuilds them
/// against the new user's workspace. Call on logout or account switch.
///
/// Without this, the engine `OnceLock` would stay frozen on the previous
/// user's `workspace_dir` and subsequent ticks / RPC queries would leak
/// into the wrong DB.
pub async fn reset_engine_for_user_switch() {
    if let Some(handle) = heartbeat_slot().lock().await.take() {
        handle.abort();
        tracing::info!("[heartbeat] loop aborted for user switch");
    }

    let lock = engine_lock();
    let mut guard = lock.lock().await;
    *guard = None;

    BOOTSTRAPPED.store(false, Ordering::SeqCst);
    tracing::info!("[subconscious] engine reset for user switch");
}
