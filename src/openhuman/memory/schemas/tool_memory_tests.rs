use super::*;

#[test]
fn schema_exposes_all_tool_memory_functions() {
    let functions: Vec<&str> = FUNCTIONS.to_vec();
    assert_eq!(
        functions,
        vec![
            "tool_rule_put",
            "tool_rule_get",
            "tool_rule_list",
            "tool_rule_delete",
            "tool_rules_for_prompt",
            "tool_rules_json",
        ]
    );
}

#[test]
fn controllers_match_function_count() {
    let registered = controllers();
    assert_eq!(registered.len(), FUNCTIONS.len());
    assert!(registered.iter().all(|c| c.schema.namespace == "memory"));
}

#[test]
fn schema_returns_none_for_unknown_function() {
    assert!(schema("not_real").is_none());
}

#[test]
fn tool_rule_put_schema_requires_tool_name_and_rule() {
    let schema = schema("tool_rule_put").unwrap();
    let input_names: Vec<&str> = schema.inputs.iter().map(|f| f.name).collect();
    assert!(input_names.contains(&"tool_name"));
    assert!(input_names.contains(&"rule"));
    assert!(schema
        .inputs
        .iter()
        .any(|f| f.name == "tool_name" && f.required));
    assert!(schema.inputs.iter().any(|f| f.name == "rule" && f.required));
}
