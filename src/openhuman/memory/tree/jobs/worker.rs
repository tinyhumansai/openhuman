use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Result;
use tokio::sync::{Notify, Semaphore};

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::jobs::handlers;
use crate::openhuman::memory::tree::jobs::store::{
    claim_next, mark_done, mark_failed, recover_stale_locks, DEFAULT_LOCK_DURATION_MS,
};

const WORKER_COUNT: usize = 3;
const POLL_INTERVAL: Duration = Duration::from_secs(5);

static WORKER_NOTIFY: OnceLock<Arc<Notify>> = OnceLock::new();
static STARTED: std::sync::Once = std::sync::Once::new();

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
        let llm_slots = Arc::new(Semaphore::new(3));
        if let Err(err) = recover_stale_locks(&config) {
            log::warn!("[memory_tree::jobs] recover_stale_locks failed at startup: {err:#}");
        }

        for idx in 0..WORKER_COUNT {
            let notify = notify.clone();
            let llm_slots = llm_slots.clone();
            let cfg = config.clone();
            tokio::spawn(async move {
                loop {
                    match run_once_with_semaphore(&cfg, llm_slots.clone()).await {
                        Ok(true) => continue,
                        Ok(false) => {
                            tokio::select! {
                                _ = notify.notified() => {}
                                _ = tokio::time::sleep(POLL_INTERVAL) => {}
                            }
                        }
                        Err(err) => {
                            log::error!("[memory_tree::jobs] worker={} loop error: {:#}", idx, err);
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            });
        }

        super::scheduler::start(config);
    });
}

pub async fn run_once(config: &Config) -> Result<bool> {
    let llm_slots = Arc::new(Semaphore::new(1));
    run_once_with_semaphore(config, llm_slots).await
}

async fn run_once_with_semaphore(config: &Config, llm_slots: Arc<Semaphore>) -> Result<bool> {
    let Some(job) = claim_next(config, DEFAULT_LOCK_DURATION_MS)? else {
        return Ok(false);
    };

    let permit = if job.kind.is_llm_bound() {
        Some(llm_slots.acquire().await?)
    } else {
        None
    };
    let result = handlers::handle_job(config, &job).await;
    drop(permit);

    match result {
        Ok(()) => {
            mark_done(config, &job.id)?;
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
            mark_failed(config, &job.id, &message)?;
        }
    }

    Ok(true)
}
