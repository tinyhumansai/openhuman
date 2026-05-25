//! # Background Ingestion Queue
//!
//! Processes documents through the entity/relation extraction pipeline on a
//! dedicated worker thread. This ensures that `doc_put` callers never block
//! on the heavier parsing and graph-write path.
//!
//! The queue uses a bounded `tokio::sync::mpsc` channel
//! ([`DEFAULT_QUEUE_CAPACITY`]) to decouple document submission from the
//! actual extraction process. Producers call [`IngestionQueue::submit`],
//! which is non-blocking; when the buffer is full the job is dropped with a
//! warn-level log so a runaway producer cannot grow the queue without bound
//! and exhaust process memory.

use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;

use super::state::IngestionState;
use super::MemoryIngestionConfig;
use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::memory_store::{NamespaceDocumentInput, UnifiedMemory};

/// Default capacity of the ingestion job channel.
///
/// Producers (`put_doc`, `store_skill_sync`) push jobs into this channel
/// without blocking; the worker drains them one-at-a-time under the
/// `IngestionState` singleton lock because the local extraction LLM cannot
/// run concurrently. A buggy or compromised producer can submit jobs much
/// faster than the worker drains them, so the channel must enforce an
/// explicit cap or the queue grows without bound and exhausts process
/// memory (each [`IngestionJob`] holds an owned document body).
///
/// 512 is a deliberate middle ground: it absorbs reasonable bulk-import
/// bursts (e.g. backfilling a Notion workspace or a long Slack history)
/// without letting a runaway loop balloon RSS — at typical document sizes
/// of 1–100 KB the in-flight buffer caps below ~50 MB.
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
/// This is a thin wrapper around a bounded `tokio::sync::mpsc::Sender` and
/// can be cloned freely to be shared across multiple producers. The bound
/// (see [`DEFAULT_QUEUE_CAPACITY`]) protects the core from runaway
/// producers; once the buffer is full, [`Self::submit`] returns `false`
/// instead of blocking or growing the queue.
#[derive(Clone)]
pub struct IngestionQueue {
    /// Sender half of the bounded job queue channel.
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
    /// queue is full (capacity reached) or the worker has shut down (e.g.,
    /// during application termination). In both drop cases the job is not
    /// persisted into the extraction pipeline — the underlying document
    /// upsert that the caller already performed is unaffected. The queue
    /// depth counter is restored before returning so the
    /// `memory_ingestion_status` RPC stays accurate.
    pub fn submit(&self, job: IngestionJob) -> bool {
        self.state.enqueue();
        match self.tx.try_send(job) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(dropped)) => {
                // Channel is at capacity — log loudly so observability can
                // surface the drop, then undo the enqueue bump so the queue
                // depth gauge does not drift upward forever under sustained
                // overflow. Include the stable `document_id` so the warn
                // line is the breadcrumb back to the upserted document
                // whose graph-extraction follow-up was skipped.
                self.state.dequeue();
                log::warn!(
                    "[memory:ingestion_queue] dropping job: queue at capacity (cap={}) doc_id={} namespace={} title={}",
                    self.capacity,
                    dropped.document_id,
                    dropped.document.namespace,
                    dropped.document.title,
                );
                false
            }
            Err(mpsc::error::TrySendError::Closed(dropped)) => {
                // Worker is gone — same accounting as the full case, but a
                // different reason worth distinguishing in logs because it
                // means the entire pipeline is dead, not just over-pressure.
                self.state.dequeue();
                log::warn!(
                    "[memory:ingestion_queue] dropping job: worker channel closed (shutdown?) doc_id={} namespace={} title={}",
                    dropped.document_id,
                    dropped.document.namespace,
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
/// snapshot as the queue worker. Uses [`DEFAULT_QUEUE_CAPACITY`].
pub fn start_worker_with_state(
    memory: Arc<UnifiedMemory>,
    state: IngestionState,
) -> IngestionQueue {
    start_worker_with_capacity(memory, state, DEFAULT_QUEUE_CAPACITY)
}

/// Start a worker with an explicit channel capacity. Exposed so unit tests
/// can drive the at-capacity drop path deterministically without faking a
/// slow worker.
///
/// # Panics
///
/// Panics if `capacity == 0`. `tokio::sync::mpsc::channel` itself panics on
/// a zero buffer, but the message is cryptic; the explicit guard here turns
/// the misuse into a clear, grep-friendly assertion at the call site.
pub(crate) fn start_worker_with_capacity(
    memory: Arc<UnifiedMemory>,
    state: IngestionState,
    capacity: usize,
) -> IngestionQueue {
    assert!(
        capacity > 0,
        "ingestion queue capacity must be greater than zero"
    );
    let (tx, rx) = mpsc::channel::<IngestionJob>(capacity);

    tokio::spawn(ingestion_worker(memory, rx, state.clone()));

    log::info!("[memory:ingestion_queue] background worker started capacity={capacity}");
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
    //! Channel-bound tests. These build an [`IngestionQueue`] from a raw
    //! `mpsc::channel` without spawning a worker — that lets the suite drive
    //! the at-capacity and channel-closed branches deterministically without
    //! standing up a real `UnifiedMemory` or contending with a draining task.
    use super::*;

    use serde_json::json;

    fn fixture_job(title: &str) -> IngestionJob {
        IngestionJob {
            document_id: format!("doc-{title}"),
            document: NamespaceDocumentInput {
                namespace: "skill-test".to_string(),
                key: title.to_string(),
                title: title.to_string(),
                content: "body".to_string(),
                source_type: "doc".to_string(),
                priority: "medium".to_string(),
                tags: Vec::new(),
                metadata: json!({}),
                category: "core".to_string(),
                session_id: None,
                document_id: None,
            },
            config: MemoryIngestionConfig::default(),
        }
    }

    #[tokio::test]
    async fn submit_succeeds_until_capacity_then_drops() {
        let state = IngestionState::new();
        let (tx, _rx) = mpsc::channel::<IngestionJob>(2);
        let queue = IngestionQueue::from_parts(tx, state.clone(), 2);

        assert!(queue.submit(fixture_job("a")), "first submit must enqueue");
        assert!(queue.submit(fixture_job("b")), "second submit must enqueue");

        // Channel is now full. tokio's bounded mpsc reserves one slot per
        // permit, so capacity=2 means at most two pending; the third must be
        // rejected with `false`.
        assert!(
            !queue.submit(fixture_job("c")),
            "submit at capacity must return false (drop)"
        );

        // queue_depth must reflect only the accepted jobs — the drop path
        // is required to decrement so the status RPC does not drift upward.
        assert_eq!(
            state.snapshot().queue_depth,
            2,
            "queue_depth must roll back on overflow drop"
        );
    }

    #[tokio::test]
    async fn submit_recovers_after_drain() {
        let state = IngestionState::new();
        let (tx, mut rx) = mpsc::channel::<IngestionJob>(1);
        let queue = IngestionQueue::from_parts(tx, state.clone(), 1);

        assert!(queue.submit(fixture_job("first")));
        assert!(
            !queue.submit(fixture_job("over")),
            "second submit at cap=1 must drop"
        );

        // Drain the receiver to free a slot.
        let pulled = rx.try_recv().expect("first job must be readable");
        assert_eq!(pulled.document.title, "first");
        // Mirror the worker's accounting (queue depth -> dequeue) so the
        // post-drain snapshot does not look like a leftover queued job.
        state.dequeue();

        assert!(
            queue.submit(fixture_job("after-drain")),
            "submit after drain must enqueue"
        );
        assert_eq!(state.snapshot().queue_depth, 1);
    }

    #[tokio::test]
    async fn submit_after_worker_gone_returns_false() {
        let state = IngestionState::new();
        let (tx, rx) = mpsc::channel::<IngestionJob>(4);
        drop(rx); // simulate worker task exiting and dropping its receiver
        let queue = IngestionQueue::from_parts(tx, state.clone(), 4);

        assert!(
            !queue.submit(fixture_job("orphan")),
            "submit must return false once the receiver is dropped"
        );
        assert_eq!(
            state.snapshot().queue_depth,
            0,
            "channel-closed drop path must roll the depth counter back"
        );
    }

    #[test]
    fn default_queue_capacity_is_bounded_and_reasonable() {
        // Guardrail so future changes don't accidentally regress to an
        // arbitrarily large default (or `usize::MAX`) without thinking about
        // the producer-side memory bound.
        assert!(DEFAULT_QUEUE_CAPACITY > 0);
        assert!(
            DEFAULT_QUEUE_CAPACITY <= 8 * 1024,
            "default capacity is the memory ceiling under sustained overflow — keep it tight"
        );
    }

    /// Zero capacity would otherwise panic from inside
    /// `tokio::sync::mpsc::channel` with a cryptic Tokio-internal message
    /// (`mpsc bounded channel requires buffer > 0`) — the explicit guard in
    /// [`start_worker_with_capacity`] turns that into a clear, grep-friendly
    /// assertion at the call site so misuse fails fast with an actionable
    /// message instead of looking like a Tokio bug.
    #[tokio::test]
    #[should_panic(expected = "ingestion queue capacity must be greater than zero")]
    async fn start_worker_rejects_zero_capacity() {
        use crate::openhuman::embeddings::NoopEmbedding;
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();
        // Panic must surface from our own assert, not from the Tokio
        // channel constructor on the line after — that's the contract this
        // test pins.
        let _ = start_worker_with_capacity(Arc::new(memory), IngestionState::new(), 0);
    }
}
