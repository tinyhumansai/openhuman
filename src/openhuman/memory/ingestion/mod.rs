//! Document ingestion and knowledge extraction for the OpenHuman memory system.
//!
//! This module provides the pipeline for taking raw unstructured text and
//! transforming it into structured memory. The process includes:
//! 1. **Chunking**: Splitting the document into manageable pieces.
//! 2. **Structured Extraction**: Using regex-based rules to identify known patterns
//!    (e.g., email headers, specific project labels).
//! 3. **Heuristic Extraction**: Using rule-based parsing to identify entities
//!    and their relationships.
//! 4. **Aggregation**: Resolving aliases, merging duplicates, and normalizing names.
//! 5. **Persistence**: Upserting the document, text chunks, and graph relations into
//!    the memory store.

pub mod queue;
pub mod state;

pub use queue::{IngestionJob, IngestionQueue, DEFAULT_QUEUE_CAPACITY};
pub use state::{IngestionState, IngestionStatusSnapshot};
pub use tinycortex::memory::ingest::{
    ExtractedEntity, ExtractedRelation, ExtractionMode, MemoryIngestionConfig,
    MemoryIngestionRequest, MemoryIngestionResult, DEFAULT_MEMORY_EXTRACTION_MODEL,
};

use serde_json::json;

use crate::openhuman::memory_store::types::NamespaceDocumentInput;
use crate::openhuman::memory_store::UnifiedMemory;

impl UnifiedMemory {
    /// Run the full ingestion pipeline for a document: parse + chunk + extract
    /// entities/relations, upsert the document row + vector chunks, and write
    /// the extracted relations into the namespace graph.
    pub async fn ingest_document(
        &self,
        request: MemoryIngestionRequest,
    ) -> Result<MemoryIngestionResult, String> {
        let (enriched_input, mut extraction) =
            tinycortex::memory::ingest::extract_enriched_document(
                &request.document,
                &request.config,
            );
        let namespace = Self::sanitize_namespace(&enriched_input.namespace);
        let document_id = self.upsert_document(enriched_input).await?;

        self.upsert_graph_relations(&namespace, &document_id, &extraction, &request.config)
            .await?;
        extraction.document_id = document_id;
        extraction.namespace = namespace;
        Ok(extraction)
    }

    /// Extract entities/relations and write them to the graph for a document
    /// that has already been stored via [`upsert_document`].
    ///
    /// This avoids the redundant second upsert that would happen if the
    /// background ingestion queue called [`ingest_document`] on an already-
    /// persisted document.
    pub async fn extract_graph(
        &self,
        document_id: &str,
        document: &NamespaceDocumentInput,
        config: &MemoryIngestionConfig,
    ) -> Result<MemoryIngestionResult, String> {
        let (_enriched, mut extraction) =
            tinycortex::memory::ingest::extract_enriched_document(document, config);
        let namespace = Self::sanitize_namespace(&document.namespace);

        self.upsert_graph_relations(&namespace, document_id, &extraction, config)
            .await?;
        extraction.document_id = document_id.to_string();
        extraction.namespace = namespace;
        Ok(extraction)
    }

    /// Clear existing relations for the document then upsert all extracted
    /// relations into the namespace graph.
    async fn upsert_graph_relations(
        &self,
        namespace: &str,
        document_id: &str,
        extraction: &MemoryIngestionResult,
        config: &MemoryIngestionConfig,
    ) -> Result<(), String> {
        self.graph_remove_document_namespace(namespace, document_id)
            .await?;

        for relation in &extraction.relations {
            let chunk_ids = relation
                .chunk_ids
                .iter()
                .filter_map(|chunk_id| chunk_id.strip_prefix("chunk:"))
                .map(|chunk_index| format!("{document_id}:{chunk_index}"))
                .collect::<Vec<_>>();

            let attrs = json!({
                "source": "ingestion",
                "model_name": config.model_name,
                "extraction_mode": config.extraction_mode.as_str(),
                "confidence": relation.confidence,
                "evidence_count": relation.evidence_count,
                "order_index": relation.order_index,
                "document_id": document_id,
                "document_ids": [document_id],
                "chunk_ids": chunk_ids,
                "entity_types": {
                    "subject": relation.subject_type,
                    "object": relation.object_type,
                },
                "metadata": relation.metadata,
            });

            self.graph_upsert_namespace(
                namespace,
                &relation.subject,
                &relation.predicate,
                &relation.object,
                &attrs,
            )
            .await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests;
