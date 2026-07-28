//! Background driver for the base-store re-embedding sweep.
//!
//! [`namespace_store::reembed`](super::namespace_store::reembed) knows *which*
//! `vector_chunks` rows need a vector and how to compute one; this module
//! decides *when* to run it. The pending set lives in the table itself, so a
//! trigger is only ever a nudge: whatever a run leaves unfinished — because the
//! provider was down, the session was signed out, or the process exited — is
//! rediscovered by the next one. That is what makes recovery survive a service
//! restart without a queue to keep in sync.
//!
//! Idempotent and non-fatal, like the tree-side
//! [`ensure_reembed_backfill`](crate::openhuman::memory::queue::ensure_reembed_backfill)
//! it runs beside: at most one sweep is in flight, and a failure is logged
//! rather than propagated into whatever user action triggered it.

use std::sync::atomic::{AtomicBool, Ordering};

/// Chunks per pass — also the provider batch size. Small enough that a failing
/// provider wastes one modest call rather than a huge one, and that a sweep
/// competes gently with live embedding traffic.
const SWEEP_BATCH: usize = 64;

/// Upper bound on passes per trigger. A backlog larger than this (flo carried
/// thousands of vector-less gmail chunks) is drained across triggers instead of
/// in one unbounded run, so a sweep can never monopolise the embedder.
const MAX_BATCHES_PER_RUN: usize = 64;

static SWEEP_RUNNING: AtomicBool = AtomicBool::new(false);

/// Clears the in-flight flag however the sweep ends, including a panic — a
/// stuck flag would silently disable re-embedding for the rest of the process.
struct SweepGuard;

impl Drop for SweepGuard {
    fn drop(&mut self) {
        SWEEP_RUNNING.store(false, Ordering::SeqCst);
    }
}

/// Nudge the sweep: drain pending chunks in the background if a sweep is not
/// already running and the memory store is up.
///
/// Returns immediately — callers are user-facing paths (sync completion,
/// startup, a credential or model change) that must not wait on the embedder.
pub fn ensure_vector_reembed() {
    if tokio::runtime::Handle::try_current().is_err() {
        // No runtime to spawn onto (unit tests, sync CLI paths). The rows stay
        // pending, so the next trigger inside the service picks them up.
        return;
    }
    let Some(client) = crate::openhuman::memory::global::client_if_ready() else {
        return;
    };
    if SWEEP_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        let _guard = SweepGuard;
        let mut repaired = 0usize;
        for _ in 0..MAX_BATCHES_PER_RUN {
            let report = client.sweep_pending_embeddings(SWEEP_BATCH).await;
            repaired += report.reembedded;
            // Nothing pending, or a pass that repaired nothing: the provider is
            // failing or the remaining rows are un-embeddable. Either way the
            // next trigger retries — spinning here would only burn quota.
            if report.scanned == 0 || report.reembedded == 0 {
                break;
            }
        }
        if repaired > 0 {
            log::info!("[memory::reembed] sweep repaired {repaired} chunk(s)");
        }
    });
}

#[cfg(test)]
#[path = "vector_reembed_tests.rs"]
mod tests;
