use super::*;

#[test]
fn all_controller_schemas_lists_four_functions() {
    let names: Vec<_> = all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert_eq!(
        names,
        vec!["list", "resolve", "score", "refresh_address_book"]
    );
}

#[test]
fn resolve_schema_requires_kind_and_value() {
    let s = schemas("resolve");
    let required: Vec<_> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, vec!["kind", "value"]);
}

#[test]
fn unknown_returns_placeholder() {
    let s = schemas("nope");
    assert_eq!(s.function, "unknown");
}

#[test]
fn registered_controllers_have_handler_per_schema() {
    let regs = all_registered_controllers();
    assert_eq!(regs.len(), 4);
}

#[test]
fn list_schema_matches_ranked_people_response_shape() {
    let schema = schemas("list");
    let TypeSchema::Array(item_ty) = &schema.outputs[0].ty else {
        panic!("people output should be an array");
    };
    let TypeSchema::Object { fields } = item_ty.as_ref() else {
        panic!("people output item should be an object");
    };
    let names: Vec<_> = fields.iter().map(|f| f.name).collect();
    assert!(names.contains(&"handles"));
    assert!(names.contains(&"components"));
}

#[test]
fn score_schema_includes_component_breakdown() {
    let schema = schemas("score");
    let names: Vec<_> = schema.outputs.iter().map(|f| f.name).collect();
    assert!(names.contains(&"components"));
}
