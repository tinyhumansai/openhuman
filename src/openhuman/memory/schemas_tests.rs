use super::*;
use serde_json::json;

const ALL_FUNCTIONS: &[&str] = &[
    "init",
    "list_documents",
    "list_namespaces",
    "delete_document",
    "query_namespace",
    "recall_context",
    "recall_memories",
    "list_files",
    "read_file",
    "write_file",
    "namespace_list",
    "doc_put",
    "doc_ingest",
    "doc_list",
    "doc_delete",
    "context_query",
    "context_recall",
    "kv_set",
    "kv_get",
    "kv_delete",
    "kv_list_namespace",
    "graph_upsert",
    "graph_query",
    "clear_namespace",
];

#[test]
fn all_controller_schemas_has_entry_per_supported_function() {
    let names: Vec<_> = all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert_eq!(names.len(), ALL_FUNCTIONS.len());
    for expected in ALL_FUNCTIONS {
        assert!(names.contains(expected), "missing schema for {expected}");
    }
}

#[test]
fn all_registered_controllers_has_handler_per_schema() {
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), ALL_FUNCTIONS.len());
    let names: Vec<_> = controllers.iter().map(|c| c.schema.function).collect();
    for expected in ALL_FUNCTIONS {
        assert!(names.contains(expected), "missing handler for {expected}");
    }
}

#[test]
fn every_schema_uses_memory_namespace() {
    for s in all_controller_schemas() {
        assert_eq!(
            s.namespace, "memory",
            "schema {} must use the memory namespace",
            s.function
        );
    }
}

#[test]
fn every_schema_has_a_non_empty_description() {
    for s in all_controller_schemas() {
        assert!(
            !s.description.is_empty(),
            "schema {} has empty description",
            s.function
        );
    }
}

#[test]
fn schemas_unknown_function_returns_unknown_placeholder() {
    let s = schemas("not-a-real-function");
    assert_eq!(s.namespace, "memory");
    assert_eq!(s.function, "unknown");
}

// ── parse_params helper ──────────────────────────────────────

#[test]
fn parse_params_deserializes_simple_struct() {
    #[derive(serde::Deserialize, Debug)]
    struct Simple {
        name: String,
        count: u32,
    }
    let mut m = Map::new();
    m.insert("name".into(), json!("hi"));
    m.insert("count".into(), json!(7));
    let out: Simple = parse_params(m).unwrap();
    assert_eq!(out.name, "hi");
    assert_eq!(out.count, 7);
}

#[test]
fn parse_params_surfaces_deserialization_errors_with_context() {
    #[derive(serde::Deserialize, Debug)]
    struct Strict {
        #[allow(dead_code)]
        count: u32,
    }
    let mut m = Map::new();
    m.insert("count".into(), json!("not-a-number"));
    let err = parse_params::<Strict>(m).unwrap_err();
    assert!(err.contains("invalid params"));
}
