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
/// This is a thin wrapper around a `tokio::sync::mpsc::UnboundedSender` and
/// can be cloned freely to be shared across multiple producers.
#[derive(Clone)]
pub struct IngestionQueue {
    /// Sender half of the job queue channel.
    tx: mpsc::UnboundedSender<IngestionJob>,
    /// Shared state — singleton lock, queue depth, status snapshot.
    state: IngestionState,
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
    /// worker has shut down (e.g., during application termination) and the
    /// job was dropped.
    pub fn submit(&self, job: IngestionJob) -> bool {
        self.state.enqueue();
        match self.tx.send(job) {
            Ok(()) => true,
            Err(e) => {
                // Worker is gone — undo the enqueue bump so depth stays accurate.
                self.state.dequeue();
                log::warn!(
                    "[memory:ingestion_queue] failed to enqueue job (worker gone?): {}",
                    e.0.document.title,
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
    let (tx, rx) = mpsc::unbounded_channel::<IngestionJob>();

    tokio::spawn(ingestion_worker(memory, rx, state.clone()));

    log::info!("[memory:ingestion_queue] background worker started");
    IngestionQueue { tx, state }
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
    mut rx: mpsc::UnboundedReceiver<IngestionJob>,
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
                log::error!(
                    "[memory:ingestion_queue] extraction failed namespace={namespace} \
                     doc_id={document_id} title={title}: {e}",
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
