//! The background job queue: the slow memory work TinyMemory hands back.
//!
//! The queue itself (dedupe, retry, history) is `tinymemory_tools::queue`;
//! this module is the host adapter: one queue per workspace over a file
//! store (the `memory`/`state`/`jobs` storage value), the engine and `[memory.recall]
//! build_delay_secs` it runs against, and the `memory_background` cron hook.
//!
//! `post_turn` and brain ingests return `BackgroundJob` values (belief builds,
//! deferred ingests) instead of running them. They are queued here, persisted
//! in the `memory`/`state`/`jobs` storage value so a restart loses none, de-duplicated
//! (two builds of one scope are one build), and run by the
//! `memory_background` cron job ([`run_due`]) once they are at least
//! `[memory.recall] build_delay_secs` old — a belief build reads the facts the
//! engine extracts from the writes, and that extraction lags behind them.
//!
//! An engine that cannot consolidate answers `Skipped`, so the same queue runs
//! on every engine; the hosted engine builds on its own schedule
//! (`Scheduled`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use tinymemory_api::Namespace;
use tinymemory_tools::queue::{JobQueue as Queue, QueueError, QueueState, QueueStore};
use tinymemory_tools::BackgroundJob;

pub use tinymemory_tools::queue::{JobRun, QueuedJob, Selection, MAX_ATTEMPTS, MAX_HISTORY};

use crate::config::Config;
use crate::memory::engine;
use crate::memory::error::{MemoryError, MemoryResult};

/// The cron job that runs the queue.
pub const BACKGROUND_JOB: &str = "memory_background";

/// Minutes between queue runs.
pub const BACKGROUND_INTERVAL_MINS: u64 = 5;

/// The persisted queue as the UI reads it.
pub type JobQueue = QueueState;

const STATE: &str = "jobs";

/// A [`QueueStore`] over the workspace's `jobs` state value.
struct FileQueueStore {
    workspace_dir: PathBuf,
}

impl QueueStore for FileQueueStore {
    fn load(&self) -> QueueState {
        crate::memory::state_store::load(&self.workspace_dir, STATE)
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "[memory:jobs] reading the queue failed");
                None
            })
            .unwrap_or_default()
    }

    fn save(&self, state: &QueueState) -> Result<(), String> {
        crate::memory::state_store::save(&self.workspace_dir, STATE, state)
            .map_err(|error| error.to_string())
    }
}

type WorkspaceQueue = Queue<FileQueueStore>;

/// One queue per workspace, so its lock serialises every run in the process.
static QUEUES: LazyLock<Mutex<HashMap<PathBuf, Arc<WorkspaceQueue>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn queue_of(workspace_dir: &Path) -> Arc<WorkspaceQueue> {
    let mut queues = QUEUES.lock().unwrap_or_else(|e| e.into_inner());
    queues
        .entry(workspace_dir.to_path_buf())
        .or_insert_with(|| {
            Arc::new(Queue::new(FileQueueStore {
                workspace_dir: workspace_dir.to_path_buf(),
            }))
        })
        .clone()
}

/// Queues `jobs` under `root`; a job already queued is kept once, at its
/// first queue time.
pub async fn enqueue(config: &Config, root: &Namespace, jobs: Vec<BackgroundJob>) {
    queue_of(&config.workspace_dir).enqueue(root, jobs).await;
}

/// Whether `workspace_dir` has queued jobs waiting. A cheap, lock-free read
/// for a host deciding which workspaces are worth a run.
pub fn has_pending(workspace_dir: &Path) -> bool {
    queue_of(workspace_dir).has_pending()
}

/// The queue as it stands.
pub async fn snapshot(config: &Config) -> JobQueue {
    queue_of(&config.workspace_dir).snapshot().await
}

/// Runs the selected jobs on the configured engine. Returns the runs it
/// recorded; see `tinymemory_tools::queue::JobQueue::run` for the retry rules.
pub async fn run(config: &Config, selection: Selection) -> MemoryResult<Vec<JobRun>> {
    let bound = engine::resolve(config).engine()?;
    queue_of(&config.workspace_dir)
        .run(
            bound.engine,
            selection,
            config.memory.recall.build_delay_secs,
        )
        .await
        .map_err(|error| match error {
            QueueError::NoSuchJob(_) => MemoryError::invalid(error.to_string()),
        })
}

/// The cron entry point: runs due jobs unless background work is paused.
pub async fn run_due(config: &Config) {
    if let crate::platform::llm_gate::Policy::Paused { reason } =
        crate::platform::llm_gate::current_policy()
    {
        tracing::debug!(
            ?reason,
            "[memory:jobs] background paused; queue left for later"
        );
        return;
    }
    match run(config, Selection::Due).await {
        Ok(runs) if !runs.is_empty() => {
            tracing::debug!(ran = runs.len(), "[memory:jobs] background run finished");
        }
        Ok(_) => {}
        Err(MemoryError::Off(_)) => {}
        Err(error) => tracing::warn!(%error, "[memory:jobs] background run failed"),
    }
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod tests;
