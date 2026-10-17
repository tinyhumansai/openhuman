//! The per-thread (per profile) delivery slot: at most one delivery turn is in
//! flight for a thread, and the batch it claimed is leased until it settles.

use std::sync::Arc;

use tinyagents_tasks::CompletionRouter;

use crate::core::runtime::tenant;
use crate::core::runtime::DispatchCtx;

/// The per-thread (per profile) delivery slot, plus the batch lease.
/// Dropping it frees both: `release` on an already-settled record is a no-op,
/// so a settled batch is untouched and an abandoned one is claimable again.
pub(super) struct DeliverySlot {
    /// [`tenant::profile_key_in`] of the thread id.
    pub(super) key: String,
    router: Arc<CompletionRouter>,
    held: Vec<String>,
}

impl DeliverySlot {
    /// `None` when a delivery is already in flight for the thread.
    pub(super) fn claim(
        ctx: &DispatchCtx,
        thread_id: &str,
        router: Arc<CompletionRouter>,
    ) -> Option<Self> {
        let key = tenant::profile_key_in(&super::super::busy_guard::caller(ctx), thread_id);
        let mut d = super::delivering().lock().expect("delivering poisoned");
        if !d.insert(key.clone()) {
            return None;
        }
        Some(Self {
            key,
            router,
            held: Vec::new(),
        })
    }

    pub(super) fn hold(&mut self, task_ids: &[String]) {
        self.held = task_ids.to_vec();
    }
}

impl Drop for DeliverySlot {
    fn drop(&mut self) {
        if !self.held.is_empty() {
            self.router.release(&self.held);
        }
        if let Ok(mut d) = super::delivering().lock() {
            d.remove(&self.key);
        }
    }
}
