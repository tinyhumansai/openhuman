//! Tests for the `query` module — hybrid retrieval scoring.

use std::sync::Arc;

use serde_json::json;
use tempfile::TempDir;

use crate::openhuman::embeddings::NoopEmbedding;
use crate::openhuman::memory::Memory;
use crate::openhuman::memory_store::{NamespaceDocumentInput, UnifiedMemory};

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
            taint: crate::openhuman::memory::MemoryTaint::Internal,
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
async fn query_scores_relation_entities_found_in_document_content() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    memory
        .upsert_document(NamespaceDocumentInput {
            namespace: "team".to_string(),
            key: "atlas-background".to_string(),
            title: "Atlas background".to_string(),
            content: "Alice coordinates the Atlas rollout notes.".to_string(),
            source_type: "doc".to_string(),
            priority: "high".to_string(),
            tags: vec!["project".to_string()],
            metadata: json!({}),
            category: "core".to_string(),
            session_id: None,
            document_id: None,
            taint: crate::openhuman::memory::MemoryTaint::Internal,
        })
        .await
        .unwrap();

    memory
        .graph_upsert_namespace("team", "Alice", "owns", "Atlas", &json!({}))
        .await
        .unwrap();

    let hits = memory
        .query_namespace_hits("team", "who owns atlas", 5)
        .await
        .unwrap();
    let hit = hits
        .iter()
        .find(|hit| hit.key == "atlas-background")
        .expect("document content should receive graph relevance");

    assert!(hit.score_breakdown.graph_relevance > 0.0);
    assert!(!hit.supporting_relations.is_empty());
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
        .any(|hit| matches!(hit.kind, crate::openhuman::memory_store::MemoryItemKind::Kv)));
}

#[tokio::test]
async fn query_returns_episodic_hits_when_available() {
    use crate::openhuman::memory_store::fts5::{self, EpisodicEntry};

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
        .filter(|h| h.kind == crate::openhuman::memory_store::MemoryItemKind::Episodic)
        .collect();
    assert!(
        !episodic_hits.is_empty(),
        "Expected at least one Episodic hit for 'Tokio async Rust'"
    );
}

#[tokio::test]
async fn query_returns_event_hits_when_available() {
    use crate::openhuman::memory_store::events::{self, EventRecord, EventType};

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
        .filter(|h| h.kind == crate::openhuman::memory_store::MemoryItemKind::Event)
        .collect();
    assert!(
        !event_hits.is_empty(),
        "Expected at least one Event hit for 'PostgreSQL database'"
    );
}

#[tokio::test]
async fn query_episodic_hits_have_correct_kind() {
    use crate::openhuman::memory_store::fts5::{self, EpisodicEntry};

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
            crate::openhuman::memory_store::MemoryItemKind::Episodic,
            "Hits with 'episodic:' id prefix must have kind Episodic"
        );
    }
}

/// Episodic FTS relevance is derived from each hit's rank position
/// (`1.0 - idx / len`). With two equally-fresh matches the only
/// differentiator is rank, so the relevance scores must be exactly the
/// per-position values {1.0, 0.5}. This pins the position-indexing math
/// for n > 1 — the single-entry tests above cannot, since idx is always 0.
#[tokio::test]
async fn query_episodic_relevance_tracks_rank_position() {
    use crate::openhuman::memory_store::fts5::{self, EpisodicEntry};

    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

    // Two distinct entries, identical timestamp (equal freshness), both
    // matching the query so episodic_hits has len == 2.
    for content in [
        "I have been using Tokio for async Rust development",
        "Tokio async runtime powers our backend services",
    ] {
        fts5::episodic_insert(
            &memory.conn,
            &EpisodicEntry {
                id: None,
                session_id: "sess-rank".into(),
                timestamp: 1000.0,
                role: "user".into(),
                content: content.into(),
                lesson: None,
                tool_calls_json: None,
                cost_microdollars: 0,
            },
        )
        .unwrap();
    }

    let hits = memory
        .query_namespace_hits("global", "Tokio async", 10)
        .await
        .unwrap();

    let mut relevances: Vec<f64> = hits
        .iter()
        .filter(|h| h.kind == crate::openhuman::memory_store::MemoryItemKind::Episodic)
        .map(|h| h.score_breakdown.episodic_relevance)
        .collect();
    relevances.sort_by(|a, b| a.partial_cmp(b).unwrap());

    assert_eq!(
        relevances.len(),
        2,
        "expected exactly two episodic hits, got {relevances:?}"
    );
    assert!(
        (relevances[0] - 0.5).abs() < 1e-9 && (relevances[1] - 1.0).abs() < 1e-9,
        "episodic relevance must be {{0.5, 1.0}} for two-element rank order, got {relevances:?}"
    );
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
            taint: crate::openhuman::memory::MemoryTaint::Internal,
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
            taint: crate::openhuman::memory::MemoryTaint::Internal,
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

// ── vector_chunks model-signature guard (embedding model-swap safety) ─────────

use async_trait::async_trait;

use crate::openhuman::embeddings::EmbeddingProvider;

/// Embedder stub that returns a fixed vector for any text, with a controllable
/// name + dimension so tests can produce distinct embedding signatures and
/// dimensionalities.
struct StubEmbedder {
    name: &'static str,
    vector: Vec<f32>,
}

#[async_trait]
impl EmbeddingProvider for StubEmbedder {
    fn name(&self) -> &str {
        self.name
    }
    fn model_id(&self) -> &str {
        self.name
    }
    fn dimensions(&self) -> usize {
        self.vector.len()
    }
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| self.vector.clone()).collect())
    }
}

fn pref_doc(key: &str, content: &str) -> NamespaceDocumentInput {
    NamespaceDocumentInput {
        namespace: "user_pref".to_string(),
        key: key.to_string(),
        title: key.to_string(),
        content: content.to_string(),
        source_type: "pref".to_string(),
        priority: "medium".to_string(),
        tags: vec![],
        metadata: json!({}),
        category: "core".to_string(),
        session_id: None,
        document_id: None,
        taint: crate::openhuman::memory::MemoryTaint::Internal,
    }
}

#[tokio::test]
async fn upsert_tags_vector_chunks_with_signature_and_dim() {
    let tmp = TempDir::new().unwrap();
    let embedder = Arc::new(StubEmbedder {
        name: "stub-a",
        vector: vec![1.0, 0.0, 0.0],
    });
    let memory = UnifiedMemory::new(tmp.path(), embedder.clone(), None).unwrap();

    memory
        .upsert_document(pref_doc("reply_language", "Reply in British English."))
        .await
        .unwrap();

    // The stored chunk carries the active model's signature.
    let chunks = memory.load_chunks_for_scope("user_pref").await.unwrap();
    assert_eq!(chunks.len(), 1, "expected exactly one chunk for the doc");
    assert_eq!(
        chunks[0].model_signature.as_deref(),
        Some(embedder.signature().as_str()),
        "chunk should be tagged with the embedder signature"
    );

    // The `dim` column reflects the embedding dimensionality.
    let dim: Option<i64> = memory
        .conn
        .lock()
        .query_row(
            "SELECT dim FROM vector_chunks WHERE namespace = 'user_pref' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(dim, Some(3));
}

#[tokio::test]
async fn vector_recall_excludes_other_model_signature() {
    let tmp = TempDir::new().unwrap();

    // Write under model A.
    let emb_a = Arc::new(StubEmbedder {
        name: "model-a",
        vector: vec![1.0, 0.0, 0.0],
    });
    {
        let memory = UnifiedMemory::new(tmp.path(), emb_a.clone(), None).unwrap();
        memory
            .upsert_document(pref_doc("p1", "formal tone for emails to my manager"))
            .await
            .unwrap();

        // Same model → the vector is scored.
        let chunks = memory.load_chunks_for_scope("user_pref").await.unwrap();
        let scores = memory
            .query_vector_scores_from_chunks(&chunks, "email tone")
            .await
            .unwrap();
        assert!(!scores.is_empty(), "same-signature vectors must be scored");
    }

    // Reopen the same DB under a DIFFERENT model (swap), same dim + vector.
    let emb_b = Arc::new(StubEmbedder {
        name: "model-b",
        vector: vec![1.0, 0.0, 0.0],
    });
    let memory_b = UnifiedMemory::new(tmp.path(), emb_b, None).unwrap();
    let chunks = memory_b.load_chunks_for_scope("user_pref").await.unwrap();
    assert_eq!(chunks.len(), 1, "the chunk persists across reopen");
    let scores = memory_b
        .query_vector_scores_from_chunks(&chunks, "email tone")
        .await
        .unwrap();
    assert!(
        scores.is_empty(),
        "vectors from a different embedding model must be excluded, not compared as garbage"
    );
}

#[tokio::test]
async fn vector_recall_skips_dimension_mismatch_for_untagged_rows() {
    let tmp = TempDir::new().unwrap();
    // Active model produces 4-dim vectors.
    let emb = Arc::new(StubEmbedder {
        name: "model-a",
        vector: vec![1.0, 0.0, 0.0, 0.0],
    });
    let memory = UnifiedMemory::new(tmp.path(), emb, None).unwrap();

    // Insert a legacy chunk: NULL signature, 2-dim vector (a pre-tagging row left
    // behind by a dimension-changing model swap).
    let legacy_vec = UnifiedMemory::vec_to_bytes(&[1.0_f32, 0.0]);
    memory
        .conn
        .lock()
        .execute(
            "INSERT INTO vector_chunks
               (namespace, document_id, chunk_id, text, embedding, metadata_json, created_at, updated_at, model_signature, dim)
             VALUES ('user_pref','legacy','legacy:0','old pref',?1,'{}',0,0,NULL,2)",
            rusqlite::params![legacy_vec],
        )
        .unwrap();

    let chunks = memory.load_chunks_for_scope("user_pref").await.unwrap();
    assert_eq!(chunks.len(), 1);
    assert!(
        chunks[0].model_signature.is_none(),
        "legacy row should have no signature"
    );
    let scores = memory
        .query_vector_scores_from_chunks(&chunks, "old pref")
        .await
        .unwrap();
    assert!(
        scores.is_empty(),
        "dimension-mismatched legacy vectors must be skipped, not scored 0"
    );
}

// ── recall_relevant_by_vector — Lane B situational-pref relevance gate ─────────

/// Embedder whose vector depends on keywords in the text, so a query can be
/// genuinely relevant (high cosine) or irrelevant (zero) to a stored pref.
struct KeywordEmbedder;

#[async_trait]
impl EmbeddingProvider for KeywordEmbedder {
    fn name(&self) -> &str {
        "keyword-stub"
    }
    fn model_id(&self) -> &str {
        "keyword-stub"
    }
    fn dimensions(&self) -> usize {
        2
    }
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let lower = t.to_lowercase();
                vec![
                    if lower.contains("rust") { 1.0 } else { 0.0 },
                    if lower.contains("email") { 1.0 } else { 0.0 },
                ]
            })
            .collect())
    }
}

fn situational_doc(key: &str, content: &str) -> NamespaceDocumentInput {
    NamespaceDocumentInput {
        namespace: "user_pref_situational".to_string(),
        key: key.to_string(),
        title: key.to_string(),
        content: content.to_string(),
        source_type: "pref".to_string(),
        priority: "medium".to_string(),
        tags: vec![],
        metadata: json!({}),
        category: "core".to_string(),
        session_id: None,
        document_id: None,
        taint: crate::openhuman::memory::MemoryTaint::Internal,
    }
}

#[tokio::test]
async fn recall_relevant_by_vector_gates_on_similarity() {
    let tmp = TempDir::new().unwrap();
    let memory = UnifiedMemory::new(tmp.path(), Arc::new(KeywordEmbedder), None).unwrap();

    // Two situational prefs that embed onto orthogonal axes.
    memory
        .upsert_document(situational_doc(
            "rust_style",
            "When writing rust, prefer explicit error handling.",
        ))
        .await
        .unwrap();
    memory
        .upsert_document(situational_doc(
            "email_tone",
            "Be formal in email to my manager.",
        ))
        .await
        .unwrap();

    // A rust-related message recalls only the rust pref.
    let hits = memory
        .recall_relevant_by_vector("user_pref_situational", "help me with my rust code", 5, 0.5)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1, "only the relevant pref should pass the gate");
    assert_eq!(hits[0].0, "rust_style");
    assert!(hits[0].1.contains("explicit error handling"));

    // An unrelated message clears the gate to nothing — no block injected.
    let none = memory
        .recall_relevant_by_vector("user_pref_situational", "what is the weather today", 5, 0.5)
        .await
        .unwrap();
    assert!(
        none.is_empty(),
        "an unrelated message must surface no situational preferences"
    );
}
