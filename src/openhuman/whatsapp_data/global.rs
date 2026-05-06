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
use std::sync::{Arc, OnceLock};

use crate::openhuman::whatsapp_data::store::WhatsAppDataStore;

/// Shared, thread-safe reference to the store.
pub type WhatsAppDataStoreRef = Arc<WhatsAppDataStore>;

static GLOBAL_STORE: OnceLock<WhatsAppDataStoreRef> = OnceLock::new();

/// Initialise the global store from a workspace directory. Idempotent —
/// only the first call has any effect; subsequent calls return the existing
/// instance.
pub fn init(workspace_dir: PathBuf) -> Result<WhatsAppDataStoreRef, String> {
    if let Some(existing) = GLOBAL_STORE.get() {
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
    let _ = GLOBAL_STORE.set(Arc::clone(&store));
    Ok(GLOBAL_STORE.get().cloned().unwrap_or(store))
}

/// Return the global store. Errors if [`init`] has not been called yet.
pub fn store() -> Result<WhatsAppDataStoreRef, String> {
    GLOBAL_STORE.get().cloned().ok_or_else(|| {
        "whatsapp_data global store accessed before init — call init(workspace) at startup"
            .to_string()
    })
}

/// Return the global store if already initialised, without error.
pub fn store_if_ready() -> Option<WhatsAppDataStoreRef> {
    GLOBAL_STORE.get().cloned()
}
