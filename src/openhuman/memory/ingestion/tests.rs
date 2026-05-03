use std::sync::Arc;

use serde_json::json;
use tempfile::TempDir;

use crate::openhuman::memory::{
    embeddings::NoopEmbedding, MemoryIngestionConfig, MemoryIngestionRequest,
    NamespaceDocumentInput, UnifiedMemory,
};

/// Test config for the heuristic-only ingestion pipeline.
fn ci_safe_config() -> MemoryIngestionConfig {
    MemoryIngestionConfig::default()
}

fn fixture(path: &str) -> String {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(
        base.join("tests")
            .join("fixtures")
            .join("ingestion")
            .join(path),
    )
    .expect("fixture should load")
}

#[tokio::test]
async fn gmail_fixture_ingestion_recovers_required_signals() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();
    let result = memory
        .ingest_document(MemoryIngestionRequest {
            document: NamespaceDocumentInput {
                namespace: "skill-gmail".to_string(),
                key: "gmail-thread-memory-integration".to_string(),
                title: "Memory integration plan for OpenHuman desktop".to_string(),
                content: fixture("gmail_thread_example.txt"),
                source_type: "gmail".to_string(),
                priority: "high".to_string(),
                tags: Vec::new(),
                metadata: json!({}),
                category: "core".to_string(),
                session_id: None,
                document_id: None,
            },
            config: ci_safe_config(),
        })
        .await
        .unwrap();

    assert!(result
        .entities
        .iter()
        .any(|entity| entity.name == "SANIL JAIN"));
    assert!(result
        .entities
        .iter()
        .any(|entity| entity.name == "RAVI KULKARNI"));
    assert!(result
        .entities
        .iter()
        .any(|entity| entity.name == "ASHA MEHTA"));
    assert!(result
        .entities
        .iter()
        .any(|entity| entity.name == "OPENHUMAN"));
    assert!(result
        .relations
        .iter()
        .any(|relation| relation.subject == "OPENHUMAN"
            && relation.predicate == "USES"
            && relation.object.contains("JSON-RPC")));
    assert!(result
        .relations
        .iter()
        .any(|relation| relation.subject == "RAVI KULKARNI" && relation.predicate == "OWNS"));
    assert!(result.preference_count >= 1);
    assert!(result.decision_count >= 1);

    let context = memory
        .query_namespace_context_data("skill-gmail", "who owns the rust memory api alignment", 5)
        .await
        .unwrap();
    assert!(context
        .hits
        .iter()
        .flat_map(|hit| hit.supporting_relations.iter())
        .any(|relation| relation.subject == "RAVI KULKARNI" && relation.predicate == "OWNS"));

    let recall = memory
        .recall_namespace_context_data("skill-gmail", 5)
        .await
        .unwrap();
    assert!(!recall.context_text.is_empty());
    assert!(recall
        .hits
        .iter()
        .any(|hit| hit.content.contains("OpenHuman") || hit.content.contains("JSON-RPC")));
    assert!(recall
        .hits
        .iter()
        .any(|hit| !hit.supporting_relations.is_empty()));

    let memories = memory
        .recall_namespace_memories("skill-gmail", 5)
        .await
        .unwrap();
    assert!(memories.iter().any(|hit| hit.content.contains("JSON-RPC")));
    assert!(memories
        .iter()
        .any(|hit| matches!(hit.kind, crate::openhuman::memory::MemoryItemKind::Document)));
    assert!(memories
        .iter()
        .any(|hit| !hit.supporting_relations.is_empty()));
}

#[tokio::test]
async fn notion_fixture_ingestion_recovers_required_signals() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();
    let result = memory
        .ingest_document(MemoryIngestionRequest {
            document: NamespaceDocumentInput {
                namespace: "skill-notion".to_string(),
                key: "notion-roadmap-memory-layer".to_string(),
                title: "OpenHuman Memory Layer Roadmap".to_string(),
                content: fixture("notion_page_example.txt"),
                source_type: "notion".to_string(),
                priority: "high".to_string(),
                tags: Vec::new(),
                metadata: json!({}),
                category: "core".to_string(),
                session_id: None,
                document_id: None,
            },
            config: ci_safe_config(),
        })
        .await
        .unwrap();

    assert!(result
        .entities
        .iter()
        .any(|entity| entity.name == "OPENHUMAN"));
    assert!(result
        .entities
        .iter()
        .any(|entity| entity.name == "SANIL JAIN"));
    assert!(result
        .relations
        .iter()
        .any(|relation| relation.subject == "OPENHUMAN"
            && relation.predicate == "USES"
            && relation.object.contains("JSON-RPC")));
    assert!(result
        .relations
        .iter()
        .any(|relation| relation.subject == "CORE CONTRACT LOCKED"
            && relation.predicate == "HAS_DEADLINE"));
    assert!(result
        .relations
        .iter()
        .any(|relation| relation.subject == "SANIL JAIN" && relation.predicate == "PREFERS"));
    assert!(result.preference_count >= 1);
    assert!(result.decision_count >= 1);

    let graph_rows = memory
        .graph_query_namespace("skill-notion", Some("OPENHUMAN"), Some("USES"))
        .await
        .unwrap();
    assert!(!graph_rows.is_empty());

    let context = memory
        .query_namespace_context_data(
            "skill-notion",
            "who prefers core-first delivery over ui-first delivery",
            5,
        )
        .await
        .unwrap();
    assert!(context
        .hits
        .iter()
        .flat_map(|hit| hit.supporting_relations.iter())
        .any(|relation| relation.subject == "SANIL JAIN" && relation.predicate == "PREFERS"));

    let recall = memory
        .recall_namespace_context_data("skill-notion", 5)
        .await
        .unwrap();
    assert!(!recall.context_text.is_empty());
    assert!(recall
        .hits
        .iter()
        .any(|hit| hit.content.contains("OpenHuman")));

    let memories = memory
        .recall_namespace_memories("skill-notion", 5)
        .await
        .unwrap();
    assert!(memories
        .iter()
        .any(|hit| hit.content.contains("OpenHuman") || hit.content.contains("core-first")));
    assert!(memories
        .iter()
        .any(|hit| matches!(hit.kind, crate::openhuman::memory::MemoryItemKind::Document)));
    assert!(memories
        .iter()
        .any(|hit| !hit.supporting_relations.is_empty()));
}
