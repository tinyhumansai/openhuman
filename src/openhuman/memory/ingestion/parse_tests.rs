use super::*;
use crate::openhuman::memory_store::types::NamespaceDocumentInput;
use serde_json::json;

fn sample_input() -> NamespaceDocumentInput {
    NamespaceDocumentInput {
        namespace: "global".into(),
        key: "doc-1".into(),
        title: "OpenHuman roadmap".into(),
        content: "Alice owns roadmap".into(),
        source_type: "manual".into(),
        priority: "normal".into(),
        tags: vec!["existing".into()],
        metadata: json!({"seed": true}),
        category: "core".into(),
        session_id: Some("session-1".into()),
        document_id: Some("doc-id-1".into()),
    }
}

#[test]
fn split_sentences_breaks_on_punctuation_and_merges_tiny_fragments() {
    let parts = split_sentences("Hello world. Ok.\nNext line?");
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0], "Hello world Ok");
    assert_eq!(parts[1], "Next line?");
}

#[test]
fn build_units_respects_extraction_mode() {
    let chunks = vec!["One. Two.".to_string(), "Three".to_string()];
    let sentence_units = build_units(&chunks, ExtractionMode::Sentence);
    let chunk_units = build_units(&chunks, ExtractionMode::Chunk);

    assert_eq!(sentence_units.len(), 2);
    assert_eq!(sentence_units[0].chunk_index, 0);
    assert_eq!(sentence_units[0].text, "One Two");
    assert_eq!(sentence_units[1].chunk_index, 1);
    assert_eq!(sentence_units[1].text, "Three");

    assert_eq!(chunk_units.len(), 2);
    assert_eq!(chunk_units[0].text, "One. Two");
    assert_eq!(chunk_units[1].text, "Three");
}

#[test]
fn find_chunk_index_prefers_hint_then_wraps() {
    let chunks = vec![
        "alpha content".to_string(),
        "beta needle".to_string(),
        "gamma trailing".to_string(),
    ];
    assert_eq!(find_chunk_index(&chunks, "needle", 1), 1);
    assert_eq!(find_chunk_index(&chunks, "alpha", 2), 0);
    assert_eq!(find_chunk_index(&chunks, "missing", 2), 2);
}

#[test]
fn alias_map_builds_reverse_lookup() {
    let mut entities = HashMap::new();
    entities.insert(
        "ALICE".into(),
        RawEntity {
            name: "ALICE".into(),
            entity_type: "PERSON".into(),
            confidence: 0.8,
        },
    );
    entities.insert(
        "ALICE SMITH".into(),
        RawEntity {
            name: "ALICE SMITH".into(),
            entity_type: "PERSON".into(),
            confidence: 0.9,
        },
    );
    let aliases = build_alias_map(&entities);
    assert_eq!(
        aliases.get("ALICE").map(String::as_str),
        Some("ALICE SMITH")
    );
    assert_eq!(resolve_alias("ALICE", &aliases), "ALICE SMITH");

    let reverse = reverse_aliases(&aliases);
    assert_eq!(reverse.get("ALICE SMITH"), Some(&vec!["ALICE".to_string()]));
}

#[test]
fn enrich_document_metadata_merges_tags_and_ingestion_details() {
    let input = sample_input();
    let parsed = ParsedIngestion {
        tags: vec!["decision".into(), "existing".into()],
        metadata: json!({"kind": "profile", "extra": 1}),
        entities: vec![],
        relations: vec![],
        chunk_count: 3,
        preference_count: 1,
        decision_count: 2,
    };
    let config = MemoryIngestionConfig::default();
    let (enriched, tags) = enrich_document_metadata(&input, &parsed, &config);

    assert_eq!(tags, vec!["decision".to_string(), "existing".to_string()]);
    assert_eq!(enriched.tags, tags);
    assert_eq!(enriched.metadata["seed"], json!(true));
    assert_eq!(enriched.metadata["extra"], json!(1));
    assert_eq!(
        enriched.metadata["ingestion"]["model_name"],
        config.model_name
    );
    assert_eq!(enriched.metadata["ingestion"]["chunk_count"], json!(3));
}

#[test]
fn extract_people_from_header_collects_named_people() {
    let mut acc = ExtractionAccumulator::default();
    let people = extract_people_from_header(
        "Alice Smith <alice@example.com>, Bob Jones <bob@example.com>",
        &mut acc,
    );
    assert_eq!(
        people,
        vec!["ALICE SMITH".to_string(), "BOB JONES".to_string()]
    );
    assert!(acc.entities.contains_key("ALICE SMITH"));
    assert!(acc.entities.contains_key("BOB JONES"));
}

#[test]
fn detect_primary_subject_only_matches_openhuman() {
    assert_eq!(
        detect_primary_subject("OpenHuman desktop roadmap"),
        Some("OPENHUMAN".to_string())
    );
    assert_eq!(detect_primary_subject("General roadmap"), None);
}
