use super::*;

#[test]
fn documents_schema_exposes_all_functions() {
    assert_eq!(controllers_core_recall().len(), FUNCTIONS_CORE_RECALL.len());
    assert_eq!(controllers_documents().len(), FUNCTIONS_DOCUMENTS.len());
    assert_eq!(controllers_ingest().len(), FUNCTIONS_INGEST.len());
    assert!(FUNCTIONS_CORE_RECALL.contains(&"init"));
    assert!(FUNCTIONS_CORE_RECALL.contains(&"clear_namespace"));
    assert!(FUNCTIONS_DOCUMENTS.contains(&"doc_put"));
    assert!(FUNCTIONS_INGEST.contains(&"doc_ingest"));
}

/// The three partitions must be disjoint and must cover the file — a
/// function that fell out of all three would silently lose its
/// registration, since `core::all` now pushes the parts, not the whole.
#[test]
fn capability_partitions_are_disjoint_and_total() {
    let mut all: Vec<&str> = Vec::new();
    all.extend(FUNCTIONS_CORE_RECALL);
    all.extend(FUNCTIONS_DOCUMENTS);
    all.extend(FUNCTIONS_INGEST);
    let mut sorted = all.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), all.len(), "a function appears in two parts");
    assert_eq!(all.len(), 16, "the documents file advertises 16 functions");
    for f in &all {
        assert!(schema(f).is_some(), "{f} has no schema");
    }
}

#[test]
fn unknown_document_schema_returns_none() {
    assert!(schema("not_real").is_none());
}

#[test]
fn query_namespace_schema_requires_namespace_and_query() {
    let schema = schema("query_namespace").unwrap();
    let required: Vec<&str> = schema
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"namespace"));
    assert!(required.contains(&"query"));
}

#[test]
fn clear_namespace_schema_requires_namespace() {
    let schema = schema("clear_namespace").unwrap();
    assert_eq!(schema.inputs.len(), 1);
    assert_eq!(schema.inputs[0].name, "namespace");
    assert!(schema.inputs[0].required);
}
