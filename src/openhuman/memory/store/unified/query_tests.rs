use std::sync::Arc;

use serde_json::json;
use tempfile::TempDir;

use crate::openhuman::memory::{embeddings::NoopEmbedding, NamespaceDocumentInput, UnifiedMemory};

#[tokio::test]
async fn graph_duplicate_upsert_aggregates_evidence_count() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    memory
        .graph_upsert_namespace(
            "team",
            "alice",
            "owns",
            "atlas",
            &json!({"document_id": "doc-1"}),
        )
        .await
        .unwrap();
    memory
        .graph_upsert_namespace(
            "team",
            "ALICE",
            "OWNS",
            "ATLAS",
            &json!({"document_ids": ["doc-2"], "evidence_count": 2}),
        )
        .await
        .unwrap();

    let rows = memory.graph_relations_for_scope("team").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].subject, "ALICE");
    assert_eq!(rows[0].predicate, "OWNS");
    assert_eq!(rows[0].object, "ATLAS");
    assert_eq!(rows[0].evidence_count, 3);
    assert_eq!(rows[0].document_ids, vec!["doc-1", "doc-2"]);
}

#[tokio::test]
async fn query_namespace_uses_graph_signal_for_document_ranking() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let document_id = memory
        .upsert_document(NamespaceDocumentInput {
            namespace: "team".to_string(),
            key: "atlas-status".to_string(),
            title: "Atlas status".to_string(),
            content: "Project Atlas is currently owned by Alice.".to_string(),
            source_type: "doc".to_string(),
            priority: "high".to_string(),
            tags: vec!["decision".to_string()],
            metadata: json!({"kind": "decision"}),
            category: "core".to_string(),
            session_id: None,
            document_id: None,
        })
        .await
        .unwrap();

    memory
        .graph_upsert_namespace(
            "team",
            "Alice",
            "owns",
            "Atlas",
            &json!({"document_id": document_id}),
        )
        .await
        .unwrap();

    let results = memory
        .query_namespace_ranked("team", "who owns atlas", 5)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].key, "atlas-status");
    assert!(results[0].score > 0.5);
}

#[tokio::test]
async fn recall_namespace_memories_includes_namespace_kv() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    memory
        .kv_set_namespace(
            "team",
            "user.preference.theme",
            &json!({"value": "sunrise", "kind": "preference"}),
        )
        .await
        .unwrap();

    let hits = memory.recall_namespace_memories("team", 5).await.unwrap();
    assert!(hits
        .iter()
        .any(|hit| matches!(hit.kind, crate::openhuman::memory::MemoryItemKind::Kv)));
}

#[tokio::test]
async fn query_returns_episodic_hits_when_available() {
    use crate::openhuman::memory::store::fts5::{self, EpisodicEntry};

    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    // Insert an episodic entry that matches the query.
    fts5::episodic_insert(
        &memory.conn,
        &EpisodicEntry {
            id: None,
            session_id: "sess-1".into(),
            timestamp: 1000.0,
            role: "user".into(),
            content: "I have been using Tokio for async Rust development".into(),
            lesson: None,
            tool_calls_json: None,
            cost_microdollars: 0,
        },
    )
    .unwrap();

    let hits = memory
        .query_namespace_hits("global", "Tokio async Rust", 10)
        .await
        .unwrap();

    let episodic_hits: Vec<_> = hits
        .iter()
        .filter(|h| h.kind == crate::openhuman::memory::MemoryItemKind::Episodic)
        .collect();
    assert!(
        !episodic_hits.is_empty(),
        "Expected at least one Episodic hit for 'Tokio async Rust'"
    );
}

#[tokio::test]
async fn query_returns_event_hits_when_available() {
    use crate::openhuman::memory::store::events::{self, EventRecord, EventType};

    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    // Insert an event that matches the query.
    events::event_insert(
        &memory.conn,
        &EventRecord {
            event_id: "evt-q-1".into(),
            segment_id: "seg-q-1".into(),
            session_id: "s1".into(),
            namespace: "global".into(),
            event_type: EventType::Decision,
            content: "We decided to use PostgreSQL as the primary database".into(),
            subject: Some("database choice".into()),
            timestamp_ref: None,
            confidence: 0.85,
            embedding: None,
            source_turn_ids: None,
            created_at: 1000.0,
        },
    )
    .unwrap();

    let hits = memory
        .query_namespace_hits("global", "PostgreSQL database", 10)
        .await
        .unwrap();

    let event_hits: Vec<_> = hits
        .iter()
        .filter(|h| h.kind == crate::openhuman::memory::MemoryItemKind::Event)
        .collect();
    assert!(
        !event_hits.is_empty(),
        "Expected at least one Event hit for 'PostgreSQL database'"
    );
}

#[tokio::test]
async fn query_episodic_hits_have_correct_kind() {
    use crate::openhuman::memory::store::fts5::{self, EpisodicEntry};

    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    fts5::episodic_insert(
        &memory.conn,
        &EpisodicEntry {
            id: None,
            session_id: "sess-kind".into(),
            timestamp: 2000.0,
            role: "assistant".into(),
            content: "The deployment pipeline uses GitHub Actions for CI".into(),
            lesson: Some("CI runs on push to main".into()),
            tool_calls_json: None,
            cost_microdollars: 0,
        },
    )
    .unwrap();

    let hits = memory
        .query_namespace_hits("global", "GitHub Actions deployment", 10)
        .await
        .unwrap();

    for hit in hits.iter().filter(|h| h.id.starts_with("episodic:")) {
        assert_eq!(
            hit.kind,
            crate::openhuman::memory::MemoryItemKind::Episodic,
            "Hits with 'episodic:' id prefix must have kind Episodic"
        );
    }
}

#[tokio::test]
async fn query_supporting_relations_contain_entity_types() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let document_id = memory
        .upsert_document(NamespaceDocumentInput {
            namespace: "team".to_string(),
            key: "alice-google".to_string(),
            title: "Alice at Google".to_string(),
            content: "Alice works on Project Alpha at Google.".to_string(),
            source_type: "doc".to_string(),
            priority: "high".to_string(),
            tags: vec!["decision".to_string()],
            metadata: json!({}),
            category: "core".to_string(),
            session_id: None,
            document_id: None,
        })
        .await
        .unwrap();

    // Upsert graph relations with entity types in attrs (mimics ingestion pipeline).
    memory
        .graph_upsert_namespace(
            "team",
            "Alice",
            "WORKS_FOR",
            "Google",
            &json!({
                "document_id": document_id,
                "entity_types": {
                    "subject": "PERSON",
                    "object": "ORGANIZATION"
                }
            }),
        )
        .await
        .unwrap();
    memory
        .graph_upsert_namespace(
            "team",
            "Alice",
            "OWNS",
            "Project Alpha",
            &json!({
                "document_id": document_id,
                "entity_types": {
                    "subject": "PERSON",
                    "object": "PROJECT"
                }
            }),
        )
        .await
        .unwrap();

    // Query path: entity types should appear in supporting_relations attrs.
    let hits = memory
        .query_namespace_hits("team", "Alice", 5)
        .await
        .unwrap();
    assert!(!hits.is_empty(), "should return at least one hit");

    let hit = &hits[0];
    assert!(
        !hit.supporting_relations.is_empty(),
        "hit should have supporting relations"
    );

    // Verify entity types are present in the attrs of supporting relations.
    for relation in &hit.supporting_relations {
        let entity_types = relation.attrs.get("entity_types");
        assert!(
            entity_types.is_some(),
            "relation {} -[{}]-> {} should have entity_types in attrs",
            relation.subject,
            relation.predicate,
            relation.object
        );
        let et = entity_types.unwrap();
        let subject_type = et.get("subject").and_then(|v| v.as_str());
        assert_eq!(
            subject_type,
            Some("PERSON"),
            "subject_type should be PERSON for Alice"
        );
    }

    // Recall path: entity types should also appear.
    let recall_hits = memory.recall_namespace_memories("team", 5).await.unwrap();
    assert!(!recall_hits.is_empty(), "recall should return hits");

    let recall_hit = &recall_hits[0];
    assert!(
        !recall_hit.supporting_relations.is_empty(),
        "recall hit should have supporting relations"
    );
    for relation in &recall_hit.supporting_relations {
        let entity_types = relation.attrs.get("entity_types");
        assert!(
            entity_types.is_some(),
            "recall relation should have entity_types in attrs"
        );
    }
}

#[tokio::test]
async fn format_context_text_includes_entity_types() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    let document_id = memory
        .upsert_document(NamespaceDocumentInput {
            namespace: "team".to_string(),
            key: "atlas-status".to_string(),
            title: "Atlas status".to_string(),
            content: "Project Atlas is owned by Alice at Google.".to_string(),
            source_type: "doc".to_string(),
            priority: "high".to_string(),
            tags: vec!["decision".to_string()],
            metadata: json!({}),
            category: "core".to_string(),
            session_id: None,
            document_id: None,
        })
        .await
        .unwrap();

    memory
        .graph_upsert_namespace(
            "team",
            "Alice",
            "OWNS",
            "Atlas",
            &json!({
                "document_id": document_id,
                "entity_types": {
                    "subject": "PERSON",
                    "object": "PROJECT"
                }
            }),
        )
        .await
        .unwrap();

    let context = memory
        .query_namespace_context_data("team", "who owns atlas", 5)
        .await
        .unwrap();
    // Entity names are normalized to uppercase during graph upsert.
    assert!(
        context.context_text.contains("ALICE (PERSON)"),
        "context_text should include entity type for Alice, got: {}",
        context.context_text
    );
    assert!(
        context.context_text.contains("ATLAS (PROJECT)"),
        "context_text should include entity type for Atlas, got: {}",
        context.context_text
    );
}
