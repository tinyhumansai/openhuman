//! Memory-queue operations: backfill-progress signalling and the re-embed
//! backfill switch-path trigger.
//!
//! Split out of `mod.rs` so the module root stays export-focused. Public paths
//! are preserved via re-exports in [`super`], so callers keep using
//! `crate::openhuman::memory_queue::<fn>`.

use std::sync::atomic::{AtomicBool, Ordering};

use super::{store, types};

/// #1574 §6 / #1365: set while a re-embed backfill chain has work pending.
///
/// Read by the first-person / subconscious retrieval layer so an empty
/// vector-search result during the backfill window is interpreted as
/// "not searched yet" rather than "no such memory" — preventing the agent
/// from confidently asserting false self-ignorance mid-re-embed. Set true
/// when a backfill is enqueued / still has rows; cleared when the chain
/// drains. Process-global (resets to false on restart; the worker re-sets
/// it on the next backfill tick — acceptable for v1).
static BACKFILL_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// Mark whether a re-embed backfill currently has pending work.
pub fn set_backfill_in_progress(v: bool) {
    BACKFILL_IN_PROGRESS.store(v, Ordering::Relaxed);
}

/// True while a re-embed backfill chain still has rows to process. The
/// #1365 absence-reasoning consumer checks this before treating an empty
/// semantic-recall result as "no memory exists".
pub fn backfill_in_progress() -> bool {
    BACKFILL_IN_PROGRESS.load(Ordering::Relaxed)
}

/// #1574 §4: ensure a re-embed backfill chain exists for the **current**
/// active signature, if (and only if) there is uncovered work.
///
/// This is the switch-path trigger: call it after the embedder config
/// changes (a new signature → every prior row is missing at it). The §7
/// migration is one-shot (`user_version`-gated) so it does NOT fire on a
/// later model switch — without this, switching silently blinds prior
/// memory. Standalone (own connection); the §7 migration keeps its own
/// in-tx enqueue (atomic with the copy). Idempotent + non-fatal: the
/// per-signature dedupe key means at most one chain per space, and a
/// covered space enqueues nothing. Errors are logged, never propagated —
/// a failed enqueue must not fail the user's settings save.
pub fn ensure_reembed_backfill(config: &crate::openhuman::config::Config) {
    let sig = crate::openhuman::memory_store::chunks::store::tree_active_signature(config);
    let result = crate::openhuman::memory_store::chunks::store::with_connection(config, |conn| {
        Ok(crate::openhuman::memory_store::chunks::store::has_uncovered_reembed_work(conn, &sig)?)
    });
    match result {
        Ok(true) => {
            let job = match types::NewJob::reembed_backfill(&types::ReembedBackfillPayload {
                signature: sig.clone(),
            }) {
                Ok(j) => j,
                Err(e) => {
                    log::warn!("[memory::jobs] ensure_reembed_backfill: build job failed: {e}");
                    return;
                }
            };
            match store::enqueue(config, &job) {
                Ok(_) => {
                    set_backfill_in_progress(true);
                    log::info!(
                        "[memory::jobs] ensure_reembed_backfill: enqueued chain for sig={sig}"
                    );
                }
                Err(e) => log::warn!(
                    "[memory::jobs] ensure_reembed_backfill: enqueue failed for sig={sig}: {e}"
                ),
            }
        }
        Ok(false) => {
            log::debug!(
                "[memory::jobs] ensure_reembed_backfill: sig={sig} fully covered; nothing to do"
            );
        }
        Err(e) => log::warn!(
            "[memory::jobs] ensure_reembed_backfill: coverage probe failed for sig={sig}: {e}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backfill_flag_roundtrip() {
        set_backfill_in_progress(false);
        assert!(!backfill_in_progress());

        set_backfill_in_progress(true);
        assert!(backfill_in_progress());

        set_backfill_in_progress(false);
        assert!(!backfill_in_progress());
    }
}
