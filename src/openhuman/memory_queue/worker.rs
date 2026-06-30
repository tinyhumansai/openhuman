//! Worker pool: claims jobs from `mem_tree_jobs`, dispatches them through
//! [`handlers::handle_job`], and settles the row.
//!
//! Concurrency control for LLM-bound work is delegated to
//! [`crate::openhuman::scheduler_gate`] — its global single-slot
//! semaphore (`LlmPermit`) is the one source of truth across this
//! worker, voice cleanup, autocomplete, triage, and reflection. The
//! worker itself just calls `wait_for_capacity()`; non-LLM jobs
//! (`AppendBuffer`, `FlushStale`) run without acquiring a permit.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Result;
use tokio::sync::Notify;

use crate::openhuman::config::Config;
use crate::openhuman::memory_queue::handlers;
use crate::openhuman::memory_queue::redact::scrub_for_log;
use crate::openhuman::memory_queue::store::{
    claim_next, claim_ready_extract_batch, mark_deferred, mark_done, mark_failed_typed,
    recover_stale_locks, release_running_locks, DEFAULT_LOCK_DURATION_MS,
};
use crate::openhuman::memory_queue::types::{Job, JobKind, JobOutcome};
use crate::openhuman::memory_tree::health::PipelineFailure;

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

/// Process-wide latch so a `SQLITE_CORRUPT` flood is reported to Sentry **once**,
/// not on every poll from every worker. Set on the first malformed-image
/// detection; cleared after a recovery attempt settles (quarantine+rebuild or a
/// quick_check that now passes) so a genuinely-new, later corruption can still
/// page once. Without this, 4 workers polling a wedged DB re-page ~1/sec
/// (Sentry TAURI-RUST-E93: 1,633 events in ~17 min from one host).
static CORRUPT_REPORTED: AtomicBool = AtomicBool::new(false);

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
            log::warn!("[memory::jobs] recover_stale_locks failed at startup: {err:#}");
        }

        // Release in-flight locks on graceful shutdown so a clean restart
        // re-claims the work immediately instead of waiting out the lease
        // (which surfaced as a stale-lock recovery warn on every launch).
        // Hard kills still fall back to lease-expiry recovery at startup
        // (bug-report-2026-05-26 I2).
        let shutdown_cfg = config.clone();
        crate::core::shutdown::register(move || {
            // NOTE: `shutdown::register` is bound `F: Fn() -> Fut`, so this
            // closure may be invoked more than once; each call must hand the
            // returned future its own owned `Config`. Moving `shutdown_cfg`
            // in directly is `E0507` (cannot move out of an `Fn` closure), so
            // the per-call clone is required, not redundant.
            let cfg = shutdown_cfg.clone();
            async move {
                match release_running_locks(&cfg) {
                    Ok(n) if n > 0 => {
                        log::info!(
                            "[memory::jobs] released {n} in-flight job lock(s) on graceful shutdown"
                        );
                    }
                    Ok(_) => {}
                    Err(err) => {
                        log::warn!(
                            "[memory::jobs] failed to release job locks on shutdown: {err:#}"
                        );
                    }
                }
            }
        });

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
                                    "[memory::jobs] worker {idx} hit SQLite busy/locked, \
                                     backing off 1s: {err:#}"
                                );
                                tokio::time::sleep(Duration::from_secs(1)).await;
                            } else if is_sqlite_io_transient(&err) {
                                // I/O errors (IOERR_TRUNCATE 1546, the `-shm` family
                                // 4618/4874/5386, IN_PAGE 8714, CANTOPEN 14) or circuit
                                // breaker open — transient
                                // filesystem / WAL condition. Back off 30 s and let the
                                // connection cache try a fresh open on next poll. These
                                // are NOT reported to Sentry (they are transient and were
                                // flooding ~19K events/4 days, see #2206).
                                log::warn!(
                                    "[memory::jobs] worker {idx} hit transient I/O error, \
                                     backing off 30s: {err:#}"
                                );
                                tokio::time::sleep(Duration::from_secs(30)).await;
                            } else if is_sqlite_disk_full(&err) {
                                // SQLITE_FULL (code 13): the host disk is full.
                                // A claim UPDATE cannot succeed until the user
                                // frees space — this is persistent, not
                                // transient, so re-polling every second and
                                // paging Sentry on each failure floods the
                                // dashboard (TAURI-RUST-4R8: ~95k events, one
                                // user) for a condition only the user can
                                // clear. Back off long and stay silent; the
                                // `ready` rows resume when space returns and
                                // `notify` still wakes us on new enqueues.
                                log::warn!(
                                    "[memory::jobs] worker {idx} hit SQLITE_FULL (disk full), \
                                     backing off 300s without reporting: {err:#}"
                                );
                                tokio::time::sleep(Duration::from_secs(300)).await;
                            } else if is_sqlite_corrupt(&err) {
                                // SQLITE_CORRUPT (code 11): the on-disk mem_tree
                                // image is malformed. Unlike busy/io-transient/
                                // disk-full, this NEVER clears on its own — the
                                // claim UPDATE fails forever, so re-polling every
                                // second and paging Sentry each time turns one
                                // unrecoverable file into a flood (TAURI-RUST-E93:
                                // 1,633 events in ~17 min, one host). Report once,
                                // drive quarantine+rebuild recovery (factored into
                                // `recover_corrupt_db_once` so it is unit-testable
                                // without spinning the live loop), then back off
                                // long so a failed recovery never re-floods.
                                // `notify` still wakes us on new enqueues once the
                                // rebuild succeeds.
                                recover_corrupt_db_once(idx, &err, &cfg);
                                tokio::time::sleep(Duration::from_secs(300)).await;
                            } else {
                                crate::core::observability::report_error(
                                    &err,
                                    "memory",
                                    "tree_jobs_worker",
                                    &[("worker_idx", &idx.to_string())],
                                );
                                tokio::time::sleep(Duration::from_secs(1)).await;
                            }
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
        let memory_uses_local = config.workload_uses_local("memory");
        log::trace!(
            "[memory::jobs] llm permit routing job_id={} kind={} memory_uses_local={}",
            job.id,
            job.kind.as_str(),
            memory_uses_local
        );
        if memory_uses_local {
            gate_permit
        } else {
            drop(gate_permit);
            None
        }
    } else {
        // Non-LLM jobs don't need the global slot; release it so an
        // LLM-bound caller waiting elsewhere in the process can run.
        drop(gate_permit);
        None
    };

    let mut jobs = vec![job];
    if jobs[0].kind == JobKind::ExtractChunk {
        let extra_limit = handlers::EXTRACT_EMBED_BATCH.saturating_sub(1);
        let mut extra = claim_ready_extract_batch(config, DEFAULT_LOCK_DURATION_MS, extra_limit)?;
        if !extra.is_empty() {
            log::debug!(
                "[memory::jobs] running extract batch count={}",
                extra.len() + 1
            );
            jobs.append(&mut extra);
        }
    }

    let results = if jobs.len() > 1 && jobs[0].kind == JobKind::ExtractChunk {
        handlers::handle_extract_batch(config, &jobs).await?
    } else {
        let job = jobs
            .pop()
            .expect("worker has exactly one claimed job in non-batch path");
        let result = handlers::handle_job(config, &job).await;
        vec![(job, result)]
    };
    drop(llm_permit);

    // A failed settle (`mark_done` / `mark_failed` / `mark_deferred` below)
    // can also return `SQLITE_BUSY`. The worker's outer `Err` arm in
    // `start` reclassifies those into a warn-log + backoff (no Sentry
    // report) via [`is_sqlite_busy`]. On a stale settle the row's
    // `locked_until_ms` eventually elapses and `recover_stale_locks`
    // requeues it, so dropping the error here is at-most a re-run.
    for (job, result) in results {
        settle_job(config, &job, result)?;
    }

    Ok(true)
}

fn settle_job(config: &Config, job: &Job, result: Result<JobOutcome>) -> Result<()> {
    match result {
        Ok(JobOutcome::Done) => {
            log::debug!(
                "[memory::jobs] done id={} kind={}",
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
                "[memory::jobs] deferred id={} kind={} until_ms={} reason={}",
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
            // #002: if the error chain carries a typed `PipelineFailure`
            // (attached at the embed/extract boundary), pass it through so
            // `mark_failed_typed` can fail fast on unrecoverable causes
            // (budget/auth/dim) instead of burning the retry budget, and
            // persist the typed reason for the status/doctor surface.
            let typed = err.downcast_ref::<PipelineFailure>();
            log::warn!(
                "[memory::jobs] job failed id={} kind={} reason={:?} err={}",
                job.id,
                job.kind.as_str(),
                typed.map(|f| f.code.as_str()),
                scrub_for_log(&message)
            );
            mark_failed_typed(config, &job, &message, typed)?;
        }
    }
    Ok(())
}

/// Classify whether an error is a transient I/O failure that should be
/// silently backed off without a Sentry report (#2206).
///
/// Covers:
/// - `SQLITE_IOERR_TRUNCATE` (1546): WAL truncation failed — usually a
///   transient filesystem hiccup.
/// - WAL `-shm` family — `SHMOPEN` (4618, the macOS cold-start failure),
///   `SHMSIZE` (4874), `SHMMAP` (5386): shared-memory side-file temporarily
///   unavailable. (4874 is SHMSIZE, not SHMMAP — the real SHMMAP is 5386.)
/// - `SQLITE_IOERR_IN_PAGE` (8714): mmap-page I/O fault.
/// - `SQLITE_CANTOPEN` / `CannotOpen` (14): DB file temporarily inaccessible.
/// - Text fallback: circuit breaker message, or rusqlite phrases that don't
///   downcast cleanly after multiple `.context()` layers.
fn is_sqlite_io_transient(err: &anyhow::Error) -> bool {
    if let Some(rusqlite::Error::SqliteFailure(f, _)) = err.downcast_ref::<rusqlite::Error>() {
        // 14 CANTOPEN, 1546 TRUNCATE, 4618 SHMOPEN, 4874 SHMSIZE, 5386 SHMMAP,
        // 8714 IN_PAGE — the WAL `-shm` cold-start family (4874 is SHMSIZE, not
        // SHMMAP; the real SHMMAP is 5386).
        if matches!(f.extended_code, 14 | 1546 | 4618 | 4874 | 5386 | 8714) {
            return true;
        }
        if f.code == rusqlite::ErrorCode::CannotOpen {
            return true;
        }
    }
    // Text fallback for errors wrapped under `.context()` layers or
    // emitted as plain `anyhow!` strings (e.g. circuit breaker message).
    let msg = format!("{err:#}").to_ascii_lowercase();
    msg.contains("circuit breaker open")
        || msg.contains("disk i/o error")
        || msg.contains("unable to open database file")
        || msg.contains("xshmmap")
        || msg.contains("truncate file")
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

/// Classify whether an error from `claim_next` is a `SQLITE_FULL` disk-full
/// condition (primary code `DiskFull`, extended 13).
///
/// Unlike `SQLITE_BUSY`/`LOCKED` or the transient I/O family, a full disk is a
/// **persistent** host condition: the claim `UPDATE` cannot succeed until the
/// user frees space. Re-polling every second and paging Sentry on each failure
/// turns one unrecoverable condition into a flood (Sentry TAURI-RUST-4R8:
/// ~95k events from a single user). The worker backs off long and stays
/// silent; the rows stay `ready` and resume when space returns.
///
/// Matching on the `DiskFull` error code is rusqlite-version-stable. The text
/// fallback covers the case where the error was flattened to a plain `anyhow!`
/// string across `.context()` layers — rusqlite renders `SQLITE_FULL` as
/// `"database or disk is full: Error code 13: Insertion failed because
/// database is full"`, so anchor on either canonical fragment.
fn is_sqlite_disk_full(err: &anyhow::Error) -> bool {
    if let Some(rusqlite::Error::SqliteFailure(sqlite_err, _)) =
        err.downcast_ref::<rusqlite::Error>()
    {
        if sqlite_err.code == rusqlite::ErrorCode::DiskFull {
            return true;
        }
    }
    let msg = format!("{err:#}").to_ascii_lowercase();
    msg.contains("database or disk is full")
        || msg.contains("insertion failed because database is full")
}

/// Classify whether an error from `claim_next` is a `SQLITE_CORRUPT` malformed-
/// image condition (primary code `DatabaseCorrupt`, code 11) or the closely-
/// related `NotADatabase` (code 26 — the header itself is unreadable).
///
/// Unlike `SQLITE_BUSY`/`LOCKED`, the transient I/O family, or `SQLITE_FULL`,
/// a malformed image is **persistent on-disk damage**: the claim `UPDATE` can
/// never succeed, so re-polling every second and paging Sentry on each failure
/// turns one corrupt file into an infinite flood (Sentry TAURI-RUST-E93:
/// ~1.6k events in ~17 min from a single host). The worker reports once, drives
/// a quarantine+rebuild recovery (`recover_corrupt_db`), and backs off long.
///
/// Matching on the error code is rusqlite-version-stable. The text fallback
/// covers the case where the rusqlite error was flattened to a plain `anyhow!`
/// string across `.context()` layers — SQLite renders these as "database disk
/// image is malformed" (code 11) and "file is not a database" (code 26).
fn is_sqlite_corrupt(err: &anyhow::Error) -> bool {
    if let Some(rusqlite::Error::SqliteFailure(sqlite_err, _)) =
        err.downcast_ref::<rusqlite::Error>()
    {
        if matches!(
            sqlite_err.code,
            rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
        ) {
            return true;
        }
    }
    let msg = format!("{err:#}").to_ascii_lowercase();
    msg.contains("database disk image is malformed") || msg.contains("file is not a database")
}

/// Handle a confirmed `SQLITE_CORRUPT` failure from the worker loop: report it
/// to Sentry **once** (process-wide [`CORRUPT_REPORTED`] latch, not per-poll
/// across the workers) and drive the quarantine+rebuild recovery in
/// [`recover_corrupt_db`](crate::openhuman::memory_store::chunks::store::recover_corrupt_db).
///
/// Factored out of [`start`]'s error arm so the report-once + recovery decision
/// logic is unit-testable without spinning the live worker loop. The caller
/// applies the long backoff after this returns.
fn recover_corrupt_db_once(idx: usize, err: &anyhow::Error, config: &Config) {
    if !CORRUPT_REPORTED.swap(true, Ordering::Relaxed) {
        crate::core::observability::report_error(
            err,
            "memory",
            "tree_jobs_worker_corrupt",
            &[("worker_idx", &idx.to_string())],
        );
    }
    log::error!(
        "[memory::jobs] worker {idx} hit SQLITE_CORRUPT (malformed DB image), \
         attempting quarantine + rebuild recovery: {err:#}"
    );
    match crate::openhuman::memory_store::chunks::store::recover_corrupt_db(config) {
        Ok(true) => {
            log::warn!(
                "[memory::jobs] worker {idx} quarantined corrupt mem_tree DB and rebuilt \
                 empty schema; queue will resume"
            );
            // Recovery settled — allow a future, genuinely-new corruption to
            // page once.
            CORRUPT_REPORTED.store(false, Ordering::Relaxed);
        }
        Ok(false) => {
            log::info!(
                "[memory::jobs] worker {idx} corruption recovery: quick_check now passes, \
                 no quarantine needed"
            );
            CORRUPT_REPORTED.store(false, Ordering::Relaxed);
        }
        Err(rec_err) => {
            log::error!(
                "[memory::jobs] worker {idx} corruption recovery FAILED, retrying after \
                 backoff: {rec_err:#}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory_queue::store::{count_by_status, enqueue, get_job};
    use crate::openhuman::memory_queue::types::{
        FlushStalePayload, JobKind, JobStatus, NewJob, ReembedBackfillPayload,
    };
    use crate::openhuman::memory_store::chunks::store::{
        tree_active_signature, upsert_chunks, upsert_staged_chunks_tx, with_connection,
    };
    use crate::openhuman::memory_store::chunks::types::{
        chunk_id, Chunk, Metadata, SourceKind, SourceRef,
    };
    use crate::openhuman::memory_store::content as content_store;
    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

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

    // ── is_sqlite_io_transient tests (#2206) ─────────────────────────────

    /// SQLITE_IOERR_TRUNCATE (extended code 1546) must be classified as
    /// transient so the worker backs off without hitting Sentry.
    #[test]
    fn is_sqlite_io_transient_matches_ioerr_truncate() {
        let raw = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::SystemIoFailure,
                extended_code: 1546, // SQLITE_IOERR_TRUNCATE
            },
            Some("disk I/O error".into()),
        );
        assert!(is_sqlite_io_transient(&anyhow::Error::from(raw)));
    }

    /// The WAL `-shm` family must classify as transient via the NUMERIC arm
    /// (the message deliberately avoids the text-fallback phrases). 4618
    /// SHMOPEN is the macOS cold-start failure; 4874 is SHMSIZE; 5386 is the
    /// real SHMMAP; 8714 is IN_PAGE.
    #[test]
    fn is_sqlite_io_transient_matches_shm_family() {
        for ext in [4618, 4874, 5386, 8714] {
            let raw = rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: rusqlite::ErrorCode::SystemIoFailure,
                    extended_code: ext,
                },
                Some("sqlite extended io failure".into()),
            );
            assert!(
                is_sqlite_io_transient(&anyhow::Error::from(raw)),
                "extended_code {ext} must classify as transient (numeric arm)"
            );
        }
    }

    /// SQLITE_CANTOPEN (code CannotOpen, extended code 14) must be
    /// classified as transient — temporary inability to open the file.
    #[test]
    fn is_sqlite_io_transient_matches_cantopen() {
        let raw = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::CannotOpen,
                extended_code: 14, // SQLITE_CANTOPEN
            },
            Some("unable to open database file".into()),
        );
        assert!(is_sqlite_io_transient(&anyhow::Error::from(raw)));
    }

    /// The circuit breaker error message produced by `get_or_init_connection`
    /// must be classified as transient via the text fallback.
    #[test]
    fn is_sqlite_io_transient_text_fallback() {
        let err = anyhow::anyhow!("memory_tree_db circuit breaker open: too many init failures");
        assert!(is_sqlite_io_transient(&err));
    }

    /// UNIQUE constraint violation must NOT be reclassified as a transient
    /// I/O error — those are genuine bugs.
    #[test]
    fn is_sqlite_io_transient_negative_constraint_violation() {
        let raw = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::ConstraintViolation,
                extended_code: 19,
            },
            Some("UNIQUE constraint failed: mem_tree_jobs.dedupe_key".into()),
        );
        assert!(!is_sqlite_io_transient(&anyhow::Error::from(raw)));
    }

    // ── is_sqlite_disk_full tests (#3909 / Sentry TAURI-RUST-4R8) ─────────

    /// `SQLITE_FULL` (primary code `DiskFull`, extended 13) is the disk-full
    /// signal from `claim_next`; it must classify so the worker backs off
    /// long instead of paging Sentry every second.
    #[test]
    fn is_sqlite_disk_full_matches_disk_full_code() {
        let raw = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::DiskFull,
                extended_code: 13,
            },
            Some("database or disk is full".into()),
        );
        assert!(is_sqlite_disk_full(&anyhow::Error::from(raw)));
    }

    /// The rusqlite error sits a few `.context()` layers deep when it bubbles
    /// out of `claim_next` → `with_connection`; the downcast must still find
    /// the `DiskFull` code.
    #[test]
    fn is_sqlite_disk_full_matches_through_context_layers() {
        let raw = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::DiskFull,
                extended_code: 13,
            },
            Some("database or disk is full".into()),
        );
        let wrapped = anyhow::Error::from(raw)
            .context("Failed to claim next mem_tree_jobs row")
            .context("with_connection closure failed");
        assert!(is_sqlite_disk_full(&wrapped));
    }

    /// Text fallback: the exact flattened Sentry string (TAURI-RUST-4R8) is
    /// classified even when no rusqlite error is available to downcast (the
    /// canonical phrase is mid-string, not a suffix).
    #[test]
    fn is_sqlite_disk_full_text_fallback() {
        let err = anyhow::anyhow!(
            "Failed to claim next mem_tree_jobs row: database or disk is full: \
             Error code 13: Insertion failed because database is full"
        );
        assert!(is_sqlite_disk_full(&err));
    }

    /// Busy/locked, constraint violations, and unrelated errors must NOT be
    /// swallowed as disk-full — those still warrant their own handling /
    /// Sentry escalation.
    #[test]
    fn is_sqlite_disk_full_does_not_match_other_errors() {
        let busy = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::DatabaseBusy,
                extended_code: 5,
            },
            Some("database is locked".into()),
        );
        assert!(!is_sqlite_disk_full(&anyhow::Error::from(busy)));

        let constraint = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::ConstraintViolation,
                extended_code: 19,
            },
            Some("UNIQUE constraint failed: mem_tree_jobs.dedupe_key".into()),
        );
        assert!(!is_sqlite_disk_full(&anyhow::Error::from(constraint)));

        assert!(!is_sqlite_disk_full(&anyhow::anyhow!(
            "upstream returned 500: internal server error"
        )));
    }

    // ── is_sqlite_corrupt tests (#4048 / Sentry TAURI-RUST-E93) ──────────────

    /// `SQLITE_CORRUPT` (primary code `DatabaseCorrupt`, code 11) is the
    /// malformed-image signal from `claim_next`; it must classify so the worker
    /// quarantines + rebuilds instead of paging Sentry every second.
    #[test]
    fn is_sqlite_corrupt_matches_database_corrupt_code() {
        let raw = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::DatabaseCorrupt,
                extended_code: 11,
            },
            Some("database disk image is malformed".into()),
        );
        assert!(is_sqlite_corrupt(&anyhow::Error::from(raw)));
    }

    /// `SQLITE_NOTADB` (code `NotADatabase`, 26 — header unreadable) is the
    /// same broad on-disk-damage class and must classify too.
    #[test]
    fn is_sqlite_corrupt_matches_not_a_database_code() {
        let raw = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::NotADatabase,
                extended_code: 26,
            },
            Some("file is not a database".into()),
        );
        assert!(is_sqlite_corrupt(&anyhow::Error::from(raw)));
    }

    /// The rusqlite error sits a few `.context()` layers deep when it bubbles
    /// out of `claim_next` → `with_connection`; the downcast must still find
    /// the `DatabaseCorrupt` code.
    #[test]
    fn is_sqlite_corrupt_matches_through_context_layers() {
        let raw = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::DatabaseCorrupt,
                extended_code: 11,
            },
            Some("database disk image is malformed".into()),
        );
        let wrapped = anyhow::Error::from(raw)
            .context("Failed to claim next mem_tree_jobs row")
            .context("with_connection closure failed");
        assert!(is_sqlite_corrupt(&wrapped));
    }

    /// Text fallback: the exact flattened Sentry string (TAURI-RUST-E93) must
    /// classify even when no rusqlite error is available to downcast.
    #[test]
    fn is_sqlite_corrupt_text_fallback() {
        let err = anyhow::anyhow!(
            "Failed to claim next mem_tree_jobs row: database disk image is malformed: \
             Error code 11: The database disk image is malformed"
        );
        assert!(is_sqlite_corrupt(&err));
    }

    /// Busy/locked, disk-full, constraint violations, and unrelated errors must
    /// NOT be swallowed as corruption — quarantining on those would destroy a
    /// perfectly good DB.
    #[test]
    fn is_sqlite_corrupt_does_not_match_other_errors() {
        let busy = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::DatabaseBusy,
                extended_code: 5,
            },
            Some("database is locked".into()),
        );
        assert!(!is_sqlite_corrupt(&anyhow::Error::from(busy)));

        let disk_full = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::DiskFull,
                extended_code: 13,
            },
            Some("database or disk is full".into()),
        );
        assert!(!is_sqlite_corrupt(&anyhow::Error::from(disk_full)));

        let constraint = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::ConstraintViolation,
                extended_code: 19,
            },
            Some("UNIQUE constraint failed: mem_tree_jobs.dedupe_key".into()),
        );
        assert!(!is_sqlite_corrupt(&anyhow::Error::from(constraint)));

        assert!(!is_sqlite_corrupt(&anyhow::anyhow!(
            "upstream returned 500: internal server error"
        )));
    }

    /// The worker's corruption arm must quarantine a malformed image and rebuild
    /// an empty, queryable schema so the queue resumes — exercising the
    /// report-once + recover path the live loop runs.
    #[tokio::test]
    async fn recover_corrupt_db_once_quarantines_and_rebuilds() {
        let (_tmp, cfg) = test_config();
        // Lay down a malformed `chunks.db` (garbage header) at the canonical path.
        let db_path = cfg.workspace_dir.join("memory_tree").join("chunks.db");
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        std::fs::write(&db_path, b"not a sqlite database, just garbage bytes").unwrap();

        let err = anyhow::anyhow!(
            "Failed to claim next mem_tree_jobs row: database disk image is malformed"
        );
        recover_corrupt_db_once(0, &err, &cfg);

        // Corrupt bytes are preserved alongside (never silently dropped) ...
        let quarantined = std::fs::read_dir(db_path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .contains("chunks.db.corrupt-")
            });
        assert!(
            quarantined,
            "corrupt image must be quarantined, not deleted"
        );

        // ... and the rebuilt queue DB is healthy and empty.
        let processed = run_once(&cfg).await.unwrap();
        assert!(!processed, "rebuilt queue starts empty");
    }

    #[tokio::test]
    async fn wake_workers_is_noop_before_start() {
        wake_workers();
    }

    #[tokio::test]
    async fn run_once_returns_false_when_queue_is_empty() {
        let (_tmp, cfg) = test_config();
        let processed = run_once(&cfg).await.unwrap();
        assert!(!processed);
    }

    #[tokio::test]
    async fn run_once_claims_and_completes_a_flush_stale_job() {
        let (_tmp, cfg) = test_config();
        let new_job = NewJob::flush_stale(&FlushStalePayload::default(), "2026-05-24", 3).unwrap();
        let id = enqueue(&cfg, &new_job).unwrap().expect("enqueue job");

        let processed = run_once(&cfg).await.unwrap();
        assert!(processed);

        let job = get_job(&cfg, &id).unwrap().expect("job should still exist");
        assert_eq!(job.kind.as_str(), "flush_stale");
        assert_eq!(job.status, JobStatus::Done);
        assert_eq!(count_by_status(&cfg, JobStatus::Done).unwrap(), 1);
        assert!(job.completed_at_ms.is_some());
        assert!(job.locked_until_ms.is_none());
    }

    #[tokio::test]
    async fn run_once_reschedules_reembed_backfill_jobs_that_defer() {
        let (_tmp, mut cfg) = test_config();
        // Deliberate "none" opt-out → InertEmbedder (zero vectors, no network)
        // so the backfill has work and Defers; this test pins the worker's
        // defer-reschedule path, not embed quality.
        cfg.embeddings_provider = Some("none".to_string());
        let ts = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let chunk = Chunk {
            id: chunk_id(SourceKind::Chat, "slack:#eng", 0, "reembed-worker-seed"),
            content: "memory content about the phoenix migration project".into(),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: "slack:#eng".into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: Some(SourceRef::new("slack://x")),
                path_scope: None,
            },
            token_count: 12,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        };
        upsert_chunks(&cfg, &[chunk.clone()]).unwrap();
        let content_root = cfg.memory_tree_content_root();
        std::fs::create_dir_all(&content_root).unwrap();
        let staged = content_store::stage_chunks(&content_root, &[chunk]).unwrap();
        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            upsert_staged_chunks_tx(&tx, &staged)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();

        let signature = tree_active_signature(&cfg);
        let new_job = NewJob::reembed_backfill(&ReembedBackfillPayload {
            signature: signature.clone(),
        })
        .unwrap();
        let id = enqueue(&cfg, &new_job)
            .unwrap()
            .expect("enqueue backfill job");

        let processed = run_once(&cfg).await.unwrap();
        assert!(processed);

        let job = get_job(&cfg, &id).unwrap().expect("job should still exist");
        assert_eq!(job.kind, JobKind::ReembedBackfill);
        assert_eq!(job.status, JobStatus::Ready);
        assert_eq!(
            job.attempts, 0,
            "defer should revert the claim attempt bump"
        );
        assert!(job.started_at_ms.is_none());
        assert!(job.locked_until_ms.is_none());
        assert!(job.completed_at_ms.is_none());
        assert!(
            job.available_at_ms > Utc::now().timestamp_millis(),
            "deferred job should be rescheduled into the future"
        );
        assert!(
            job.last_error
                .as_deref()
                .unwrap_or("")
                .contains("re-embed backfill"),
            "defer reason should be recorded for visibility"
        );
        assert_eq!(count_by_status(&cfg, JobStatus::Ready).unwrap(), 1);
    }
}
