//! # Background Ingestion Queue
//!
//! Processes documents through the entity/relation extraction pipeline on a
//! dedicated worker thread. This ensures that `doc_put` callers never block
//! on the heavier parsing and graph-write path.
//!
//! The queue uses a `tokio::sync::mpsc` channel to decouple document submission
//! from the actual extraction process.

use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;

use super::state::IngestionState;
use super::MemoryIngestionConfig;
use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::memory::store::{NamespaceDocumentInput, UnifiedMemory};

/// Default bounded-channel capacity for the ingestion queue. Sized to absorb
/// realistic bursts (bulk skill sync of ~200 docs) while capping memory usage.
pub const DEFAULT_QUEUE_CAPACITY: usize = 512;

/// A job submitted to the ingestion worker.
///
/// Contains all the necessary information to process a document for graph
/// extraction, including the document content itself and the configuration
/// for the extraction process.
#[derive(Debug, Clone)]
pub struct IngestionJob {
    /// The document that was already stored via `upsert_document`.
    pub document: NamespaceDocumentInput,
    /// The document ID returned by `upsert_document`.
    pub document_id: String,
    /// Configuration for the extraction process (e.g., model name, thresholds).
    pub config: MemoryIngestionConfig,
}

/// Handle used by callers to submit ingestion jobs.
///
/// This is a thin wrapper around a `tokio::sync::mpsc::Sender` and
/// can be cloned freely to be shared across multiple producers.
#[derive(Clone)]
pub struct IngestionQueue {
    /// Sender half of the job queue channel.
    tx: mpsc::Sender<IngestionJob>,
    /// Shared state — singleton lock, queue depth, status snapshot.
    state: IngestionState,
    /// The actual channel capacity this queue was created with. Stored so
    /// backpressure logs always reflect the real configured size rather than
    /// the `DEFAULT_QUEUE_CAPACITY` constant (which may differ for test
    /// queues or future callers of `start_worker_with_capacity`).
    capacity: usize,
}

impl IngestionQueue {
    /// Submit a document for background graph extraction. Returns immediately.
    ///
    /// # Arguments
    ///
    /// * `job` - The [`IngestionJob`] to be processed.
    ///
    /// # Returns
    ///
    /// Returns `true` if the job was successfully enqueued, `false` if the
    /// queue is full (backpressure) or the worker has shut down.
    pub fn submit(&self, job: IngestionJob) -> bool {
        self.state.enqueue();
        match self.tx.try_send(job) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(dropped)) => {
                self.state.dequeue();
                log::warn!(
                    "[memory:ingestion_queue] queue full (capacity {}), dropping job: {}",
                    self.capacity,
                    dropped.document.title,
                );
                false
            }
            Err(mpsc::error::TrySendError::Closed(dropped)) => {
                self.state.dequeue();
                log::warn!(
                    "[memory:ingestion_queue] failed to enqueue job (worker gone?): {}",
                    dropped.document.title,
                );
                false
            }
        }
    }

    /// Returns a clone of the shared ingestion state. Use this to drive the
    /// status RPC or to share the singleton lock with synchronous ingest
    /// paths that bypass the queue.
    pub fn state(&self) -> IngestionState {
        self.state.clone()
    }

    /// Build a queue handle from a raw sender, state, and capacity. Test-only.
    #[cfg(test)]
    fn from_parts(tx: mpsc::Sender<IngestionJob>, state: IngestionState, capacity: usize) -> Self {
        Self {
            tx,
            state,
            capacity,
        }
    }
}

/// Start the background ingestion worker.
///
/// # Arguments
///
/// * `memory` - An `Arc` to the [`UnifiedMemory`] instance used for extraction.
///
/// # Returns
///
/// Returns an [`IngestionQueue`] handle that can be cloned and shared with
/// any number of producers. The worker runs on a dedicated tokio task,
/// processing jobs sequentially so ingestion work stays serialized.
pub fn start_worker(memory: Arc<UnifiedMemory>) -> IngestionQueue {
    let state = IngestionState::new();
    start_worker_with_state(memory, state)
}

/// Start a worker bound to a caller-supplied [`IngestionState`]. Useful when
/// the synchronous ingest path needs to share the same singleton lock and
/// snapshot as the queue worker.
pub fn start_worker_with_state(
    memory: Arc<UnifiedMemory>,
    state: IngestionState,
) -> IngestionQueue {
    start_worker_with_capacity(memory, state, DEFAULT_QUEUE_CAPACITY)
}

/// Start a worker with an explicit channel capacity. Exposed for
/// deterministic tests that need a tiny queue to exercise backpressure.
pub fn start_worker_with_capacity(
    memory: Arc<UnifiedMemory>,
    state: IngestionState,
    capacity: usize,
) -> IngestionQueue {
    let (tx, rx) = mpsc::channel::<IngestionJob>(capacity);

    tokio::spawn(ingestion_worker(memory, rx, state.clone()));

    log::debug!(
        "[memory:ingestion_queue] background worker started (capacity={})",
        capacity,
    );
    IngestionQueue {
        tx,
        state,
        capacity,
    }
}

/// The main worker loop for background document ingestion.
///
/// This function runs as a long-lived tokio task, waiting for jobs to arrive
/// on the receiver channel and processing them one by one.
///
/// # Arguments
///
/// * `memory` - The [`UnifiedMemory`] instance.
/// * `rx` - The receiver half of the job queue channel.
async fn ingestion_worker(
    memory: Arc<UnifiedMemory>,
    mut rx: mpsc::Receiver<IngestionJob>,
    state: IngestionState,
) {
    log::debug!("[memory:ingestion_queue] worker loop entered");

    // Continuously receive and process jobs until the channel is closed.
    while let Some(job) = rx.recv().await {
        let title = job.document.title.clone();
        let namespace = job.document.namespace.clone();
        let document_id = job.document_id.clone();

        log::debug!(
            "[memory:ingestion_queue] processing job: namespace={namespace}, \
             doc_id={document_id}, title={title}",
        );

        // Acquire the singleton lock so only one ingestion runs at a time
        // (covers both queue worker and synchronous callers sharing this
        // state). Decrement the pending-queue counter only after we hold the
        // lock — while we're blocked waiting on it the job is still queued.
        let _guard = state.acquire().await;
        state.dequeue();

        let queue_depth = state.snapshot().queue_depth;
        state.mark_running(&document_id, &title, &namespace);
        publish_global(DomainEvent::MemoryIngestionStarted {
            document_id: document_id.clone(),
            title: title.clone(),
            namespace: namespace.clone(),
            queue_depth,
        });

        let started = Instant::now();
        let success = match memory
            .extract_graph(&document_id, &job.document, &job.config)
            .await
        {
            Ok(result) => {
                log::info!(
                    "[memory:ingestion_queue] extracted namespace={namespace} \
                     doc_id={document_id} title={title} \
                     — entities={}, relations={}, chunks={}",
                    result.entity_count,
                    result.relation_count,
                    result.chunk_count,
                );
                true
            }
            Err(e) => {
                crate::core::observability::report_error(
                    &e,
                    "memory",
                    "ingestion_extract",
                    &[
                        ("namespace", namespace.as_str()),
                        ("doc_id", document_id.as_str()),
                    ],
                );
                false
            }
        };

        let elapsed_ms = started.elapsed().as_millis() as u64;
        let completed_at_ms = chrono::Utc::now().timestamp_millis();
        state.mark_completed(&document_id, success, completed_at_ms);
        publish_global(DomainEvent::MemoryIngestionCompleted {
            document_id,
            namespace,
            success,
            elapsed_ms,
            queue_depth: state.snapshot().queue_depth,
        });
    }

    log::info!("[memory:ingestion_queue] worker shut down (channel closed)");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn submit_when_full_returns_false() {
        // Capacity-1 channel, fill it, then submit another — exercises the Full branch.
        let state = IngestionState::new();
        let (tx, _rx) = mpsc::channel::<IngestionJob>(1);
        // Pre-fill the slot directly so submit() sees a full channel.
        tx.try_send(make_dummy_job("filler")).ok();

        let queue = IngestionQueue::from_parts(tx, state.clone(), 1);
        assert!(!queue.submit(make_dummy_job("overflow")));
        // Depth should be 0 — enqueue was rolled back.
        assert_eq!(state.snapshot().queue_depth, 0);
    }

    #[tokio::test]
    async fn submit_when_worker_gone_returns_false() {
        let state = IngestionState::new();
        let (tx, rx) = mpsc::channel::<IngestionJob>(4);
        drop(rx); // simulate worker shutdown

        let queue = IngestionQueue::from_parts(tx, state.clone(), 4);
        assert!(!queue.submit(make_dummy_job("orphan")));
        assert_eq!(state.snapshot().queue_depth, 0);
    }

    /// Verify that `submit()` succeeds again after transient backpressure is
    /// relieved (the channel drains and a slot becomes available).
    #[tokio::test]
    async fn submit_recovers_after_backpressure() {
        let state = IngestionState::new();
        // Capacity-2 channel so we can fill one slot and still have headroom
        // for the recovery submit.
        let (tx, mut rx) = mpsc::channel::<IngestionJob>(2);

        // Pre-fill both slots directly to force the Full condition on submit.
        tx.try_send(make_dummy_job("filler-a")).ok();
        tx.try_send(make_dummy_job("filler-b")).ok();

        let queue = IngestionQueue::from_parts(tx, state.clone(), 2);

        // Channel is now full — submit should return false and roll back depth.
        assert!(!queue.submit(make_dummy_job("overflow")));
        assert_eq!(
            state.snapshot().queue_depth,
            0,
            "depth must be 0 after rejected submit"
        );

        // Drain one slot to free up space.
        let _ = rx.recv().await;

        // submit() should now succeed and increment queue_depth by 1.
        assert!(queue.submit(make_dummy_job("recovered")));
        assert_eq!(
            state.snapshot().queue_depth,
            1,
            "depth must reflect the recovered enqueue"
        );
    }

    fn make_dummy_job(title: &str) -> IngestionJob {
        use crate::openhuman::memory::ingestion::MemoryIngestionConfig;
        use crate::openhuman::memory::store::types::NamespaceDocumentInput;
        IngestionJob {
            document_id: format!("doc-{title}"),
            document: NamespaceDocumentInput {
                namespace: "test".to_string(),
                key: title.to_string(),
                title: title.to_string(),
                content: "body".to_string(),
                source_type: "doc".to_string(),
                priority: "normal".to_string(),
                tags: vec![],
                metadata: serde_json::Value::Null,
                category: "core".to_string(),
                session_id: None,
                document_id: None,
            },
            config: MemoryIngestionConfig::default(),
        }
    }
}
