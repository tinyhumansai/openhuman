//! Host install slot, process-wide in production and thread-local in unit tests.

use super::MasterKeySource;
use std::sync::{Arc, RwLock};

#[cfg(not(test))]
fn slot() -> &'static RwLock<Option<Arc<dyn MasterKeySource>>> {
    static SLOT: std::sync::OnceLock<RwLock<Option<Arc<dyn MasterKeySource>>>> =
        std::sync::OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

#[cfg(test)]
fn slot() -> &'static RwLock<Option<Arc<dyn MasterKeySource>>> {
    thread_local! {
        static SLOT: &'static RwLock<Option<Arc<dyn MasterKeySource>>> =
            Box::leak(Box::new(RwLock::new(None)));
    }
    SLOT.with(|slot| *slot)
}

/// Install a host provider before core boot, replacing the previous provider.
pub fn install_master_key_source(provider: Arc<dyn MasterKeySource>) {
    *slot().write().unwrap_or_else(|e| e.into_inner()) = Some(provider);
}

/// The provider installed by the host, if any.
pub fn installed_master_key_source() -> Option<Arc<dyn MasterKeySource>> {
    slot().read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Remove the installed provider. Call only while the core is stopped.
pub fn clear_master_key_source() {
    *slot().write().unwrap_or_else(|e| e.into_inner()) = None;
}
