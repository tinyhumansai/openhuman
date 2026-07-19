//! Memory-queue operations: backfill-progress signalling and the re-embed
//! backfill switch-path trigger.
//!
//! Split out of `mod.rs` so the module root stays export-focused. Public paths
//! are preserved via re-exports in [`super`], so callers keep using
//! `crate::openhuman::memory_queue::<fn>`.

/// Mark whether a re-embed backfill currently has pending work.
pub fn set_backfill_in_progress(v: bool) {
    tinycortex::memory::queue::set_backfill_in_progress(v);
}

/// True while a re-embed backfill chain still has rows to process. The
/// #1365 absence-reasoning consumer checks this before treating an empty
/// semantic-recall result as "no memory exists".
pub fn backfill_in_progress() -> bool {
    tinycortex::memory::queue::backfill_in_progress()
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
    let memory =
        crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone());
    let delegates = crate::openhuman::tinycortex::HostQueueDelegates::new(config.clone());
    if let Err(error) = tinycortex::memory::queue::ensure_reembed_backfill(&memory, &delegates) {
        log::warn!("[memory::jobs] ensure_reembed_backfill failed: {error:#}");
    }
}
