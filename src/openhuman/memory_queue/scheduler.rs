//! Wall-clock scheduler that periodically enqueues a [`JobKind::FlushStale`]
//! so low-volume source-tree L0 buffers seal promptly.
//!
//! The daily global-digest loop was removed along with the global tree —
//! source trees plus the entity index are the substrate, so there is no
//! cross-source digest to enqueue. Only the stale-buffer flush remains.

use std::time::Duration;

use chrono::{Timelike, Utc};

use crate::openhuman::config::Config;
use crate::openhuman::memory_queue::store;
use crate::openhuman::memory_queue::types::{FlushStalePayload, NewJob};

static STARTED: std::sync::Once = std::sync::Once::new();

/// Start the periodic flush_stale scheduler. Takes the full `Config` so the
/// enqueues match the same workspace + LLM settings the workers see — not
/// `Config::default()`.
pub fn start(config: Config) {
    STARTED.call_once(|| {
        // Periodic flush_stale loop (every 3 h) so L0 buffers seal
        // promptly even for low-volume sources.
        let cfg = config.clone();
        tokio::spawn(async move {
            // Fire once on startup so new installs & restarts don't wait
            // up to 3 h for the first seal window.
            retry_transient_failures(&cfg);
            enqueue_flush_stale(&cfg);
            loop {
                tokio::time::sleep(Duration::from_secs(3 * 60 * 60)).await;
                retry_transient_failures(&cfg);
                enqueue_flush_stale(&cfg);
            }
        });
    });
}

/// Self-heal the pipeline before each flush window: requeue jobs that
/// failed for transient reasons (network blips, timeouts, SQLITE_BUSY)
/// so chunks never sit unprocessed until the next manual sync.
/// Unrecoverable failures stay parked — see
/// [`store::requeue_transient_failed`].
fn retry_transient_failures(config: &Config) {
    match store::requeue_transient_failed(config) {
        Ok(0) => {}
        Ok(n) => {
            log::info!("[memory::jobs] periodic retry requeued {n} transient-failed job(s)");
            super::worker::wake_workers();
        }
        Err(err) => {
            log::warn!("[memory::jobs] periodic transient-failure retry failed: {err:#}");
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory_queue::store::{
        claim_next, count_by_status, DEFAULT_LOCK_DURATION_MS,
    };
    use crate::openhuman::memory_queue::types::{FlushStalePayload, JobKind, JobStatus};
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
}
