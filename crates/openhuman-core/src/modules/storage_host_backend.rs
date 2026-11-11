//! Which backend a module's storage binds to. Under `cfg(test)` a connector
//! module test serves its own storage here while it holds the module lock
//! (`module_guard`), kept out of the process storage slot so it is never
//! another concurrent test's storage (MAIN-57).

use std::sync::Arc;

use crate::storage::StorageBackend;

/// The backend `module`'s storage binds to: the host's, or under `cfg(test)`
/// the storage a connector-module test is serving.
pub(super) fn for_module(
    module: &str,
    environment: &dyn super::Environment,
) -> Option<Arc<dyn StorageBackend>> {
    #[cfg(test)]
    if module == super::super::connectors::MODULE_ID {
        if let Some(backend) = test_module_storage() {
            return Some(backend);
        }
    }
    let _ = module;
    environment.backend()
}

#[cfg(test)]
/// Storage for module calls in unit tests that hold a module lock (the
/// connector module's `module_guard`). A module call carries no workspace, so
/// such a test needs a backend; this one is read only here, so it is never
/// another test's storage the way an installed process slot is (MAIN-57).
static TEST_MODULE_STORAGE: std::sync::Mutex<Option<Arc<dyn StorageBackend>>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
fn test_module_storage() -> Option<Arc<dyn StorageBackend>> {
    TEST_MODULE_STORAGE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

#[cfg(test)]
/// Serve `backend` to module calls until the returned guard drops. Hold the
/// module's test lock for as long as it lives.
pub(crate) fn serve_test_module_storage(backend: Arc<dyn StorageBackend>) -> TestModuleStorage {
    let previous = TEST_MODULE_STORAGE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .replace(backend);
    TestModuleStorage { previous }
}

#[cfg(test)]
/// Withdraw the backend module calls are served, as a storage outage would,
/// while the guard from [`serve_test_module_storage`] is still held.
pub(crate) fn withdraw_test_module_storage() {
    *TEST_MODULE_STORAGE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

#[cfg(test)]
/// Restores the previous test module storage on drop.
pub(crate) struct TestModuleStorage {
    previous: Option<Arc<dyn StorageBackend>>,
}

#[cfg(test)]
impl Drop for TestModuleStorage {
    fn drop(&mut self) {
        *TEST_MODULE_STORAGE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = self.previous.take();
    }
}
