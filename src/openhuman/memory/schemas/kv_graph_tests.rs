use super::*;

#[test]
fn kv_graph_schema_exposes_all_functions() {
    assert_eq!(
        FUNCTIONS,
        &[
            "kv_set",
            "kv_get",
            "kv_delete",
            "kv_list_namespace",
            "graph_upsert",
            "graph_query",
        ]
    );
    assert_eq!(controllers().len(), FUNCTIONS.len());
}

#[test]
fn unknown_kv_graph_schema_returns_none() {
    assert!(schema("not_real").is_none());
}

#[test]
fn graph_upsert_schema_requires_subject_predicate_and_object() {
    let schema = schema("graph_upsert").unwrap();
    let required: Vec<&str> = schema
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"subject"));
    assert!(required.contains(&"predicate"));
    assert!(required.contains(&"object"));
}
