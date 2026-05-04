//! SQLite persistence for the memory-tree job queue.
//!
//! Producers call [`enqueue`] inside their own writes (or with a fresh tx)
//! to atomically commit the side-effect plus its follow-up job. The worker
//! pool calls [`claim_next`] to lease a job, [`mark_done`] / [`mark_failed`]
//! to settle it, and [`recover_stale_locks`] on startup to flip rows whose
//! `locked_until_ms` expired without a settle.
//!
//! Concurrency:
//! - The dedupe key is enforced by a partial `UNIQUE` index that only
//!   covers `status IN ('ready', 'running')`. Producers use `INSERT OR
//!   IGNORE` so a duplicate enqueue while a job is in flight or queued is
//!   a silent no-op; a duplicate enqueue after the first completes is
//!   accepted and creates a fresh row.
//! - `claim_next` is one statement: `UPDATE … WHERE id = (SELECT … LIMIT 1)
//!   RETURNING …`. SQLite serialises writes, so no two workers can claim
//!   the same row.

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use uuid::Uuid;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::jobs::types::{Job, JobKind, JobStatus, NewJob};
use crate::openhuman::memory::tree::store::with_connection;

/// Default visibility lock — a worker that crashes mid-job will have its
/// row recovered after this window. 5 min is comfortably larger than any
/// expected single-job runtime (LLM extract or summarise) without leaving
/// real failures stuck for hours.
pub const DEFAULT_LOCK_DURATION_MS: i64 = 5 * 60 * 1_000;

/// Backoff math for retry. Returns `now + min(base * 2^attempts, cap)`.
const RETRY_BASE_MS: i64 = 60 * 1_000;
const RETRY_CAP_MS: i64 = 60 * 60 * 1_000;
const DEFAULT_MAX_ATTEMPTS: u32 = 5;

/// Enqueue one job. Idempotent on `dedupe_key` while another active row
/// (status `ready`/`running`) shares it. Returns `Some(id)` if the row
/// was inserted, `None` if a duplicate was suppressed.
pub fn enqueue(config: &Config, job: &NewJob) -> Result<Option<String>> {
    with_connection(config, |conn| enqueue_conn(conn, job))
}

/// Enqueue inside a caller-owned transaction. Use this when the producer
/// is already mid-tx (e.g. `ingest::persist` writing chunks + jobs in one
/// commit) so the queue insert lands atomically with the side-effect.
/// `Transaction` derefs to `Connection`, so callers just pass `&tx`.
pub fn enqueue_tx(tx: &Transaction<'_>, job: &NewJob) -> Result<Option<String>> {
    enqueue_conn(tx, job)
}

pub(crate) fn enqueue_conn(conn: &Connection, job: &NewJob) -> Result<Option<String>> {
    let id = format!("job:{}", Uuid::new_v4());
    let now_ms = Utc::now().timestamp_millis();
    let available_at = job.available_at_ms.unwrap_or(now_ms);
    let max_attempts = job.max_attempts.unwrap_or(DEFAULT_MAX_ATTEMPTS) as i64;

    let inserted = conn.execute(
        "INSERT OR IGNORE INTO mem_tree_jobs (
            id, kind, payload_json, dedupe_key, status, attempts, max_attempts,
            available_at_ms, locked_until_ms, last_error,
            created_at_ms, started_at_ms, completed_at_ms
        ) VALUES (?1, ?2, ?3, ?4, 'ready', 0, ?5, ?6, NULL, NULL, ?7, NULL, NULL)",
        params![
            id,
            job.kind.as_str(),
            job.payload_json,
            job.dedupe_key,
            max_attempts,
            available_at,
            now_ms,
        ],
    )?;

    if inserted == 0 {
        log::debug!(
            "[memory_tree::jobs] enqueue suppressed by dedupe kind={} key={:?}",
            job.kind.as_str(),
            job.dedupe_key
        );
        return Ok(None);
    }
    log::debug!(
        "[memory_tree::jobs] enqueued id={} kind={} avail_at_ms={} dedupe={:?}",
        id,
        job.kind.as_str(),
        available_at,
        job.dedupe_key
    );
    Ok(Some(id))
}

/// Atomically claim the next ready job whose `available_at_ms` has come
/// due. Sets `status=running`, bumps `attempts`, stamps `started_at_ms`
/// and `locked_until_ms`. Returns `None` when the queue is empty / not
/// yet due.
pub fn claim_next(config: &Config, lock_duration_ms: i64) -> Result<Option<Job>> {
    with_connection(config, |conn| {
        let now_ms = Utc::now().timestamp_millis();
        let lock_until = now_ms.saturating_add(lock_duration_ms);

        let row = conn
            .query_row(
                "UPDATE mem_tree_jobs
                    SET status = 'running',
                        attempts = attempts + 1,
                        started_at_ms = ?1,
                        locked_until_ms = ?2,
                        last_error = NULL
                  WHERE id = (
                      SELECT id FROM mem_tree_jobs
                       WHERE status = 'ready'
                         AND available_at_ms <= ?1
                       ORDER BY available_at_ms ASC
                       LIMIT 1
                  )
              RETURNING id, kind, payload_json, dedupe_key, status, attempts,
                        max_attempts, available_at_ms, locked_until_ms, last_error,
                        created_at_ms, started_at_ms, completed_at_ms",
                params![now_ms, lock_until],
                row_to_job,
            )
            .optional()
            .context("Failed to claim next mem_tree_jobs row")?;
        if let Some(j) = &row {
            log::debug!(
                "[memory_tree::jobs] claimed id={} kind={} attempt={}/{}",
                j.id,
                j.kind.as_str(),
                j.attempts,
                j.max_attempts
            );
        }
        Ok(row)
    })
}

/// Mark a claimed job as `done`. Clears the lock and stamps `completed_at_ms`.
///
/// The UPDATE is gated on `attempts` and `started_at_ms` matching the values
/// in `job` (the snapshot returned by [`claim_next`]). If the lease expired
/// and another worker re-claimed the row, `rows_affected` will be 0 — the
/// stale worker's settlement is a silent no-op rather than clobbering the new
/// lessee's state.
pub fn mark_done(config: &Config, job: &Job) -> Result<()> {
    let job_id = &job.id;
    let claim_attempts = job.attempts as i64;
    let claim_started_at = job.started_at_ms;
    with_connection(config, |conn| {
        let now_ms = Utc::now().timestamp_millis();
        let n = conn.execute(
            "UPDATE mem_tree_jobs
                SET status = 'done',
                    completed_at_ms = ?1,
                    locked_until_ms = NULL,
                    last_error = NULL
              WHERE id = ?2
                AND attempts = ?3
                AND started_at_ms IS ?4",
            params![now_ms, job_id, claim_attempts, claim_started_at],
        )?;
        if n == 0 {
            // Either the job row was deleted (shouldn't happen) or the lease
            // expired and a second worker re-claimed the row. Log and move on —
            // this is a known race outcome, not a bug in the current worker.
            log::warn!(
                "[memory_tree::jobs] mark_done id={job_id} was a no-op \
                 (stale lease: attempts={claim_attempts} started_at_ms={claim_started_at:?})"
            );
        }
        Ok(())
    })
}

/// Settle a failed job. If `attempts < max_attempts`, the row goes back
/// to `ready` with an exponential-backoff `available_at_ms`. Otherwise
/// it terminates as `failed`. Either way `last_error` is recorded.
///
/// Like [`mark_done`], the UPDATE is gated on the claim-token
/// (`attempts` + `started_at_ms`) so a stale worker's failure settlement
/// cannot clobber an active lessee's row — rows_affected == 0 is a silent
/// no-op.
pub fn mark_failed(config: &Config, job: &Job, error: &str) -> Result<()> {
    let job_id = &job.id;
    let attempts = job.attempts as i64;
    let max_attempts = job.max_attempts as i64;
    let claim_started_at = job.started_at_ms;
    with_connection(config, |conn| {
        let now_ms = Utc::now().timestamp_millis();

        if attempts >= max_attempts {
            log::warn!(
                "[memory_tree::jobs] terminal failure id={job_id} \
                 attempts={attempts}/{max_attempts} err={error}"
            );
            let n = conn.execute(
                "UPDATE mem_tree_jobs
                    SET status = 'failed',
                        completed_at_ms = ?1,
                        locked_until_ms = NULL,
                        last_error = ?2
                  WHERE id = ?3
                    AND attempts = ?4
                    AND started_at_ms IS ?5",
                params![now_ms, error, job_id, attempts, claim_started_at],
            )?;
            if n == 0 {
                log::warn!(
                    "[memory_tree::jobs] mark_failed(terminal) id={job_id} was a no-op \
                     (stale lease: attempts={attempts} started_at_ms={claim_started_at:?})"
                );
            }
        } else {
            let backoff = backoff_ms(attempts as u32);
            let next_at = now_ms.saturating_add(backoff);
            log::info!(
                "[memory_tree::jobs] retry id={job_id} attempt={attempts}/{max_attempts} \
                 next_at_ms={next_at} err={error}"
            );
            let n = conn.execute(
                "UPDATE mem_tree_jobs
                    SET status = 'ready',
                        available_at_ms = ?1,
                        locked_until_ms = NULL,
                        last_error = ?2
                  WHERE id = ?3
                    AND attempts = ?4
                    AND started_at_ms IS ?5",
                params![next_at, error, job_id, attempts, claim_started_at],
            )?;
            if n == 0 {
                log::warn!(
                    "[memory_tree::jobs] mark_failed(retry) id={job_id} was a no-op \
                     (stale lease: attempts={attempts} started_at_ms={claim_started_at:?})"
                );
            }
        }
        Ok(())
    })
}

/// Flip any `running` row whose `locked_until_ms` has expired back to
/// `ready`. Called once at worker startup so a process crash mid-job
/// doesn't leave work stranded. Returns the number of rows recovered.
pub fn recover_stale_locks(config: &Config) -> Result<usize> {
    with_connection(config, |conn| {
        let now_ms = Utc::now().timestamp_millis();
        let n = conn.execute(
            "UPDATE mem_tree_jobs
                SET status = 'ready',
                    last_error = COALESCE(last_error, 'recovered_from_stale_lock')
              WHERE status = 'running'
                AND locked_until_ms IS NOT NULL
                AND locked_until_ms < ?1",
            params![now_ms],
        )?;
        if n > 0 {
            log::warn!("[memory_tree::jobs] recovered {n} stale-locked job(s) at startup");
        }
        Ok(n)
    })
}

/// Quick count helper for tests / diagnostics.
pub fn count_by_status(config: &Config, status: JobStatus) -> Result<u64> {
    with_connection(config, |conn| {
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM mem_tree_jobs WHERE status = ?1",
            params![status.as_str()],
            |r| r.get(0),
        )?;
        Ok(n.max(0) as u64)
    })
}

/// Total count regardless of status — handy for assertions.
pub fn count_total(config: &Config) -> Result<u64> {
    with_connection(config, |conn| {
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM mem_tree_jobs", [], |r| r.get(0))?;
        Ok(n.max(0) as u64)
    })
}

/// Fetch one job by id (test/diagnostic helper).
pub fn get_job(config: &Config, id: &str) -> Result<Option<Job>> {
    with_connection(config, |conn| {
        let job = conn
            .query_row(
                "SELECT id, kind, payload_json, dedupe_key, status, attempts, max_attempts,
                        available_at_ms, locked_until_ms, last_error,
                        created_at_ms, started_at_ms, completed_at_ms
                   FROM mem_tree_jobs WHERE id = ?1",
                params![id],
                row_to_job,
            )
            .optional()?;
        Ok(job)
    })
}

fn row_to_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    let id: String = row.get(0)?;
    let kind_s: String = row.get(1)?;
    let payload_json: String = row.get(2)?;
    let dedupe_key: Option<String> = row.get(3)?;
    let status_s: String = row.get(4)?;
    let attempts: i64 = row.get(5)?;
    let max_attempts: i64 = row.get(6)?;
    let available_at_ms: i64 = row.get(7)?;
    let locked_until_ms: Option<i64> = row.get(8)?;
    let last_error: Option<String> = row.get(9)?;
    let created_at_ms: i64 = row.get(10)?;
    let started_at_ms: Option<i64> = row.get(11)?;
    let completed_at_ms: Option<i64> = row.get(12)?;

    let kind = JobKind::parse(&kind_s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, e.into())
    })?;
    let status = JobStatus::parse(&status_s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, e.into())
    })?;

    Ok(Job {
        id,
        kind,
        payload_json,
        dedupe_key,
        status,
        attempts: attempts.max(0) as u32,
        max_attempts: max_attempts.max(0) as u32,
        available_at_ms,
        locked_until_ms,
        last_error,
        created_at_ms,
        started_at_ms,
        completed_at_ms,
    })
}

/// Exponential backoff: attempt 1 → 60s, 2 → 120s, 3 → 240s, capped at 1h.
fn backoff_ms(attempts_so_far: u32) -> i64 {
    // attempts_so_far is the count BEFORE the next retry's attempt — so the
    // first retry uses attempts_so_far=1, giving base*2^0 = 60s.
    let exp = attempts_so_far.saturating_sub(1).min(20); // cap shift
    let mult = 1i64 << exp; // 1, 2, 4, …
    let raw = RETRY_BASE_MS.saturating_mul(mult);
    raw.min(RETRY_CAP_MS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::jobs::types::{
        AppendBufferPayload, AppendTarget, ExtractChunkPayload, NodeRef,
    };
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

    #[test]
    fn enqueue_and_claim_roundtrip() {
        let (_tmp, cfg) = test_config();
        let payload = ExtractChunkPayload {
            chunk_id: "c1".into(),
        };
        let nj = NewJob::extract_chunk(&payload).unwrap();
        let id = enqueue(&cfg, &nj).unwrap().expect("inserted");

        let claimed = claim_next(&cfg, DEFAULT_LOCK_DURATION_MS).unwrap().unwrap();
        assert_eq!(claimed.id, id);
        assert_eq!(claimed.status, JobStatus::Running);
        assert_eq!(claimed.attempts, 1);
        assert!(claimed.locked_until_ms.is_some());

        // Second claim should see no eligible row (the only one is now running).
        let again = claim_next(&cfg, DEFAULT_LOCK_DURATION_MS).unwrap();
        assert!(again.is_none());

        mark_done(&cfg, &claimed).unwrap();
        let row = get_job(&cfg, &id).unwrap().unwrap();
        assert_eq!(row.status, JobStatus::Done);
        assert!(row.completed_at_ms.is_some());
        assert!(row.locked_until_ms.is_none());
    }

    #[test]
    fn enqueue_dedupes_active_jobs() {
        let (_tmp, cfg) = test_config();
        let payload = ExtractChunkPayload {
            chunk_id: "c1".into(),
        };
        let nj = NewJob::extract_chunk(&payload).unwrap();
        let id1 = enqueue(&cfg, &nj).unwrap();
        let id2 = enqueue(&cfg, &nj).unwrap();
        assert!(id1.is_some());
        assert!(id2.is_none(), "duplicate should be suppressed while ready");
        assert_eq!(count_total(&cfg).unwrap(), 1);
    }

    #[test]
    fn enqueue_after_done_creates_fresh_row() {
        let (_tmp, cfg) = test_config();
        let payload = ExtractChunkPayload {
            chunk_id: "c1".into(),
        };
        let nj = NewJob::extract_chunk(&payload).unwrap();
        let id1 = enqueue(&cfg, &nj).unwrap().unwrap();
        let claimed = claim_next(&cfg, DEFAULT_LOCK_DURATION_MS).unwrap().unwrap();
        assert_eq!(claimed.id, id1);
        mark_done(&cfg, &claimed).unwrap();

        // Now the dedupe key is free (partial index excludes 'done').
        let id2 = enqueue(&cfg, &nj).unwrap();
        assert!(id2.is_some());
        assert_ne!(id2.unwrap(), id1);
        assert_eq!(count_total(&cfg).unwrap(), 2);
    }

    #[test]
    fn mark_failed_retries_then_terminates() {
        let (_tmp, cfg) = test_config();
        let payload = AppendBufferPayload {
            node: NodeRef::Leaf {
                chunk_id: "c1".into(),
            },
            target: AppendTarget::Source {
                source_id: "slack:#x".into(),
            },
        };
        let mut nj = NewJob::append_buffer(&payload).unwrap();
        nj.max_attempts = Some(2);
        let id = enqueue(&cfg, &nj).unwrap().unwrap();

        // Fail #1 — should bounce back to 'ready' with future available_at.
        let attempt1 = claim_next(&cfg, DEFAULT_LOCK_DURATION_MS).unwrap().unwrap();
        mark_failed(&cfg, &attempt1, "boom").unwrap();
        let row = get_job(&cfg, &id).unwrap().unwrap();
        assert_eq!(row.status, JobStatus::Ready);
        assert!(row.available_at_ms > Utc::now().timestamp_millis());
        assert_eq!(row.last_error.as_deref(), Some("boom"));

        // Force the row available again so the test doesn't hinge on sleep.
        with_connection(&cfg, |c| {
            c.execute(
                "UPDATE mem_tree_jobs SET available_at_ms = 0 WHERE id = ?1",
                params![id],
            )?;
            Ok(())
        })
        .unwrap();

        // Fail #2 — exceeds max_attempts → terminal 'failed'.
        let attempt2 = claim_next(&cfg, DEFAULT_LOCK_DURATION_MS).unwrap().unwrap();
        mark_failed(&cfg, &attempt2, "fatal").unwrap();
        let row = get_job(&cfg, &id).unwrap().unwrap();
        assert_eq!(row.status, JobStatus::Failed);
        assert_eq!(row.last_error.as_deref(), Some("fatal"));
        assert!(row.completed_at_ms.is_some());
    }

    #[test]
    fn recover_stale_locks_resets_running_rows() {
        let (_tmp, cfg) = test_config();
        let payload = ExtractChunkPayload {
            chunk_id: "c1".into(),
        };
        let nj = NewJob::extract_chunk(&payload).unwrap();
        let id = enqueue(&cfg, &nj).unwrap().unwrap();

        // Claim with a lock window that's already in the past so recovery
        // sees it as expired.
        let _ = claim_next(&cfg, -1).unwrap().unwrap();

        let recovered = recover_stale_locks(&cfg).unwrap();
        assert_eq!(recovered, 1);
        let row = get_job(&cfg, &id).unwrap().unwrap();
        assert_eq!(row.status, JobStatus::Ready);
    }

    /// Happy path: a non-stale settlement still succeeds after the claim-token
    /// check is applied. Regression guard so the common case isn't broken.
    #[test]
    fn mark_done_succeeds_for_current_lessee() {
        let (_tmp, cfg) = test_config();
        let payload = ExtractChunkPayload {
            chunk_id: "c-happy".into(),
        };
        let nj = NewJob::extract_chunk(&payload).unwrap();
        let id = enqueue(&cfg, &nj).unwrap().expect("inserted");

        let claimed = claim_next(&cfg, DEFAULT_LOCK_DURATION_MS).unwrap().unwrap();
        assert_eq!(claimed.id, id);

        // Current lessee should settle successfully.
        mark_done(&cfg, &claimed).unwrap();
        let row = get_job(&cfg, &id).unwrap().unwrap();
        assert_eq!(row.status, JobStatus::Done);
        assert!(row.completed_at_ms.is_some());
        assert!(row.locked_until_ms.is_none());
    }

    /// Stale-worker settlement is a no-op: after a lock expires and a second
    /// worker re-claims the job, the first worker's `mark_done` must not
    /// clobber the new lessee's row.
    #[test]
    fn stale_worker_settlement_is_noop() {
        let (_tmp, cfg) = test_config();
        let payload = ExtractChunkPayload {
            chunk_id: "c-stale".into(),
        };
        let nj = NewJob::extract_chunk(&payload).unwrap();
        let id = enqueue(&cfg, &nj).unwrap().expect("inserted");

        // Worker A claims with a lock that's already expired (negative window).
        let worker_a_job = claim_next(&cfg, -1).unwrap().unwrap();
        assert_eq!(worker_a_job.id, id);
        assert_eq!(worker_a_job.attempts, 1);

        // Simulate lease expiry: recover_stale_locks resets the row to 'ready'.
        let recovered = recover_stale_locks(&cfg).unwrap();
        assert_eq!(recovered, 1);

        // Worker B claims the reset row — different lease token (attempts=2).
        let worker_b_job = claim_next(&cfg, DEFAULT_LOCK_DURATION_MS).unwrap().unwrap();
        assert_eq!(worker_b_job.id, id);
        assert_eq!(worker_b_job.attempts, 2);

        // Worker A (stale) tries to mark done using its old claim snapshot.
        mark_done(&cfg, &worker_a_job).unwrap(); // must NOT return Err

        // Worker B's row must be untouched — still 'running' with attempts=2.
        let row = get_job(&cfg, &id).unwrap().unwrap();
        assert_eq!(
            row.status,
            JobStatus::Running,
            "stale settlement must not clobber Worker B's running row"
        );
        assert_eq!(
            row.attempts, 2,
            "attempts must still reflect Worker B's claim"
        );
    }

    /// Same contract as stale_worker_settlement_is_noop but for mark_failed.
    #[test]
    fn stale_worker_mark_failed_is_noop() {
        let (_tmp, cfg) = test_config();
        let payload = ExtractChunkPayload {
            chunk_id: "c-stale-fail".into(),
        };
        let nj = NewJob::extract_chunk(&payload).unwrap();
        let id = enqueue(&cfg, &nj).unwrap().expect("inserted");

        // Worker A claims with an already-expired lock.
        let worker_a_job = claim_next(&cfg, -1).unwrap().unwrap();
        assert_eq!(worker_a_job.attempts, 1);

        // Lease expires, recovered, Worker B re-claims.
        let recovered = recover_stale_locks(&cfg).unwrap();
        assert_eq!(recovered, 1);
        let worker_b_job = claim_next(&cfg, DEFAULT_LOCK_DURATION_MS).unwrap().unwrap();
        assert_eq!(worker_b_job.attempts, 2);

        // Worker A (stale) tries to record a failure — must be a no-op.
        mark_failed(&cfg, &worker_a_job, "stale error").unwrap();

        // Worker B's row must be untouched.
        let row = get_job(&cfg, &id).unwrap().unwrap();
        assert_eq!(
            row.status,
            JobStatus::Running,
            "stale mark_failed must not clobber Worker B's running row"
        );
        assert_ne!(
            row.last_error.as_deref(),
            Some("stale error"),
            "stale error must not be written to the row"
        );
        assert_eq!(row.attempts, 2);
    }

    #[test]
    fn backoff_grows_then_caps() {
        assert_eq!(backoff_ms(1), 60_000);
        assert_eq!(backoff_ms(2), 120_000);
        assert_eq!(backoff_ms(3), 240_000);
        // Eventually clamps at the cap.
        assert_eq!(backoff_ms(20), RETRY_CAP_MS);
        assert_eq!(backoff_ms(99), RETRY_CAP_MS);
    }

    #[test]
    fn count_by_status_reports_each_state() {
        let (_tmp, cfg) = test_config();
        for i in 0..3 {
            let p = ExtractChunkPayload {
                chunk_id: format!("c{i}"),
            };
            let nj = NewJob::extract_chunk(&p).unwrap();
            enqueue(&cfg, &nj).unwrap();
        }
        assert_eq!(count_by_status(&cfg, JobStatus::Ready).unwrap(), 3);
        let claimed = claim_next(&cfg, DEFAULT_LOCK_DURATION_MS).unwrap().unwrap();
        mark_done(&cfg, &claimed).unwrap();
        assert_eq!(count_by_status(&cfg, JobStatus::Done).unwrap(), 1);
        assert_eq!(count_by_status(&cfg, JobStatus::Ready).unwrap(), 2);
    }
}
