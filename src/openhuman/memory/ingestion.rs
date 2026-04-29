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

#[path = "ingestion_parse.rs"]
mod ingestion_parse;
#[path = "ingestion_regex.rs"]
mod ingestion_regex;
#[path = "ingestion_rules.rs"]
mod ingestion_rules;
#[path = "ingestion_types.rs"]
mod ingestion_types;

pub use ingestion_types::{
    ExtractedEntity, ExtractedRelation, ExtractionMode, MemoryIngestionConfig,
    MemoryIngestionRequest, MemoryIngestionResult, DEFAULT_MEMORY_EXTRACTION_MODEL,
};

use ingestion_parse::{enrich_document_metadata, parse_document};
use ingestion_types::ParsedIngestion;
use serde_json::json;

use crate::openhuman::memory::store::types::NamespaceDocumentInput;
use crate::openhuman::memory::UnifiedMemory;

impl UnifiedMemory {
    pub async fn ingest_document(
        &self,
        request: MemoryIngestionRequest,
    ) -> Result<MemoryIngestionResult, String> {
        let parsed = parse_document(
            &request.document.content,
            &request.document.title,
            &request.config,
        )
        .await;
        let (enriched_input, tags) =
            enrich_document_metadata(&request.document, &parsed, &request.config);
        let namespace = Self::sanitize_namespace(&enriched_input.namespace);
        let document_id = self.upsert_document(enriched_input).await?;

        self.upsert_graph_relations(&namespace, &document_id, &parsed, &request.config)
            .await?;

        Ok(MemoryIngestionResult {
            document_id,
            namespace,
            model_name: request.config.model_name,
            extraction_mode: request.config.extraction_mode.as_str().to_string(),
            chunk_count: parsed.chunk_count,
            entity_count: parsed.entities.len(),
            relation_count: parsed.relations.len(),
            preference_count: parsed.preference_count,
            decision_count: parsed.decision_count,
            tags,
            entities: parsed.entities,
            relations: parsed.relations,
        })
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
        let parsed = parse_document(&document.content, &document.title, config).await;
        let namespace = Self::sanitize_namespace(&document.namespace);

        self.upsert_graph_relations(&namespace, document_id, &parsed, config)
            .await?;

        let (_, tags) = enrich_document_metadata(document, &parsed, config);

        Ok(MemoryIngestionResult {
            document_id: document_id.to_string(),
            namespace,
            model_name: config.model_name.clone(),
            extraction_mode: config.extraction_mode.as_str().to_string(),
            chunk_count: parsed.chunk_count,
            entity_count: parsed.entities.len(),
            relation_count: parsed.relations.len(),
            preference_count: parsed.preference_count,
            decision_count: parsed.decision_count,
            tags,
            entities: parsed.entities,
            relations: parsed.relations,
        })
    }

    /// Clear existing relations for the document then upsert all extracted
    /// relations into the namespace graph.
    async fn upsert_graph_relations(
        &self,
        namespace: &str,
        document_id: &str,
        parsed: &ParsedIngestion,
        config: &MemoryIngestionConfig,
    ) -> Result<(), String> {
        self.graph_remove_document_namespace(namespace, document_id)
            .await?;

        for relation in &parsed.relations {
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
#[path = "ingestion_tests.rs"]
mod tests;
