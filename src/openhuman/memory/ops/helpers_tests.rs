use super::*;
// The score breakdown a `NamespaceMemoryHit` carries is contract
// vocabulary — `tinymemory-core` only re-exports it — so the fixture names
// it where it is defined. Same item either way; the engine path was a
// compile-time link this host is shedding (#5560).
use crate::openhuman::memory::api::types::RetrievalScoreBreakdown;

fn sample_hit(kind: MemoryItemKind) -> NamespaceMemoryHit {
    NamespaceMemoryHit {
        id: "hit-1".into(),
        kind,
        namespace: "global".into(),
        key: "note-1".into(),
        title: Some("Title".into()),
        content: "Body text".into(),
        category: "core".into(),
        source_type: Some("manual".into()),
        updated_at: 1.5,
        score: 0.7,
        score_breakdown: RetrievalScoreBreakdown::default(),
        document_id: Some("doc-1".into()),
        chunk_id: Some("chunk-1".into()),
        supporting_relations: vec![GraphRelationRecord {
            namespace: Some("global".into()),
            subject: "Alice".into(),
            predicate: "OWNS".into(),
            object: "OpenHuman".into(),
            attrs: json!({"entity_types": {"subject": "PERSON", "object": "PRODUCT"}}),
            updated_at: 2.0,
            evidence_count: 1,
            order_index: Some(0),
            document_ids: vec!["doc-1".into()],
            chunk_ids: vec!["chunk-1".into()],
        }],
        taint: crate::openhuman::memory::MemoryTaint::Internal,
    }
}

#[test]
fn timestamp_to_rfc3339_rejects_invalid_values() {
    assert!(timestamp_to_rfc3339(f64::NAN).is_none());
    assert!(timestamp_to_rfc3339(f64::INFINITY).is_none());
    assert!(timestamp_to_rfc3339(-1.0).is_none());
    assert!(timestamp_to_rfc3339(1.5).is_some());
}

#[test]
fn relation_identity_and_metadata_include_namespace_and_attrs() {
    let relation = sample_hit(MemoryItemKind::Document)
        .supporting_relations
        .remove(0);
    assert_eq!(relation_identity(&relation), "global|Alice|OWNS|OpenHuman");
    let meta = relation_metadata(&relation);
    assert_eq!(meta["namespace"], "global");
    assert_eq!(meta["attrs"]["entity_types"]["subject"], "PERSON");
}

#[test]
fn build_retrieval_context_deduplicates_relations_and_entities() {
    let hit = sample_hit(MemoryItemKind::Document);
    let ctx = build_retrieval_context(&[hit.clone(), hit]);
    assert_eq!(ctx.chunks.len(), 2);
    assert_eq!(ctx.relations.len(), 1);
    assert!(ctx.entities.iter().any(|e| e.name == "Alice"));
    assert!(ctx.entities.iter().any(|e| e.name == "OpenHuman"));
}

#[test]
fn format_llm_context_message_includes_query_and_relation_text() {
    let hit = sample_hit(MemoryItemKind::Document);
    let text = format_llm_context_message(Some("who owns it"), &[hit]).unwrap();
    assert!(text.contains("Query: who owns it"));
    assert!(text.contains("Title: Body text"));
    assert!(text.contains("Alice (PERSON) -[OWNS]-> OpenHuman (PRODUCT)"));
}
