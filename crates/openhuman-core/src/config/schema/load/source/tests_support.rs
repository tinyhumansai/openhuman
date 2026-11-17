//! Test seam: force the document source for the current thread without
//! installing a process-wide backend that every other test would see.
use std::cell::RefCell;
use std::sync::Arc;

use tinystoragedrivers::secrets::KeyProvider;

use crate::storage::{Scope, Storage};

/// What a forced document source is built from.
#[derive(Clone)]
pub(crate) struct Forced {
    pub scoped: Storage,
    pub scope: Scope,
    pub keys: Arc<dyn KeyProvider>,
    pub identity: Arc<()>,
}

thread_local! {
    static FORCED: RefCell<Option<Forced>> = const { RefCell::new(None) };
}

pub(crate) fn forced_document_scope() -> Option<Forced> {
    FORCED.with(|forced| forced.borrow().clone())
}

/// Forces the document source on this thread until the guard drops, then
/// restores whatever was forced before (guards nest).
pub(crate) struct ForcedDocumentSource {
    previous: Option<Forced>,
}

impl ForcedDocumentSource {
    pub(crate) fn new(scoped: Storage, scope: Scope, keys: Arc<dyn KeyProvider>) -> Self {
        let previous = FORCED.with(|forced| {
            forced.borrow_mut().replace(Forced {
                scoped,
                scope,
                keys,
                identity: Arc::new(()),
            })
        });
        Self { previous }
    }
}

impl Drop for ForcedDocumentSource {
    fn drop(&mut self) {
        let previous = self.previous.take();
        FORCED.with(|forced| *forced.borrow_mut() = previous);
    }
}
