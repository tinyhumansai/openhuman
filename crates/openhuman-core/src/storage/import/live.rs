//! The session key-value store a turn registers on its `RunContext`.
//!
//! Sessions live in the storage backend (`DriverSessionStores`), one scope per
//! agent. The file-era mirror of transcripts into a second layout, and the
//! store-backed shadow read that compared against it, are gone: the backend is
//! the only record. What remains is the harness-visible handle to the agent's
//! key-value store, registered on each turn's `RunContext.stores` under
//! [`TINYAGENTS_SESSION_KV_STORE`].

use crate::core::runtime::DispatchCtx;
use std::sync::Arc;

use tinyagents_harness::store::Store;

/// Store-registry name under which the session KV store is registered on each
/// turn's `RunContext.stores`. Slash-free so it round-trips store-name
/// sanitizers.
pub const TINYAGENTS_SESSION_KV_STORE: &str = "openhuman_sessions";

/// The calling agent's key-value store as an `Arc<dyn Store>` for registration
/// on the per-turn `RunContext.stores` under [`TINYAGENTS_SESSION_KV_STORE`].
///
/// Best-effort: `None` when the config (hence workspace) cannot be resolved and
/// no store is installed.
pub async fn session_kv_store(ctx: &DispatchCtx) -> Option<Arc<dyn Store>> {
    if let Some(stores) = crate::storage::sessions::current(ctx) {
        log::debug!("[session-store] registering the host session store's kv on RunContext.stores");
        return Some(stores.kv);
    }
    let cfg = match crate::config::ops::load_current_or_init(ctx).await {
        Ok(cfg) => cfg,
        Err(err) => {
            log::warn!("[session-store] cannot resolve config for store registration: {err:#}");
            return None;
        }
    };
    let stores = crate::storage::sessions::stores_for(&cfg.workspace_dir)?;
    log::debug!(
        "[session-store] opened session kv store for RunContext.stores workspace={}",
        cfg.workspace_dir.display()
    );
    Some(stores.kv)
}
