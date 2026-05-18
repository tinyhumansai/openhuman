//! Process-global scratch store used as a fallback when no thread context
//! is present (e.g. agent-tool invocations outside a chat thread). The
//! authoritative per-thread state lives in
//! [`crate::openhuman::agent::task_board::TaskBoardStore`].

use crate::openhuman::agent::task_board::TaskBoardCard;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::sync::Arc;

/// Process-global scratch list of cards used when there is no current
/// thread id. Replaced wholesale by callers.
#[derive(Default)]
pub struct ScratchTodoStore {
    inner: Mutex<Vec<TaskBoardCard>>,
}

impl ScratchTodoStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Vec<TaskBoardCard> {
        self.inner.lock().clone()
    }

    pub fn replace(&self, cards: Vec<TaskBoardCard>) {
        *self.inner.lock() = cards;
    }
}

// NOTE: scratch CRUD calls in [`super::ops`] (load → mutate → save) are
// serialised by a coarser process-global mutex on the ops side, so the
// pair runs in one critical section even though [`snapshot`] and
// [`replace`] each take this inner lock independently.

/// Process-global scratch store handle. The same `Arc` is returned across
/// calls so tool re-registration doesn't lose in-memory state.
pub fn global_scratch_store() -> Arc<ScratchTodoStore> {
    static STORE: OnceCell<Arc<ScratchTodoStore>> = OnceCell::new();
    STORE
        .get_or_init(|| Arc::new(ScratchTodoStore::new()))
        .clone()
}
