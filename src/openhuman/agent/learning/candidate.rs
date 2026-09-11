//! Learning candidates — observations the stability detector may promote into
//! durable profile facts.
//!
//! Two halves live here, and they arrived from different places:
//!
//! - the **taxonomy** ([`FacetClass`], [`CueFamily`], [`LearningCandidate`],
//!   [`EvidenceRef`]) is defined in `tinymemory-api`, the contract crate the
//!   host and the loadable memory module both compile against, and is
//!   re-exported below so the ~30 call sites that already spell
//!   `candidate::FacetClass` keep resolving;
//! - the **buffer** ([`Buffer`] and its [`global`] singleton) is this process's
//!   own, and is defined here.
//!
//! # Why the buffer came home (#5560)
//!
//! It used to be `pub use tinymemory_core::learning_candidate::*;` — the last
//! line in this file that named the memory engine, and therefore a compile-time
//! link to it bought for a `VecDeque` behind a `parking_lot::Mutex`. The engine
//! crate's own module docs explain why the type could not follow the taxonomy
//! down into `tinymemory-api`: **a `static` is not a payload.** The contract
//! crate is compiled into both the host binary and the module `cdylib`, so
//! moving `global()` there would not give the two sides one queue, it would
//! give them two.
//!
//! That is exactly why moving the definition *up* into the host is free. The
//! split it looks like it might create is **already live and cannot be closed
//! by a re-export**: the one producer inside the memory workspace is
//! `sync::composio::providers::profile`, which runs inside the module and
//! pushes into the module's copy of the static; the host's stability detector
//! drains the host's copy. Every producer in *this* process —
//! `learning::extract::heuristics`, `extract::summary_facets`,
//! `extract::signature`, `learning::reflection` — reaches the buffer through
//! `candidate::global()`, i.e. through this file, and so does the only
//! consumer, `learning::stability_detector`. Defining the static here keeps
//! them on one buffer, which is the property that matters; delivering a
//! candidate across the module boundary needs a bus member or an event, and is
//! tracked upstream rather than papered over here.
//!
//! The capacity, the FIFO-overflow eviction and every method signature are
//! carried over unchanged, so a producer that used to evict at 1024 still does.

use std::collections::VecDeque;
use std::sync::OnceLock;

use parking_lot::Mutex;

// The taxonomy, on the contract crate. Named on `tinymemory_api` rather than
// through `memory::api`, which deliberately re-exports only what crosses the
// module bus — `learning` does not (see `memory::api`'s docs), and the whole
// point of this file is that the *producer* on the far side of that bus and the
// consumer here are not sharing a queue.
pub use tinymemory_api::host::EvidenceRef;
pub use tinymemory_api::learning::{CueFamily, FacetClass, LearningCandidate};

// ── Buffer ───────────────────────────────────────────────────────────────────

/// Thread-safe, bounded ring-buffer of [`LearningCandidate`] items.
///
/// Backed by a `parking_lot::Mutex<VecDeque<LearningCandidate>>`. When full the
/// oldest entry is evicted to make room (FIFO overflow), which keeps memory
/// bounded and naturally prioritises recent evidence.
///
/// [`global`] is the singleton every producer in this process pushes into and
/// the stability detector drains; tests build their own with [`Buffer::new`].
pub struct Buffer {
    inner: Mutex<VecDeque<LearningCandidate>>,
    capacity: usize,
}

impl Buffer {
    /// Create a new buffer with the given capacity.
    ///
    /// `capacity` must be ≥ 1. A capacity of zero would make every `push` a
    /// silent no-op, so it is clamped rather than honoured.
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            inner: Mutex::new(VecDeque::with_capacity(cap)),
            capacity: cap,
        }
    }

    /// Push a candidate onto the buffer.
    ///
    /// If the buffer is already at capacity the oldest entry is evicted first
    /// (FIFO overflow), so the buffer always reflects the most recent evidence.
    pub fn push(&self, candidate: LearningCandidate) {
        let mut guard = self.inner.lock();
        if guard.len() >= self.capacity {
            guard.pop_front(); // evict oldest
        }
        guard.push_back(candidate);
    }

    /// Drain all candidates from the buffer and return them in FIFO order.
    ///
    /// After this call the buffer is empty.
    pub fn drain(&self) -> Vec<LearningCandidate> {
        let mut guard = self.inner.lock();
        guard.drain(..).collect()
    }

    /// Clone all candidates without removing them.
    ///
    /// Useful for inspection or debugging.
    pub fn peek(&self) -> Vec<LearningCandidate> {
        let guard = self.inner.lock();
        guard.iter().cloned().collect()
    }

    /// Current number of candidates in the buffer.
    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }

    /// Returns `true` when the buffer holds no candidates.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Maximum number of candidates the buffer will hold.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

// ── Global singleton ─────────────────────────────────────────────────────────

static GLOBAL_BUFFER: OnceLock<Buffer> = OnceLock::new();

/// Return the global [`Buffer`] singleton.
///
/// Initialised on first call with a capacity of 1024 — the same default the
/// engine's copy carried, so a run that overflows evicts at the same point it
/// did before this definition moved.
pub fn global() -> &'static Buffer {
    GLOBAL_BUFFER.get_or_init(|| Buffer::new(1024))
}

#[cfg(test)]
#[path = "candidate_tests.rs"]
mod tests;
