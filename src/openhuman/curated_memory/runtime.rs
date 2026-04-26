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

/// Open MEMORY.md / USER.md at `<workspace>/memories/` and register the
/// curated-memory runtime singleton. Idempotent — second-init is a no-op.
pub async fn bootstrap(workspace_dir: &std::path::Path) {
    let mem_dir = workspace_dir.join("memories");
    let memory_store = crate::openhuman::curated_memory::MemoryStore::open(
        &mem_dir,
        crate::openhuman::curated_memory::MemoryFile::Memory,
        2200,
    );
    let user_store = crate::openhuman::curated_memory::MemoryStore::open(
        &mem_dir,
        crate::openhuman::curated_memory::MemoryFile::User,
        1375,
    );
    match (memory_store, user_store) {
        (Ok(memory), Ok(user)) => {
            let rt = Arc::new(CuratedMemoryRuntime {
                memory: Arc::new(memory),
                user: Arc::new(user),
            });
            match init(rt).await {
                Ok(()) => log::info!(
                    "[curated_memory] runtime initialised at {}",
                    mem_dir.display()
                ),
                Err(e) => log::debug!("[curated_memory] init skipped: {e}"),
            }
        }
        (Err(e), _) | (_, Err(e)) => log::warn!(
            "[curated_memory] failed to open stores at {}: {e}",
            mem_dir.display()
        ),
    }
}
