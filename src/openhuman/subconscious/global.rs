//! Global singleton for the SubconsciousEngine.
//!
//! Shared between the heartbeat background loop and RPC handlers
//! so both see the same decision log, counters, and last_tick_at.

use super::engine::SubconsciousEngine;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

static ENGINE: OnceLock<Arc<Mutex<Option<SubconsciousEngine>>>> = OnceLock::new();

fn engine_lock() -> &'static Arc<Mutex<Option<SubconsciousEngine>>> {
    ENGINE.get_or_init(|| Arc::new(Mutex::new(None)))
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
