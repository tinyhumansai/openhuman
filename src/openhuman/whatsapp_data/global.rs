//! Process-global WhatsApp data store singleton.
//!
//! One `WhatsAppDataStore` lives for the entire core process, shared by RPC
//! handlers and any other subsystem that needs it.
//!
//! # Usage
//!
//! ```ignore
//! // At startup:
//! whatsapp_data::global::init(workspace_dir)?;
//!
//! // In RPC handlers:
//! let store = whatsapp_data::global::store()?;
//! ```

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::openhuman::whatsapp_data::store::WhatsAppDataStore;

/// Shared, thread-safe reference to the store.
pub type WhatsAppDataStoreRef = Arc<WhatsAppDataStore>;

// `RwLock<Option<…>>` rather than `OnceLock` so tests can swap workspaces
// between runs (each test uses its own temp dir; without reset, the second
// test would attach to a dropped sqlite path). Production callers still get
// strict idempotency: `init` is a no-op once a store is set.
static GLOBAL_STORE: RwLock<Option<WhatsAppDataStoreRef>> = RwLock::new(None);

/// Initialise the global store from a workspace directory. Idempotent —
/// only the first call has any effect; subsequent calls return the existing
/// instance.
pub fn init(workspace_dir: PathBuf) -> Result<WhatsAppDataStoreRef, String> {
    if let Some(existing) = GLOBAL_STORE
        .read()
        .map_err(|e| format!("[whatsapp_data:global] read lock poisoned: {e}"))?
        .as_ref()
    {
        log::debug!("[whatsapp_data:global] already initialised");
        return Ok(Arc::clone(existing));
    }
    log::info!(
        "[whatsapp_data:global] initialising store workspace={}",
        workspace_dir.display()
    );
    let store = Arc::new(
        WhatsAppDataStore::new(&workspace_dir)
            .map_err(|e| format!("[whatsapp_data] store init failed: {e}"))?,
    );
    let mut guard = GLOBAL_STORE
        .write()
        .map_err(|e| format!("[whatsapp_data:global] write lock poisoned: {e}"))?;
    // Race-resolve: another caller may have inited while we were building.
    if let Some(existing) = guard.as_ref() {
        return Ok(Arc::clone(existing));
    }
    *guard = Some(Arc::clone(&store));
    Ok(store)
}

/// Return the global store. Errors if [`init`] has not been called yet.
pub fn store() -> Result<WhatsAppDataStoreRef, String> {
    GLOBAL_STORE
        .read()
        .map_err(|e| format!("[whatsapp_data:global] read lock poisoned: {e}"))?
        .as_ref()
        .map(Arc::clone)
        .ok_or_else(|| {
            "whatsapp_data global store accessed before init — call init(workspace) at startup"
                .to_string()
        })
}

/// Return the global store if already initialised, without error.
pub fn store_if_ready() -> Option<WhatsAppDataStoreRef> {
    GLOBAL_STORE.read().ok()?.as_ref().map(Arc::clone)
}

/// Drop any currently-installed store handle so the next [`init`] re-binds
/// the global to a fresh workspace. Reachable from integration tests under
/// `tests/`, which see the crate as an external consumer and therefore can't
/// use a `#[cfg(test)]`-only symbol. Gated behind `cfg(any(test,
/// debug_assertions))` so the symbol is compiled out of release builds —
/// `cargo test` and dev builds keep `debug_assertions` on, `--release` turns
/// it off. Production callers MUST NOT invoke this at runtime — the SQLite
/// connection used by in-flight handlers would be released mid-call. Hidden
/// from rustdoc to discourage misuse.
#[cfg(any(test, debug_assertions))]
#[doc(hidden)]
pub fn reset_for_tests() {
    if let Ok(mut guard) = GLOBAL_STORE.write() {
        *guard = None;
    }
}
