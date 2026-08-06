use super::*;
use serde_json::json;

#[test]
fn schema_array_path_prefers_the_shallowest_array() {
    let schema = json!({"properties": {
        "nested": {"properties": {"items": {"type": "array"}}},
        "top": {"type": "array"}
    }});
    assert_eq!(
        compute_primary_array_path(Some(&schema)).as_deref(),
        Some("top")
    );
}

#[test]
fn value_array_path_skips_only_named_root_keys() {
    let value = json!({"metadata": [], "data": {"metadata": []}});
    assert_eq!(
        compute_primary_array_path_from_value(&value, &["metadata"]).as_deref(),
        Some("data.metadata")
    );
}

#[test]
fn response_fields_are_sorted_and_ignore_schema_keywords() {
    let schema = json!({"type": "object", "z": {}, "a": {}});
    assert_eq!(response_fields_from_schema(Some(&schema)), ["a", "z"]);
}

#[test]
fn required_arguments_treat_missing_and_null_as_absent() {
    let required = vec!["missing".to_string(), "null".to_string(), "set".to_string()];
    assert_eq!(
        missing_required_args(&required, &json!({"null": null, "set": 1})),
        ["missing", "null"]
    );
}

#[test]
fn unsupported_arguments_follow_schema_openness() {
    let strict = json!({"properties": {"known": {}}});
    assert_eq!(
        unsupported_arg_names(Some(&strict), &json!({"known": 1, "extra": 2})),
        Some(vec!["extra".to_string()])
    );
    let open = json!({"properties": {"known": {}}, "additionalProperties": true});
    assert_eq!(
        unsupported_arg_names(Some(&open), &json!({"extra": 2})),
        None
    );
}
