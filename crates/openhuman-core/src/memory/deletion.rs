//! Deletions that must reach the engine, whenever it is reachable.
//!
//! Deleting a chat thread, forgetting a channel (one thread deletion per
//! thread it owned) or removing a source asks the engine to delete for good. When memory is off
//! (signed out, no credential) the engine cannot be reached, and when it
//! fails the deletion has not happened. Either way the deletion is recorded
//! here, in `<workspace>/memory/pending_deletions.json`, and [`drain`] runs
//! it again on the next sign-in ([`super::bus`], on `CredentialChanged`) and
//! on every background tick, until it succeeds.
//!
//! Every deletion is hard: a forget by filter removes the matched events by
//! `memory_ids` with an explicit `redact_events` cascade.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use async_trait::async_trait;
use tinymemory_api::{ForgetTarget, ItemKind, MetaFilter};
use tinymemory_tools::deletion::{Backoff, DeletionExecutor, DeletionQueue, Drain};
use tinymemory_tools::list_store::{ListStore, StoreError};

pub use tinymemory_tools::deletion::PendingDeletion;

use crate::config::Config;

use super::engine;
use super::error::{MemoryError, MemoryResult};

const STATE: &str = "pending_deletions";

/// The workspace's `pending_deletions` state value: a stop mid-write never
/// loses the queue (the storage write is atomic). An unparsable value is
/// dropped and logged; an empty queue removes the value.
struct FileStore {
    workspace_dir: PathBuf,
}

impl ListStore<PendingDeletion> for FileStore {
    fn load(&self) -> Result<Vec<PendingDeletion>, StoreError> {
        super::state_store::load(&self.workspace_dir, STATE)
            .map(Option::unwrap_or_default)
            .or_else(|error| {
                tracing::warn!(%error, "[memory:deletion] pending deletions unreadable; dropped");
                Ok(Vec::new())
            })
    }

    fn save(&self, all: &[PendingDeletion]) -> Result<(), StoreError> {
        if all.is_empty() {
            return super::state_store::remove(&self.workspace_dir, STATE)
                .map_err(|error| StoreError(error.to_string()));
        }
        super::state_store::save(&self.workspace_dir, STATE, &all)
            .map_err(|error| StoreError(error.to_string()))
    }
}

type Queue = DeletionQueue<FileStore>;

/// One queue per workspace, so the in-memory retry backoff and the drain lock
/// are shared by every trigger.
static QUEUES: LazyLock<Mutex<HashMap<PathBuf, Arc<Queue>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn queue(workspace_dir: &Path) -> Arc<Queue> {
    let mut queues = QUEUES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    queues
        .entry(workspace_dir.to_path_buf())
        .or_insert_with(|| {
            Arc::new(DeletionQueue::new(
                FileStore {
                    workspace_dir: workspace_dir.to_path_buf(),
                },
                Backoff::default(),
            ))
        })
        .clone()
}

/// Records `deletion` to run on the next [`drain`]. A deletion already
/// queued is not queued twice.
pub fn enqueue(workspace_dir: &Path, deletion: PendingDeletion) {
    queue(workspace_dir).enqueue(deletion);
}

/// Every deletion still owed, oldest first.
#[must_use]
pub fn pending(workspace_dir: &Path) -> Vec<PendingDeletion> {
    queue(workspace_dir).pending()
}

/// Runs queued deletions against the engine.
struct Executor<'a> {
    config: &'a Config,
}

#[async_trait]
impl DeletionExecutor for Executor<'_> {
    fn is_reachable(&self) -> bool {
        engine::resolve(self.config).is_on()
    }

    async fn run(&self, deletion: &PendingDeletion) -> Result<usize, String> {
        let result = match deletion {
            PendingDeletion::Thread { thread_id } => {
                forget_thread_now(self.config, thread_id).await
            }
            PendingDeletion::Source { source_id } => {
                super::sources::forget_items(self.config, source_id).await
            }
        };
        result.map_err(|error| error.code().to_string())
    }
}

/// Runs every queued deletion now (a sign-in), settling each one that
/// succeeds. Returns how many settled.
pub async fn drain(config: &Config) -> usize {
    queue(&config.workspace_dir)
        .drain(&Executor { config }, Drain::Force, chrono::Utc::now())
        .await
}

/// The background tick: like [`drain`], but a deletion that failed recently
/// waits out its backoff.
pub async fn drain_due(config: &Config) -> usize {
    queue(&config.workspace_dir)
        .drain(&Executor { config }, Drain::Due, chrono::Utc::now())
        .await
}

/// Forgets every conversation item of `thread_id` now.
async fn forget_thread_now(config: &Config, thread_id: &str) -> MemoryResult<usize> {
    let bound = engine::resolve(config).engine()?;
    let filter = MetaFilter {
        thread_id: Some(thread_id.to_string()),
        ..MetaFilter::kinds([ItemKind::Conversation])
    };
    let report = bound.engine.forget(ForgetTarget::Filter(filter)).await?;
    tracing::debug!(
        forgotten = report.forgotten,
        "[memory:deletion] thread conversation memory forgotten"
    );
    Ok(report.forgotten)
}

/// Forgets a deleted thread's conversation memory, for good. Memory off or
/// a failed forget queues the deletion for the next [`drain`] rather than
/// failing the caller: the thread itself is already gone. Returns how many
/// items went now.
pub async fn forget_thread(config: &Config, thread_id: &str) -> usize {
    if thread_id.trim().is_empty() {
        return 0;
    }
    let deletion = PendingDeletion::Thread {
        thread_id: thread_id.to_string(),
    };
    match forget_thread_now(config, thread_id).await {
        Ok(forgotten) => forgotten,
        Err(MemoryError::Off(_)) => {
            enqueue(&config.workspace_dir, deletion);
            0
        }
        Err(error) => {
            tracing::warn!(
                code = error.code(),
                "[memory:deletion] forgetting a deleted thread failed; queued"
            );
            enqueue(&config.workspace_dir, deletion);
            0
        }
    }
}

#[cfg(test)]
#[path = "deletion_tests.rs"]
mod tests;
