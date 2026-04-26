use serde_json::json;

use super::{build_retrieval_context, filter_hits_by_document_ids, format_llm_context_message};
use crate::openhuman::memory::store::GraphRelationRecord;
use crate::openhuman::memory::{MemoryItemKind, NamespaceMemoryHit, RetrievalScoreBreakdown};

fn sample_hit() -> NamespaceMemoryHit {
    NamespaceMemoryHit {
        id: "doc-1".to_string(),
        kind: MemoryItemKind::Document,
        namespace: "team".to_string(),
        key: "atlas-status".to_string(),
        title: Some("Atlas status".to_string()),
        content: "Project Atlas is owned by Alice.".to_string(),
        category: "core".to_string(),
        source_type: Some("doc".to_string()),
        updated_at: 1_700_000_000.0,
        score: 0.92,
        score_breakdown: RetrievalScoreBreakdown {
            keyword_relevance: 0.3,
            vector_similarity: 0.4,
            graph_relevance: 0.9,
            episodic_relevance: 0.0,
            freshness: 0.0,
            final_score: 0.92,
        },
        document_id: Some("doc-1".to_string()),
        chunk_id: Some("doc-1#chunk-1".to_string()),
        supporting_relations: vec![GraphRelationRecord {
            namespace: Some("team".to_string()),
            subject: "Alice".to_string(),
            predicate: "OWNS".to_string(),
            object: "Atlas".to_string(),
            attrs: json!({"source": "graph"}),
            updated_at: 1_700_000_000.0,
            evidence_count: 2,
            order_index: Some(1),
            document_ids: vec!["doc-1".to_string()],
            chunk_ids: vec!["doc-1#chunk-1".to_string()],
        }],
    }
}

#[test]
fn build_retrieval_context_projects_hits_into_relations_and_chunks() {
    let context = build_retrieval_context(&[sample_hit()]);
    assert_eq!(context.entities.len(), 2);
    assert_eq!(context.relations.len(), 1);
    assert_eq!(context.chunks.len(), 1);
    assert_eq!(context.chunks[0].document_id.as_deref(), Some("doc-1"));
    assert_eq!(context.relations[0].predicate, "OWNS");
}

fn sample_hit_with_entity_types() -> NamespaceMemoryHit {
    NamespaceMemoryHit {
        id: "doc-2".to_string(),
        kind: MemoryItemKind::Document,
        namespace: "team".to_string(),
        key: "atlas-status".to_string(),
        title: Some("Atlas status".to_string()),
        content: "Project Atlas is owned by Alice.".to_string(),
        category: "core".to_string(),
        source_type: Some("doc".to_string()),
        updated_at: 1_700_000_000.0,
        score: 0.92,
        score_breakdown: RetrievalScoreBreakdown {
            keyword_relevance: 0.3,
            vector_similarity: 0.4,
            graph_relevance: 0.9,
            episodic_relevance: 0.0,
            freshness: 0.0,
            final_score: 0.92,
        },
        document_id: Some("doc-2".to_string()),
        chunk_id: Some("doc-2#chunk-1".to_string()),
        supporting_relations: vec![GraphRelationRecord {
            namespace: Some("team".to_string()),
            subject: "Alice".to_string(),
            predicate: "OWNS".to_string(),
            object: "Atlas".to_string(),
            attrs: json!({
                "source": "ingestion",
                "entity_types": {
                    "subject": "PERSON",
                    "object": "PROJECT"
                }
            }),
            updated_at: 1_700_000_000.0,
            evidence_count: 2,
            order_index: Some(1),
            document_ids: vec!["doc-2".to_string()],
            chunk_ids: vec!["doc-2#chunk-1".to_string()],
        }],
    }
}

#[test]
fn build_retrieval_context_extracts_entity_types_from_attrs() {
    let context = build_retrieval_context(&[sample_hit_with_entity_types()]);
    assert_eq!(context.entities.len(), 2);

    let alice = context.entities.iter().find(|e| e.name == "Alice").unwrap();
    assert_eq!(alice.entity_type.as_deref(), Some("PERSON"));

    let atlas = context.entities.iter().find(|e| e.name == "Atlas").unwrap();
    assert_eq!(atlas.entity_type.as_deref(), Some("PROJECT"));
}

#[test]
fn build_retrieval_context_entity_type_none_when_attrs_missing() {
    let context = build_retrieval_context(&[sample_hit()]);
    assert_eq!(context.entities.len(), 2);

    for entity in &context.entities {
        assert_eq!(
            entity.entity_type, None,
            "entity_type should be None when attrs has no entity_types"
        );
    }
}

#[test]
fn helpers_filter_document_ids_and_format_context_message() {
    let hit = sample_hit();
    let filtered = filter_hits_by_document_ids(vec![hit.clone()], Some(&["doc-2".to_string()]));
    assert!(filtered.is_empty());

    let message = format_llm_context_message(Some("who owns atlas"), &[hit])
        .expect("context message should exist");
    assert!(message.contains("Query: who owns atlas"));
    // Without entity_types in attrs, relations render without type annotations.
    assert!(message.contains("Alice -[OWNS]-> Atlas"));
}

#[test]
fn format_llm_context_message_includes_entity_types_when_present() {
    let hit = sample_hit_with_entity_types();
    let message = format_llm_context_message(Some("who owns atlas"), &[hit])
        .expect("context message should exist");
    assert!(
        message.contains("Alice (PERSON) -[OWNS]-> Atlas (PROJECT)"),
        "expected entity types in relation text, got: {message}"
    );
}

// ── Pure-helper coverage ───────────────────────────────────────

use super::{
    chunk_metadata, default_category, default_priority, default_source_type, error_envelope,
    extract_entity_type, maybe_retrieval_context, memory_counts, memory_kind_label,
    memory_request_id, relation_identity, relation_metadata, timestamp_to_rfc3339,
    validate_memory_relative_path,
};
use crate::openhuman::memory::{ApiEnvelope, MemoryRetrievalContext};
use crate::rpc::RpcOutcome;

#[test]
fn memory_request_id_is_nonempty_and_unique() {
    let a = memory_request_id();
    let b = memory_request_id();
    assert!(!a.is_empty());
    assert!(!b.is_empty());
    assert_ne!(a, b);
}

#[test]
fn memory_counts_builds_btreemap_from_entries() {
    let m = memory_counts([("documents", 3), ("kv", 1)]);
    assert_eq!(m.get("documents"), Some(&3));
    assert_eq!(m.get("kv"), Some(&1));
    assert_eq!(m.len(), 2);
}

#[test]
fn memory_counts_is_empty_for_empty_input() {
    let m: std::collections::BTreeMap<String, usize> = memory_counts(std::iter::empty());
    assert!(m.is_empty());
}

#[test]
fn timestamp_to_rfc3339_valid_seconds_and_fractional() {
    let s = timestamp_to_rfc3339(1_700_000_000.0).unwrap();
    assert!(s.contains("2023"));
    // Fractional seconds should preserve nanoseconds within range.
    let s = timestamp_to_rfc3339(1_700_000_000.5).unwrap();
    assert!(s.contains("2023"));
}

#[test]
fn timestamp_to_rfc3339_rejects_non_finite_and_negative() {
    assert!(timestamp_to_rfc3339(f64::NAN).is_none());
    assert!(timestamp_to_rfc3339(f64::INFINITY).is_none());
    assert!(timestamp_to_rfc3339(-1.0).is_none());
}

#[test]
fn memory_kind_label_maps_each_variant() {
    assert_eq!(memory_kind_label(&MemoryItemKind::Document), "document");
    assert_eq!(memory_kind_label(&MemoryItemKind::Kv), "kv");
    assert_eq!(memory_kind_label(&MemoryItemKind::Episodic), "episodic");
    assert_eq!(memory_kind_label(&MemoryItemKind::Event), "event");
}

fn relation_fixture(namespace: Option<&str>) -> GraphRelationRecord {
    GraphRelationRecord {
        namespace: namespace.map(str::to_string),
        subject: "Alice".into(),
        predicate: "OWNS".into(),
        object: "Atlas".into(),
        attrs: json!({"entity_types":{"subject":"PERSON","object":"PROJECT"}}),
        updated_at: 1_700_000_000.0,
        evidence_count: 2,
        order_index: Some(1),
        document_ids: vec!["doc-1".into()],
        chunk_ids: vec!["doc-1#c1".into()],
    }
}

#[test]
fn relation_identity_uses_global_for_missing_namespace() {
    let rel = relation_fixture(None);
    assert_eq!(relation_identity(&rel), "global|Alice|OWNS|Atlas");
    let rel = relation_fixture(Some("team"));
    assert_eq!(relation_identity(&rel), "team|Alice|OWNS|Atlas");
}

#[test]
fn relation_metadata_includes_expected_keys() {
    let rel = relation_fixture(Some("team"));
    let m = relation_metadata(&rel);
    assert_eq!(m["namespace"], "team");
    assert_eq!(m["order_index"], 1);
    assert!(m["document_ids"].is_array());
    assert!(m["updated_at"].is_string());
}

#[test]
fn chunk_metadata_exposes_score_breakdown() {
    let m = chunk_metadata(&sample_hit());
    assert_eq!(m["kind"], "document");
    assert_eq!(m["namespace"], "team");
    assert!(m["score_breakdown"]["final_score"].is_number());
}

#[test]
fn extract_entity_type_returns_nonempty_or_none() {
    let attrs = json!({"entity_types":{"subject":"PERSON","object":""}});
    assert_eq!(
        extract_entity_type(&attrs, "subject"),
        Some("PERSON".into())
    );
    // Empty string → None.
    assert_eq!(extract_entity_type(&attrs, "object"), None);
    // Missing role → None.
    assert_eq!(extract_entity_type(&attrs, "missing"), None);
    // Empty attrs → None.
    assert_eq!(extract_entity_type(&json!({}), "subject"), None);
}

#[test]
fn format_llm_context_message_returns_none_for_empty_hits() {
    assert!(format_llm_context_message(None, &[]).is_none());
    assert!(format_llm_context_message(Some("query"), &[]).is_none());
}

#[test]
fn filter_hits_by_document_ids_passes_through_when_filter_is_none() {
    let hits = vec![sample_hit()];
    let filtered = filter_hits_by_document_ids(hits.clone(), None);
    assert_eq!(filtered.len(), 1);
}

#[test]
fn filter_hits_by_document_ids_retains_matching_ids() {
    let hits = vec![sample_hit()];
    let filtered = filter_hits_by_document_ids(hits, Some(&["doc-1".to_string()]));
    assert_eq!(filtered.len(), 1);
}

#[test]
fn maybe_retrieval_context_respects_include_flag() {
    let empty = MemoryRetrievalContext {
        entities: vec![],
        relations: vec![],
        chunks: vec![],
    };
    // include=false → always None
    assert!(maybe_retrieval_context(false, empty.clone()).is_none());
    // include=true but context empty → None
    assert!(maybe_retrieval_context(true, empty).is_none());
    // include=true + non-empty context → Some
    let ctx = build_retrieval_context(&[sample_hit()]);
    assert!(maybe_retrieval_context(true, ctx).is_some());
}

#[test]
fn default_constants_are_stable() {
    assert!(!default_source_type().is_empty());
    assert!(!default_priority().is_empty());
    assert!(!default_category().is_empty());
}

#[test]
fn validate_memory_relative_path_rejects_empty_absolute_and_traversal() {
    assert!(validate_memory_relative_path("").is_err());
    assert!(validate_memory_relative_path("/etc/passwd").is_err());
    assert!(validate_memory_relative_path("../secrets").is_err());
    assert!(validate_memory_relative_path("ok/subdir/file.md").is_ok());
    assert!(validate_memory_relative_path("simple.txt").is_ok());
}

#[test]
fn error_envelope_produces_api_error_with_code_and_message() {
    let envelope: RpcOutcome<ApiEnvelope<serde_json::Value>> =
        error_envelope::<serde_json::Value>("NOT_FOUND", "missing".into());
    let api = &envelope.value;
    assert!(api.data.is_none());
    let err = api.error.as_ref().expect("error set");
    assert_eq!(err.code, "NOT_FOUND");
    assert_eq!(err.message, "missing");
    // Meta must carry a request id.
    assert!(!api.meta.request_id.is_empty());
}
