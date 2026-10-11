//! Host install slot, process-wide in production and thread-local in unit tests.

use super::backend::SecretBackend;
use std::sync::{Arc, RwLock};

#[cfg(not(test))]
fn slot() -> &'static RwLock<Option<Arc<dyn SecretBackend>>> {
    static SLOT: std::sync::OnceLock<RwLock<Option<Arc<dyn SecretBackend>>>> =
        std::sync::OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

#[cfg(test)]
fn slot() -> &'static RwLock<Option<Arc<dyn SecretBackend>>> {
    thread_local! {
        static SLOT: &'static RwLock<Option<Arc<dyn SecretBackend>>> =
            Box::leak(Box::new(RwLock::new(None)));
    }
    SLOT.with(|slot| *slot)
}

/// Install a host provider before core boot, replacing the previous provider.
pub fn install_secret_backend(provider: Arc<dyn SecretBackend>) {
    *slot().write().unwrap_or_else(|e| e.into_inner()) = Some(provider);
    super::reset_availability_cache();
}

/// The provider installed by the host, if any.
pub fn installed_secret_backend() -> Option<Arc<dyn SecretBackend>> {
    slot().read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Remove the installed provider. Call only while the core is stopped.
pub fn clear_secret_backend() {
    *slot().write().unwrap_or_else(|e| e.into_inner()) = None;
    super::reset_availability_cache();
}
