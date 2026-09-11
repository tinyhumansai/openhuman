use super::*;
use serde_json::json;

#[test]
fn all_schemas_returns_five() {
    assert_eq!(all_controller_schemas().len(), 5);
}

#[test]
fn all_controllers_returns_five() {
    assert_eq!(all_registered_controllers().len(), 5);
}

#[test]
fn all_use_tree_summarizer_namespace() {
    for s in all_controller_schemas() {
        assert_eq!(s.namespace, "tree_summarizer");
        assert!(!s.description.is_empty());
    }
}

#[test]
fn schemas_and_controllers_match() {
    let s = all_controller_schemas();
    let c = all_registered_controllers();
    for (schema, ctrl) in s.iter().zip(c.iter()) {
        assert_eq!(schema.function, ctrl.schema.function);
    }
}

#[test]
fn known_functions_resolve() {
    for fn_name in ["ingest", "run", "query", "status", "rebuild"] {
        let s = schemas(fn_name);
        assert_ne!(s.function, "unknown", "{fn_name} fell through");
    }
}

#[test]
fn unknown_function_returns_unknown() {
    let s = schemas("nonexistent");
    assert_eq!(s.function, "unknown");
}

#[test]
fn ingest_requires_namespace_and_content() {
    let s = schemas("ingest");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"namespace"));
    assert!(required.contains(&"content"));
}

#[test]
fn query_requires_namespace() {
    let s = schemas("query");
    let required: Vec<&str> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"namespace"));
}

#[test]
fn status_requires_namespace() {
    let s = schemas("status");
    assert!(s.inputs.iter().any(|f| f.name == "namespace" && f.required));
}

// ── Param helper tests ──────────────────────────────────────────

#[test]
fn read_required_parses_string() {
    let mut m = Map::new();
    m.insert("key".into(), Value::String("val".into()));
    let result: String = read_required(&m, "key").unwrap();
    assert_eq!(result, "val");
}

#[test]
fn read_required_errors_on_missing() {
    let m = Map::new();
    let err = read_required::<String>(&m, "key").unwrap_err();
    assert!(err.contains("missing required"));
}

#[test]
fn read_optional_returns_none_for_missing() {
    let m = Map::new();
    let result: Option<String> = read_optional(&m, "key").unwrap();
    assert!(result.is_none());
}

#[test]
fn read_optional_returns_none_for_null() {
    let mut m = Map::new();
    m.insert("key".into(), Value::Null);
    let result: Option<String> = read_optional(&m, "key").unwrap();
    assert!(result.is_none());
}

#[test]
fn read_optional_returns_some_for_value() {
    let mut m = Map::new();
    m.insert("key".into(), Value::String("val".into()));
    let result: Option<String> = read_optional(&m, "key").unwrap();
    assert_eq!(result, Some("val".into()));
}

#[test]
fn read_optional_timestamp_valid_rfc3339() {
    let mut m = Map::new();
    m.insert("ts".into(), Value::String("2026-04-17T12:00:00Z".into()));
    let result = read_optional_timestamp(&m, "ts").unwrap();
    assert!(result.is_some());
}

#[test]
fn read_optional_timestamp_invalid_format() {
    let mut m = Map::new();
    m.insert("ts".into(), Value::String("not-a-date".into()));
    assert!(read_optional_timestamp(&m, "ts").is_err());
}

#[test]
fn read_optional_timestamp_non_string() {
    let mut m = Map::new();
    m.insert("ts".into(), json!(12345));
    assert!(read_optional_timestamp(&m, "ts").is_err());
}

#[test]
fn read_optional_timestamp_none_for_missing() {
    let m = Map::new();
    assert!(read_optional_timestamp(&m, "ts").unwrap().is_none());
}

// ── type_name ───────────────────────────────────────────────────

#[test]
fn type_name_covers_all_variants() {
    assert_eq!(type_name(&Value::Null), "null");
    assert_eq!(type_name(&Value::Bool(true)), "bool");
    assert_eq!(type_name(&json!(42)), "number");
    assert_eq!(type_name(&json!("s")), "string");
    assert_eq!(type_name(&json!([1])), "array");
    assert_eq!(type_name(&json!({})), "object");
}

// ── namespace_input helper ───────────────────────────────────────

#[test]
fn namespace_input_is_required_string() {
    let f = namespace_input("test");
    assert_eq!(f.name, "namespace");
    assert!(f.required);
    assert!(matches!(f.ty, TypeSchema::String));
}
