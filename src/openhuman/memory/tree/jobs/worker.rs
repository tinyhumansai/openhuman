//! Worker pool: claims jobs from `mem_tree_jobs`, dispatches them through
//! [`handlers::handle_job`], and settles the row.
//!
//! Concurrency control for LLM-bound work is delegated to
//! [`crate::openhuman::scheduler_gate`] — its global single-slot
//! semaphore (`LlmPermit`) is the one source of truth across this
//! worker, voice cleanup, autocomplete, triage, and reflection. The
//! worker itself just calls `wait_for_capacity()`; non-LLM jobs
//! (`AppendBuffer`, `FlushStale`) run without acquiring a permit.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Result;
use tokio::sync::Notify;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::jobs::handlers;
use crate::openhuman::memory::tree::jobs::redact::scrub_for_log;
use crate::openhuman::memory::tree::jobs::store::{
    claim_next, mark_deferred, mark_done, mark_failed, recover_stale_locks,
    DEFAULT_LOCK_DURATION_MS,
};
use crate::openhuman::memory::tree::jobs::types::JobOutcome;

/// Number of concurrent job-worker tasks. Each worker claims one job
/// at a time via `claim_next` (atomic UPDATE under SQLite WAL with
/// `locked_until_ms` + status='running'), so multiple workers
/// parallelize independent jobs without double-claim risk.
///
/// On cloud backends, LLM-bound jobs drop the global LLM permit
/// after claim (see `run_once`) so all 4 workers can run cloud
/// extract/summarise calls in parallel.
///
/// On local backends, the single global LLM slot still serialises
/// Ollama calls for laptop-RAM safety. Note that `wait_for_capacity`
/// is acquired **before** `claim_next`, so non-LLM jobs (AppendBuffer,
/// FlushStale, TopicRoute) also block on the gate when an LLM job
/// holds the permit — they only run in parallel with each other while
/// no LLM job is in flight. Bumping `WORKER_COUNT` therefore helps
/// throughput most when local LLM calls are sparse.
const WORKER_COUNT: usize = 4;
const POLL_INTERVAL: Duration = Duration::from_secs(5);

static WORKER_NOTIFY: OnceLock<Arc<Notify>> = OnceLock::new();
static STARTED: std::sync::Once = std::sync::Once::new();

/// Notify any idle workers so they re-poll immediately instead of waiting
/// out [`POLL_INTERVAL`]. Cheap no-op before [`start`] has run.
pub fn wake_workers() {
    if let Some(notify) = WORKER_NOTIFY.get() {
        notify.notify_waiters();
    }
}

/// Start the worker pool + daily scheduler. Takes the full `Config` so
/// each spawned task sees the user's actual settings (LLM endpoints,
/// embedder model, timeouts) — not `Config::default()`. Without this,
/// workers fall back to inert/regex-only behavior regardless of what's
/// in `config.toml`, defeating the entire async pipeline.
///
/// Idempotent (`Once`-guarded) so repeat calls during bootstrap are
/// safe no-ops after the first.
pub fn start(config: Config) {
    STARTED.call_once(|| {
        let notify = WORKER_NOTIFY
            .get_or_init(|| Arc::new(Notify::new()))
            .clone();
        if let Err(err) = recover_stale_locks(&config) {
            log::warn!("[memory_tree::jobs] recover_stale_locks failed at startup: {err:#}");
        }

        for idx in 0..WORKER_COUNT {
            let notify = notify.clone();
            let cfg = config.clone();
            tokio::spawn(async move {
                loop {
                    match run_once(&cfg).await {
                        Ok(true) => continue,
                        Ok(false) => {
                            tokio::select! {
                                _ = notify.notified() => {}
                                _ = tokio::time::sleep(POLL_INTERVAL) => {}
                            }
                        }
                        Err(err) => {
                            // SQLite `BUSY` / `LOCKED` is transient write-lock
                            // contention (multiple workers + the scheduler +
                            // ingest producers all write the same DB). The
                            // configured `busy_timeout` already retries
                            // inside rusqlite; if we still see it here, the
                            // right answer is to back off and re-poll — not
                            // to page Sentry. The next loop iteration will
                            // try `claim_next` again and almost always
                            // succeed. See OPENHUMAN-TAURI-BP.
                            if is_sqlite_busy(&err) {
                                log::warn!(
                                    "[memory_tree::jobs] worker {idx} hit SQLite busy/locked, \
                                     backing off: {err:#}"
                                );
                            } else {
                                crate::core::observability::report_error(
                                    &err,
                                    "memory",
                                    "tree_jobs_worker",
                                    &[("worker_idx", &idx.to_string())],
                                );
                            }
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            });
        }

        super::scheduler::start(config);
    });
}

/// Claim and run a single job. Returns `true` when work was processed,
/// `false` when no eligible row was available.
pub async fn run_once(config: &Config) -> Result<bool> {
    // Cooperative throttle BEFORE `claim_next()`. Holding the DB claim
    // across an awaited `wait_for_capacity()` would let `Paused` mode
    // sit on the row past `DEFAULT_LOCK_DURATION_MS`, after which
    // `recover_stale_locks()` would requeue it for another worker to
    // pick up — duplicating side effects. Throttling here means
    // non-LLM jobs (AppendBuffer/FlushStale) also experience the same
    // gate delay, but that's fine: in Throttled mode the host is
    // already overloaded and a 30s breather between any DB-write batch
    // is welcome; in Paused mode the user has explicitly asked us to
    // stand down. Returns immediately in Aggressive/Normal so plugged-in
    // desktops with headroom pay zero cost.
    //
    // For LLM-bound jobs the returned `LlmPermit` reserves the global
    // single slot for the lifetime of `handle_job`. Non-LLM jobs
    // (`AppendBuffer`, `FlushStale`) drop the permit before the
    // handler runs so they don't block the slot.
    let gate_permit = crate::openhuman::scheduler_gate::wait_for_capacity().await;

    let Some(job) = claim_next(config, DEFAULT_LOCK_DURATION_MS)? else {
        return Ok(false);
    };

    let llm_permit = if job.kind.is_llm_bound() {
        // Local Ollama loads ~1.3 GB resident per concurrent call —
        // hold the gate to enforce process-wide single-slot RAM
        // safety. Cloud calls are bandwidth-bound, not RAM-bound:
        // drop the permit so multiple workers can run cloud
        // extract/summarise calls in parallel (the worker pool
        // itself, sized to `WORKER_COUNT`, is the upstream bound).
        match config.memory_tree.llm_backend {
            crate::openhuman::config::LlmBackend::Local => gate_permit,
            crate::openhuman::config::LlmBackend::Cloud => {
                drop(gate_permit);
                None
            }
        }
    } else {
        // Non-LLM jobs don't need the global slot; release it so an
        // LLM-bound caller waiting elsewhere in the process can run.
        drop(gate_permit);
        None
    };

    let result = handlers::handle_job(config, &job).await;
    drop(llm_permit);

    // A failed settle (`mark_done` / `mark_failed` / `mark_deferred` below)
    // can also return `SQLITE_BUSY`. The worker's outer `Err` arm in
    // `start` reclassifies those into a warn-log + backoff (no Sentry
    // report) via [`is_sqlite_busy`]. On a stale settle the row's
    // `locked_until_ms` eventually elapses and `recover_stale_locks`
    // requeues it, so dropping the error here is at-most a re-run.
    match result {
        Ok(JobOutcome::Done) => {
            log::debug!(
                "[memory_tree::jobs] done id={} kind={}",
                job.id,
                job.kind.as_str()
            );
            mark_done(config, &job)?;
        }
        Ok(JobOutcome::Defer { until_ms, reason }) => {
            // Defer is normal operation (transient blocker, e.g. rate
            // limit) — log at info, not warn — and do NOT count this
            // claim toward the failure-attempt budget. `mark_deferred`
            // reverts the bump applied by `claim_next` so the row's
            // attempts counter stays where it was before this claim.
            //
            // `reason` is handler-supplied free-form text and may
            // include upstream provider responses; scrub for log
            // emission while keeping the original in DB state.
            log::info!(
                "[memory_tree::jobs] deferred id={} kind={} until_ms={} reason={}",
                job.id,
                job.kind.as_str(),
                until_ms,
                scrub_for_log(&reason)
            );
            mark_deferred(config, &job, until_ms, &reason)?;
        }
        Err(err) => {
            // Preserve the full anyhow cause chain in the persisted
            // last_error so a reader of mem_tree_jobs can see the root
            // cause, not just the top-level message. The log line gets
            // the same chain after `scrub_for_log`, since anyhow chains
            // commonly embed upstream HTTP bodies / auth headers.
            let message = format!("{err:#}");
            log::warn!(
                "[memory_tree::jobs] job failed id={} kind={} err={}",
                job.id,
                job.kind.as_str(),
                scrub_for_log(&message)
            );
            mark_failed(config, &job, &message)?;
        }
    }

    Ok(true)
}

/// Classify whether an error from `run_once` is a transient SQLite
/// write-lock contention (`SQLITE_BUSY` or `SQLITE_LOCKED`).
///
/// The configured `busy_timeout` already absorbs short waits inside
/// rusqlite; this helper catches the residual case where the busy
/// handler exhausts and the error bubbles up. Treated as a soft signal:
/// the worker logs a warning and re-polls on the next loop iteration
/// rather than escalating to Sentry.
fn is_sqlite_busy(err: &anyhow::Error) -> bool {
    if let Some(rusqlite::Error::SqliteFailure(sqlite_err, _)) =
        err.downcast_ref::<rusqlite::Error>()
    {
        return matches!(
            sqlite_err.code,
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
        );
    }
    // Fallback for chained/wrapped errors: the rusqlite `Error` may sit
    // a few `context()` layers deep. anyhow's alternate `Display`
    // joins every cause with ": ", so the SQLite-rendered text is
    // searchable in the flattened chain. Match the two well-known
    // phrases SQLite emits for these codes.
    let msg = format!("{err:#}").to_ascii_lowercase();
    msg.contains("database is locked") || msg.contains("database table is locked")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Raw `rusqlite::Error::SqliteFailure` with the `DatabaseBusy` code
    /// is what surfaces when the `busy_timeout` is exhausted on a write.
    #[test]
    fn is_sqlite_busy_matches_database_busy_code() {
        let raw = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::DatabaseBusy,
                extended_code: 5, // SQLITE_BUSY
            },
            Some("database is locked".into()),
        );
        let err = anyhow::Error::from(raw);
        assert!(is_sqlite_busy(&err));
    }

    /// `SQLITE_LOCKED` is the per-table flavour (e.g. shared cache); same
    /// classification — transient, retry.
    #[test]
    fn is_sqlite_busy_matches_database_locked_code() {
        let raw = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::DatabaseLocked,
                extended_code: 6, // SQLITE_LOCKED
            },
            Some("database table is locked".into()),
        );
        let err = anyhow::Error::from(raw);
        assert!(is_sqlite_busy(&err));
    }

    /// When the rusqlite error is buried under `.context(...)` layers
    /// (as happens when `with_connection` wraps the closure result),
    /// the downcast still finds it. Regression guard: don't rely on
    /// matching the top-level error type.
    #[test]
    fn is_sqlite_busy_matches_through_context_layers() {
        let raw = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::DatabaseBusy,
                extended_code: 5,
            },
            Some("database is locked".into()),
        );
        let wrapped: anyhow::Error = anyhow::Error::from(raw)
            .context("Failed to claim next mem_tree_jobs row")
            .context("with_connection closure failed");
        assert!(is_sqlite_busy(&wrapped));
    }

    /// Fallback text-match: if the rusqlite error has been re-rendered
    /// into a plain `anyhow!` (no downcast available), the "database is
    /// locked" phrase still triggers the busy classification.
    #[test]
    fn is_sqlite_busy_text_fallback() {
        let err = anyhow::anyhow!("Failed to claim next mem_tree_jobs row: database is locked");
        assert!(is_sqlite_busy(&err));
    }

    /// Non-busy SQLite failures (e.g. UNIQUE constraint) must NOT be
    /// reclassified — those are real bugs worth reporting.
    #[test]
    fn is_sqlite_busy_does_not_match_constraint_violation() {
        let raw = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::ConstraintViolation,
                extended_code: 19,
            },
            Some("UNIQUE constraint failed: mem_tree_jobs.dedupe_key".into()),
        );
        let err = anyhow::Error::from(raw);
        assert!(!is_sqlite_busy(&err));
    }

    /// Generic non-SQLite errors must not be reclassified as busy.
    #[test]
    fn is_sqlite_busy_does_not_match_unrelated_errors() {
        let err = anyhow::anyhow!("upstream returned 500: internal server error");
        assert!(!is_sqlite_busy(&err));
    }
}
