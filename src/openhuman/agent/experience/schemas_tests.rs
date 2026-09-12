use super::*;
use crate::core::TypeSchema;
use std::collections::BTreeSet;

#[test]
fn schemas_cover_capture_retrieve_list_and_dismiss() {
    let functions: BTreeSet<_> = all_controller_schemas()
        .into_iter()
        .map(|schema| schema.function)
        .collect();

    assert_eq!(
        functions,
        BTreeSet::from(["capture", "retrieve", "list", "dismiss"])
    );

    let registered: BTreeSet<_> = all_registered_controllers()
        .into_iter()
        .map(|controller| controller.schema.function)
        .collect();
    assert_eq!(registered, functions);
}

#[test]
fn retrieve_schema_has_query_and_tools_inputs() {
    let schema = schemas("retrieve");
    assert_eq!(schema.namespace, "agent_experience");

    let query = schema
        .inputs
        .iter()
        .find(|input| input.name == "query")
        .expect("query input");
    assert_eq!(query.ty, TypeSchema::String);
    assert!(query.required);

    let tools = schema
        .inputs
        .iter()
        .find(|input| input.name == "tools")
        .expect("tools input");
    assert_eq!(tools.ty, TypeSchema::Array(Box::new(TypeSchema::String)));
    assert!(!tools.required);
}
