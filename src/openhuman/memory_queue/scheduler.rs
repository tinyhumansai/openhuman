//! Wall-clock scheduler that wakes once a day shortly after UTC midnight to
//! enqueue the global [`JobKind::DigestDaily`] for yesterday and a
//! [`JobKind::FlushStale`] for today. Also exposes manual-trigger helpers
//! for catch-up and testing.

use std::time::Duration;

use anyhow::Result;
use chrono::{Datelike, Duration as ChronoDuration, NaiveDate, TimeZone, Timelike, Utc};

use crate::openhuman::config::Config;
use crate::openhuman::memory::jobs::store;
use crate::openhuman::memory::jobs::types::{DigestDailyPayload, FlushStalePayload, NewJob};

static STARTED: std::sync::Once = std::sync::Once::new();

/// Start the daily wall-clock scheduler. Takes the full `Config` so the
/// digest enqueues match the same workspace + LLM settings the workers
/// see — not `Config::default()`.
pub fn start(config: Config) {
    STARTED.call_once(|| {
        // Daily midnight loop: digest + flush_stale.
        let cfg1 = config.clone();
        tokio::spawn(async move {
            loop {
                if let Err(err) = enqueue_daily_jobs(&cfg1) {
                    log::warn!("[memory::jobs] scheduler enqueue failed: {err:#}");
                }
                tokio::time::sleep(next_sleep_duration()).await;
            }
        });

        // Periodic flush_stale loop (every 3 h) so L0 buffers seal
        // promptly even for low-volume sources.
        let cfg2 = config.clone();
        tokio::spawn(async move {
            // Fire once on startup so new installs & restarts don't wait
            // up to 3 h for the first seal window.
            enqueue_flush_stale(&cfg2);
            loop {
                tokio::time::sleep(Duration::from_secs(3 * 60 * 60)).await;
                enqueue_flush_stale(&cfg2);
            }
        });
    });
}

fn enqueue_flush_stale(config: &Config) {
    // Take a single `Utc::now()` reading and derive both the date and
    // 3-hour block from it so the dedupe key can't disagree with itself
    // across a 3-hour boundary.
    let now = Utc::now();
    let today_iso = now.date_naive().format("%Y-%m-%d").to_string();
    let hour_block = now.hour() / 3;
    match NewJob::flush_stale(&FlushStalePayload::default(), &today_iso, hour_block) {
        Ok(new_job) => {
            match store::enqueue(config, &new_job) {
                Ok(Some(_)) => {
                    super::worker::wake_workers();
                }
                Ok(None) => {} // dedupe-suppressed — OK
                Err(err) => {
                    log::warn!("[memory::jobs] periodic flush_stale enqueue failed: {err:#}");
                }
            }
        }
        Err(err) => {
            log::warn!("[memory::jobs] flush_stale job build failed: {err:#}");
        }
    }
}

fn enqueue_daily_jobs(config: &Config) -> anyhow::Result<()> {
    let now = Utc::now();
    let yesterday = now.date_naive() - ChronoDuration::days(1);
    let date_iso = yesterday.format("%Y-%m-%d").to_string();

    if store::enqueue(
        config,
        &NewJob::digest_daily(&DigestDailyPayload {
            date_iso: date_iso.clone(),
        })?,
    )?
    .is_some()
    {
        super::worker::wake_workers();
    }

    let today_iso = now.date_naive().format("%Y-%m-%d").to_string();
    let hour_block = now.hour() / 3;
    if store::enqueue(
        config,
        &NewJob::flush_stale(&FlushStalePayload::default(), &today_iso, hour_block)?,
    )?
    .is_some()
    {
        super::worker::wake_workers();
    }

    Ok(())
}

/// Manually enqueue a `digest_daily` job for `date`. Idempotent — if a
/// digest already ran for that day, the handler's `find_existing_daily`
/// check will return `Skipped` without doing any work; if a job for the
/// same date is already queued or running, the partial unique index on
/// `dedupe_key` suppresses the duplicate.
///
/// Useful for catch-up after the process was down across midnight, or
/// to force a re-run for testing / debugging.
pub fn trigger_digest(config: &Config, date: NaiveDate) -> Result<Option<String>> {
    let payload = DigestDailyPayload {
        date_iso: date.format("%Y-%m-%d").to_string(),
    };
    let job_id = store::enqueue(config, &NewJob::digest_daily(&payload)?)?;
    if job_id.is_some() {
        log::info!(
            "[memory::jobs] manual digest trigger enqueued date={} id={:?}",
            payload.date_iso,
            job_id.as_deref()
        );
        super::worker::wake_workers();
    } else {
        log::debug!(
            "[memory::jobs] manual digest trigger dedupe-suppressed date={} \
             (an active job for this date already exists)",
            payload.date_iso
        );
    }
    Ok(job_id)
}

/// Enqueue `digest_daily` jobs for the last `days_back` calendar days
/// (excluding today). Catch-up helper for cases where the scheduler
/// missed days because the process was down.
///
/// Returns the number of jobs newly enqueued. Days that already have a
/// completed digest are still re-enqueued — the handler is idempotent
/// and skips them — so this is safe to call repeatedly.
pub fn backfill_missing_digests(config: &Config, days_back: i64) -> Result<usize> {
    if days_back <= 0 {
        return Ok(0);
    }
    let today = Utc::now().date_naive();
    let mut enqueued = 0usize;
    for offset in 1..=days_back {
        let date = today - ChronoDuration::days(offset);
        if trigger_digest(config, date)?.is_some() {
            enqueued += 1;
        }
    }
    log::info!(
        "[memory::jobs] backfill_missing_digests window={}d enqueued={}",
        days_back,
        enqueued
    );
    Ok(enqueued)
}

fn next_sleep_duration() -> Duration {
    let now = Utc::now();
    let tomorrow = now.date_naive() + ChronoDuration::days(1);
    let next = Utc
        .with_ymd_and_hms(tomorrow.year(), tomorrow.month(), tomorrow.day(), 0, 5, 0)
        // UTC has no DST gaps/overlaps, so `single()` always returns
        // `Some` for any valid (Y, M, D, h, m, s). Fallback retained
        // only as a defensive belt-and-braces against future API churn.
        .single()
        .unwrap_or_else(|| now + ChronoDuration::hours(24));
    (next - now)
        .to_std()
        .unwrap_or_else(|_| Duration::from_secs(60))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::jobs::store::{
        claim_next, count_by_status, count_total, mark_done, DEFAULT_LOCK_DURATION_MS,
    };
    use crate::openhuman::memory::jobs::types::{
        DigestDailyPayload, FlushStalePayload, JobKind, JobStatus,
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
    fn trigger_digest_enqueues_a_job() {
        let (_tmp, cfg) = test_config();
        let date = NaiveDate::from_ymd_opt(2026, 4, 27).unwrap();
        let id = trigger_digest(&cfg, date).unwrap();
        assert!(id.is_some(), "first trigger must enqueue");
        assert_eq!(count_by_status(&cfg, JobStatus::Ready).unwrap(), 1);
    }

    #[test]
    fn trigger_digest_dedupes_active_jobs() {
        let (_tmp, cfg) = test_config();
        let date = NaiveDate::from_ymd_opt(2026, 4, 27).unwrap();
        let first = trigger_digest(&cfg, date).unwrap();
        let second = trigger_digest(&cfg, date).unwrap();
        assert!(first.is_some());
        assert!(
            second.is_none(),
            "duplicate trigger must be dedupe-suppressed while active"
        );
        assert_eq!(count_total(&cfg).unwrap(), 1);
    }

    #[test]
    fn trigger_digest_after_done_creates_fresh_row() {
        let (_tmp, cfg) = test_config();
        let date = NaiveDate::from_ymd_opt(2026, 4, 27).unwrap();
        let id1 = trigger_digest(&cfg, date).unwrap().unwrap();
        // Simulate a worker finishing the job — claim it first so we have a
        // Job snapshot for the claim-token-gated mark_done.
        let claimed = claim_next(&cfg, DEFAULT_LOCK_DURATION_MS).unwrap().unwrap();
        assert_eq!(claimed.id, id1);
        mark_done(&cfg, &claimed).unwrap();

        let id2 = trigger_digest(&cfg, date).unwrap();
        assert!(
            id2.is_some(),
            "after the prior job completes, a fresh trigger must enqueue"
        );
        assert_ne!(id2.unwrap(), id1);
        assert_eq!(count_total(&cfg).unwrap(), 2);
    }

    #[test]
    fn backfill_missing_digests_enqueues_one_per_day() {
        let (_tmp, cfg) = test_config();
        let n = backfill_missing_digests(&cfg, 5).unwrap();
        assert_eq!(n, 5, "expected one job per day in the 5-day window");
        assert_eq!(count_total(&cfg).unwrap(), 5);
    }

    #[test]
    fn backfill_missing_digests_zero_window_is_noop() {
        let (_tmp, cfg) = test_config();
        let n = backfill_missing_digests(&cfg, 0).unwrap();
        assert_eq!(n, 0);
        assert_eq!(count_total(&cfg).unwrap(), 0);
    }

    #[test]
    fn backfill_missing_digests_negative_window_is_noop() {
        let (_tmp, cfg) = test_config();
        let n = backfill_missing_digests(&cfg, -3).unwrap();
        assert_eq!(n, 0);
        assert_eq!(count_total(&cfg).unwrap(), 0);
    }

    #[test]
    fn backfill_missing_digests_is_idempotent_while_active() {
        let (_tmp, cfg) = test_config();
        let n1 = backfill_missing_digests(&cfg, 3).unwrap();
        let n2 = backfill_missing_digests(&cfg, 3).unwrap();
        assert_eq!(n1, 3);
        assert_eq!(n2, 0, "second call must be fully dedupe-suppressed");
        assert_eq!(count_total(&cfg).unwrap(), 3);
    }

    #[test]
    fn enqueue_flush_stale_enqueues_at_most_one_job_per_current_block() {
        let (_tmp, cfg) = test_config();
        enqueue_flush_stale(&cfg);
        enqueue_flush_stale(&cfg);

        assert_eq!(
            count_by_status(&cfg, JobStatus::Ready).unwrap(),
            1,
            "second enqueue in same 3h block should be dedupe-suppressed"
        );

        let claimed = claim_next(&cfg, DEFAULT_LOCK_DURATION_MS).unwrap().unwrap();
        assert_eq!(claimed.kind, JobKind::FlushStale);
        let payload: FlushStalePayload = serde_json::from_str(&claimed.payload_json).unwrap();
        assert_eq!(payload.max_age_secs, None);
    }

    #[test]
    fn enqueue_daily_jobs_adds_digest_and_flush_jobs() {
        let (_tmp, cfg) = test_config();
        enqueue_daily_jobs(&cfg).unwrap();

        assert_eq!(count_total(&cfg).unwrap(), 2);

        let first = claim_next(&cfg, DEFAULT_LOCK_DURATION_MS).unwrap().unwrap();
        assert_eq!(
            first.kind,
            JobKind::DigestDaily,
            "digest_daily should be claimed ahead of flush_stale"
        );
        let digest: DigestDailyPayload = serde_json::from_str(&first.payload_json).unwrap();
        assert!(!digest.date_iso.is_empty());
        mark_done(&cfg, &first).unwrap();

        let second = claim_next(&cfg, DEFAULT_LOCK_DURATION_MS).unwrap().unwrap();
        assert_eq!(second.kind, JobKind::FlushStale);
        let flush: FlushStalePayload = serde_json::from_str(&second.payload_json).unwrap();
        assert_eq!(flush.max_age_secs, None);
    }

    #[test]
    fn enqueue_daily_jobs_is_fully_deduped_while_jobs_remain_active() {
        let (_tmp, cfg) = test_config();
        enqueue_daily_jobs(&cfg).unwrap();
        enqueue_daily_jobs(&cfg).unwrap();

        assert_eq!(
            count_total(&cfg).unwrap(),
            2,
            "same-day scheduler rerun should not create duplicate active jobs"
        );
    }

    #[test]
    fn enqueue_daily_jobs_reenqueues_after_prior_rows_complete() {
        let (_tmp, cfg) = test_config();
        enqueue_daily_jobs(&cfg).unwrap();

        let first = claim_next(&cfg, DEFAULT_LOCK_DURATION_MS).unwrap().unwrap();
        mark_done(&cfg, &first).unwrap();
        let second = claim_next(&cfg, DEFAULT_LOCK_DURATION_MS).unwrap().unwrap();
        mark_done(&cfg, &second).unwrap();

        enqueue_daily_jobs(&cfg).unwrap();

        assert_eq!(
            count_total(&cfg).unwrap(),
            4,
            "completed daily jobs should allow a fresh digest+flush pair"
        );
        assert_eq!(count_by_status(&cfg, JobStatus::Ready).unwrap(), 2);
    }

    #[test]
    fn next_sleep_duration_targets_near_next_midnight_utc_plus_five_minutes() {
        let sleep = next_sleep_duration();
        assert!(
            sleep.as_secs() > 0,
            "scheduler sleep should always be positive"
        );
        assert!(
            sleep.as_secs() <= 24 * 60 * 60 + 5 * 60,
            "scheduler should never sleep for more than ~24h+5m"
        );
    }
}
