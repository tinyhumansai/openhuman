//! Process-global runtime handles for the curated-memory controllers.
//! Mirrors the pattern in `life_capture::runtime`: stateless controller
//! handlers fetch the long-lived `MemoryStore`s from a `OnceCell` set at
//! startup, returning a structured "not initialised" error if startup
//! hasn't run yet (e.g. during pure-RPC unit tests).

use std::sync::Arc;
use tokio::sync::OnceCell;

use crate::openhuman::curated_memory::MemoryStore;

pub struct CuratedMemoryRuntime {
    pub memory: Arc<MemoryStore>,
    pub user: Arc<MemoryStore>,
}

static RUNTIME: OnceCell<Arc<CuratedMemoryRuntime>> = OnceCell::const_new();

pub async fn init(rt: Arc<CuratedMemoryRuntime>) -> Result<(), &'static str> {
    RUNTIME
        .set(rt)
        .map_err(|_| "curated_memory runtime already initialised")
}

pub fn get() -> Result<Arc<CuratedMemoryRuntime>, &'static str> {
    RUNTIME
        .get()
        .cloned()
        .ok_or("curated_memory runtime not initialised")
}
