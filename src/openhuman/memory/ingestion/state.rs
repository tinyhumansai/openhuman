//! Shared state + singleton lock for memory ingestion.
//!
//! Memory ingestion runs the local extraction LLM and must not run more than
//! once concurrently — otherwise multiple jobs contend for the same local AI
//! and either thrash or fail. [`IngestionState`] enforces the singleton via
//! [`tokio::sync::Mutex`] and exposes a snapshot suitable for the
//! `openhuman.memory_ingestion_status` RPC.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;
use serde::Serialize;
use tokio::sync::Mutex;

/// Snapshot of ingestion state, surfaced over RPC.
#[derive(Debug, Clone, Default, Serialize)]
pub struct IngestionStatusSnapshot {
    /// Whether an ingestion job is currently running.
    pub running: bool,
    /// Document id of the in-flight job, if any.
    pub current_document_id: Option<String>,
    /// Document title of the in-flight job, if any (best-effort).
    pub current_title: Option<String>,
    /// Namespace of the in-flight job, if any.
    pub current_namespace: Option<String>,
    /// Number of jobs waiting in the queue (not counting the running one).
    pub queue_depth: usize,
    /// Unix-ms timestamp of when the most recent job completed.
    pub last_completed_at: Option<i64>,
    /// Document id of the most recent completed job.
    pub last_document_id: Option<String>,
    /// Whether the most recent job succeeded.
    pub last_success: Option<bool>,
}

/// Shared ingestion state + singleton lock. Cheap to clone.
#[derive(Clone)]
pub struct IngestionState {
    inner: Arc<IngestionStateInner>,
}

struct IngestionStateInner {
    /// Singleton lock — held while a job is running.
    run_lock: Mutex<()>,
    /// Queue depth — bumped on submit, decremented when the worker pulls a job.
    queue_depth: AtomicUsize,
    /// Snapshot for status RPC.
    snapshot: RwLock<IngestionStatusSnapshot>,
}

impl Default for IngestionState {
    fn default() -> Self {
        Self::new()
    }
}

impl IngestionState {
    /// Create a fresh state with empty snapshot and zero queue depth.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(IngestionStateInner {
                run_lock: Mutex::new(()),
                queue_depth: AtomicUsize::new(0),
                snapshot: RwLock::new(IngestionStatusSnapshot::default()),
            }),
        }
    }

    /// Bump the pending-queue depth (call on `submit`).
    pub fn enqueue(&self) {
        self.inner.queue_depth.fetch_add(1, Ordering::SeqCst);
    }

    /// Decrement pending-queue depth (call when the worker has pulled a job
    /// off the channel and is about to acquire the run lock).
    pub fn dequeue(&self) {
        self.inner.queue_depth.fetch_sub(1, Ordering::SeqCst);
    }

    /// Acquire the singleton run lock. Holders run ingestion serialised; any
    /// other caller blocks until the holder drops the guard.
    pub async fn acquire(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.inner.run_lock.lock().await
    }

    /// Mark a job as in-flight in the snapshot. Caller must already hold
    /// [`Self::acquire`].
    pub fn mark_running(&self, document_id: &str, title: &str, namespace: &str) {
        let mut snap = self.inner.snapshot.write();
        snap.running = true;
        snap.current_document_id = Some(document_id.to_string());
        snap.current_title = Some(title.to_string());
        snap.current_namespace = Some(namespace.to_string());
    }

    /// Mark the in-flight job as finished.
    pub fn mark_completed(&self, document_id: &str, success: bool, completed_at_ms: i64) {
        let mut snap = self.inner.snapshot.write();
        snap.running = false;
        snap.current_document_id = None;
        snap.current_title = None;
        snap.current_namespace = None;
        snap.last_completed_at = Some(completed_at_ms);
        snap.last_document_id = Some(document_id.to_string());
        snap.last_success = Some(success);
    }

    /// Returns a clone of the current snapshot. Includes live queue depth.
    pub fn snapshot(&self) -> IngestionStatusSnapshot {
        let mut snap = self.inner.snapshot.read().clone();
        snap.queue_depth = self.inner.queue_depth.load(Ordering::SeqCst);
        snap
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn singleton_serialises_concurrent_acquires() {
        let state = IngestionState::new();
        let counter = Arc::new(parking_lot::Mutex::new(0u32));
        let max_concurrent = Arc::new(parking_lot::Mutex::new(0u32));

        let mut handles = Vec::new();
        for _ in 0..4 {
            let state = state.clone();
            let counter = Arc::clone(&counter);
            let max_concurrent = Arc::clone(&max_concurrent);
            handles.push(tokio::spawn(async move {
                let _g = state.acquire().await;
                let now = {
                    let mut c = counter.lock();
                    *c += 1;
                    *c
                };
                {
                    let mut m = max_concurrent.lock();
                    if now > *m {
                        *m = now;
                    }
                }
                sleep(Duration::from_millis(20)).await;
                *counter.lock() -= 1;
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        assert_eq!(*max_concurrent.lock(), 1, "ingestion must be singleton");
    }

    #[test]
    fn snapshot_reports_running_and_queue_depth() {
        let state = IngestionState::new();
        state.enqueue();
        state.enqueue();
        let snap = state.snapshot();
        assert_eq!(snap.queue_depth, 2);
        assert!(!snap.running);

        state.dequeue();
        state.mark_running("doc-1", "title", "ns");
        let snap = state.snapshot();
        assert_eq!(snap.queue_depth, 1);
        assert!(snap.running);
        assert_eq!(snap.current_document_id.as_deref(), Some("doc-1"));

        state.mark_completed("doc-1", true, 12345);
        let snap = state.snapshot();
        assert!(!snap.running);
        assert_eq!(snap.last_document_id.as_deref(), Some("doc-1"));
        assert_eq!(snap.last_success, Some(true));
        assert_eq!(snap.last_completed_at, Some(12345));
    }
}
