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
use crate::openhuman::memory::tree::jobs::store::{
    claim_next, mark_done, mark_failed, recover_stale_locks, DEFAULT_LOCK_DURATION_MS,
};

const WORKER_COUNT: usize = 1;
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
        gate_permit
    } else {
        // Non-LLM jobs don't need the global slot; release it so an
        // LLM-bound caller waiting elsewhere in the process can run.
        drop(gate_permit);
        None
    };

    let result = handlers::handle_job(config, &job).await;
    drop(llm_permit);

    match result {
        Ok(()) => {
            mark_done(config, &job)?;
        }
        Err(err) => {
            // Preserve the full anyhow cause chain in the persisted
            // last_error so a reader of mem_tree_jobs can see the root
            // cause, not just the top-level message. Mirrors the {:#}
            // log format used right above.
            let message = format!("{err:#}");
            log::warn!(
                "[memory_tree::jobs] job failed id={} kind={} err={:#}",
                job.id,
                job.kind.as_str(),
                err
            );
            mark_failed(config, &job, &message)?;
        }
    }

    Ok(true)
}
