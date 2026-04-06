//! # Background Ingestion Queue
//!
//! Processes documents through the entity/relation extraction pipeline on a
//! dedicated worker thread. This ensures that `doc_put` callers never block
//! on the resource-intensive GLiNER model.
//!
//! The queue uses a `tokio::sync::mpsc` channel to decouple document submission
//! from the actual extraction process.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::openhuman::memory::ingestion::MemoryIngestionConfig;
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
        match self.tx.send(job) {
            Ok(()) => true,
            Err(e) => {
                log::warn!(
                    "[memory:ingestion_queue] failed to enqueue job (worker gone?): {}",
                    e.0.document.title,
                );
                false
            }
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
/// processing jobs sequentially so the GLiNER model is never loaded in
/// parallel (it is heavyweight).
pub fn start_worker(memory: Arc<UnifiedMemory>) -> IngestionQueue {
    // Create an unbounded channel for the ingestion jobs.
    let (tx, rx) = mpsc::unbounded_channel::<IngestionJob>();

    // Spawn the worker loop as a background task.
    tokio::spawn(ingestion_worker(memory, rx));

    log::info!("[memory:ingestion_queue] background worker started");
    IngestionQueue { tx }
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

        // Perform the graph extraction. This is the most resource-intensive step.
        match memory
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
            }
            Err(e) => {
                log::error!(
                    "[memory:ingestion_queue] extraction failed namespace={namespace} \
                     doc_id={document_id} title={title}: {e}",
                );
            }
        }
    }

    log::info!("[memory:ingestion_queue] worker shut down (channel closed)");
}
