//! Memory-queue operations: backfill-progress signalling, the re-embed
//! backfill switch-path trigger, and the provider-change failed-job un-park.
//!
//! Split out of `mod.rs` so the module root stays export-focused. Public paths
//! are preserved via re-exports in [`super`], so callers keep using
//! `crate::openhuman::memory::queue::<fn>`.

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
    let memory = crate::openhuman::memory::tinycortex::memory_config_from(
        config,
        config.workspace_dir.clone(),
    );
    let delegates = crate::openhuman::memory::tinycortex::HostQueueDelegates::new(config.clone());
    if let Err(error) = tinycortex::memory::queue::ensure_reembed_backfill(&memory, &delegates) {
        log::warn!("[memory::jobs] ensure_reembed_backfill failed: {error:#}");
    }
}

/// #5324: un-park terminally-`failed` jobs after the user changes their
/// embedding provider or supplies a key.
///
/// The motivating case is `budget_exhausted`: once the managed embedding
/// budget is spent, every embed job fails as `Unrecoverable` and is parked
/// forever. `Unrecoverable` is meant to read as "cannot succeed *without user
/// action*", not "will never be retried" — so the moment the user takes that
/// action (points embeddings at local Ollama, or pastes a BYO key) the parked
/// jobs must get a fresh attempt budget instead of waiting for the user to
/// find the "Retry failed" button in Memory Tree settings.
///
/// Deliberately requeues **all** failed jobs, not just the budget-exhausted
/// ones: scoping by `failure_code` would need a new filtered query in the
/// vendored `tinycortex` crate, and the cost of the wider net is bounded — a
/// job that fails for an unrelated reason (dim mismatch, empty input) simply
/// fails once more and re-parks, and this only runs on an explicit user
/// config change, never on a timer or on login.
///
/// Non-fatal to the settings save by design, but NOT silent: on a store
/// failure this returns `Err` rather than a `0` that reads identically to
/// "nothing to requeue". The caller keeps the save successful and surfaces the
/// recovery failure in its RPC outcome, so a queue that stayed parked is never
/// presented to the user as remediated. `Ok(n)` is the number of jobs flipped
/// back to `ready` (`Ok(0)` = nothing was parked).
pub fn requeue_failed_after_provider_change(
    config: &crate::openhuman::config::Config,
) -> Result<u64, String> {
    // Entry record (see AGENTS.md "Debug logging"): state-transition op, so log
    // entry + every branch + outcome. Prefix matches this module's sibling
    // `ensure_reembed_backfill` (`[memory::jobs]`) — a stable, grep-friendly
    // domain prefix.
    log::debug!("[memory::jobs] provider change: evaluating parked failed jobs for requeue");
    match super::store::requeue_failed(config) {
        Ok(0) => {
            log::debug!("[memory::jobs] provider change: no failed jobs to requeue");
            Ok(0)
        }
        Ok(requeued) => {
            log::info!(
                "[memory::jobs] provider change: requeued {requeued} failed job(s) for a fresh attempt"
            );
            // Wake the worker pool so the un-parked jobs are picked up
            // promptly rather than at the next scheduled flush window.
            super::wake_workers();
            Ok(requeued)
        }
        Err(error) => {
            log::warn!("[memory::jobs] provider change: requeue_failed failed: {error:#}");
            Err(format!("{error:#}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;
    use crate::openhuman::memory::tree::health::{FailureCode, PipelineFailure};
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    /// Nothing parked ⇒ nothing to un-park. Must not error, and must not wake
    /// the worker pool for no reason.
    #[test]
    fn requeue_after_provider_change_is_zero_on_an_empty_queue() {
        let (_tmp, cfg) = test_config();
        assert_eq!(requeue_failed_after_provider_change(&cfg).unwrap(), 0);
    }

    /// The #5324 case: jobs parked as `budget_exhausted` (unrecoverable, so
    /// the periodic transient-requeue deliberately leaves them alone) must be
    /// flipped back to `ready` when the user changes their embedding provider.
    #[tokio::test]
    async fn requeue_after_provider_change_unparks_budget_exhausted_jobs() {
        use crate::openhuman::memory::queue::store;
        use crate::openhuman::memory::queue::types::{FlushStalePayload, JobStatus, NewJob};

        let (_tmp, cfg) = test_config();
        let new_job = NewJob::flush_stale(&FlushStalePayload::default(), "2026-08-05", 3).unwrap();
        let id = store::enqueue(&cfg, &new_job)
            .unwrap()
            .expect("enqueue job");
        let job = store::get_job(&cfg, &id).unwrap().expect("job exists");

        // Park it exactly the way an exhausted managed budget does.
        let failure = PipelineFailure::new(FailureCode::BudgetExhausted);
        assert!(
            failure.is_unrecoverable(),
            "precondition: parked, not retried"
        );
        store::mark_failed_typed(&cfg, &job, "Insufficient budget", Some(&failure)).unwrap();
        assert_eq!(
            store::count_by_status(&cfg, JobStatus::Failed).unwrap(),
            1,
            "precondition: the job is parked"
        );
        assert_eq!(
            store::count_failed_unrecoverable(&cfg).unwrap(),
            1,
            "precondition: parked as unrecoverable, so periodic retry skips it"
        );

        assert_eq!(requeue_failed_after_provider_change(&cfg).unwrap(), 1);

        assert_eq!(
            store::count_by_status(&cfg, JobStatus::Ready).unwrap(),
            1,
            "the job must be retryable again after the user fixes their provider"
        );
        assert_eq!(store::count_by_status(&cfg, JobStatus::Failed).unwrap(), 0);
    }

    /// Idempotent: calling it again once the queue is drained of failures is a
    /// no-op, so re-saving settings repeatedly cannot spam the worker pool.
    #[tokio::test]
    async fn requeue_after_provider_change_is_idempotent() {
        use crate::openhuman::memory::queue::store;
        use crate::openhuman::memory::queue::types::{FlushStalePayload, NewJob};

        let (_tmp, cfg) = test_config();
        let new_job = NewJob::flush_stale(&FlushStalePayload::default(), "2026-08-05", 3).unwrap();
        let id = store::enqueue(&cfg, &new_job)
            .unwrap()
            .expect("enqueue job");
        let job = store::get_job(&cfg, &id).unwrap().expect("job exists");
        store::mark_failed_typed(
            &cfg,
            &job,
            "Insufficient budget",
            Some(&PipelineFailure::new(FailureCode::BudgetExhausted)),
        )
        .unwrap();

        assert_eq!(requeue_failed_after_provider_change(&cfg).unwrap(), 1);
        assert_eq!(requeue_failed_after_provider_change(&cfg).unwrap(), 0);
    }

    /// CodeRabbit (#5324): a store failure must SURFACE as `Err`, not collapse
    /// into a `0` that reads identically to "nothing to requeue" and makes a
    /// still-parked queue look remediated.
    #[test]
    fn requeue_after_provider_change_surfaces_store_errors() {
        let tmp = TempDir::new().unwrap();
        // Point workspace_dir at a regular file, so the queue DB underneath it
        // cannot be opened (ENOTDIR). The failure must propagate to the caller.
        let as_file = tmp.path().join("workspace-is-a-file");
        std::fs::write(&as_file, b"not a directory").unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = as_file;

        let out = requeue_failed_after_provider_change(&cfg);
        assert!(
            out.is_err(),
            "a store failure must surface as Err, not a misleading Ok(0): {out:?}"
        );
    }
}
