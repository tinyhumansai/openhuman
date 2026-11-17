use super::*;
use crate::core::runtime::DispatchCtx;

#[test]
fn controller_schema_inventory_is_stable() {
    let schemas = all_controller_schemas();
    let functions: Vec<_> = schemas.iter().map(|schema| schema.function).collect();
    assert_eq!(functions, vec!["list"]);
    assert_eq!(schemas.len(), all_registered_controllers().len());
    for schema in &schemas {
        assert_eq!(schema.namespace, "commands");
    }
}

#[test]
fn list_schema_has_no_required_inputs() {
    let schema = schemas("list");
    assert!(schema.inputs.is_empty());
    assert_eq!(schema.outputs.len(), 1);
    assert_eq!(schema.outputs[0].name, "commands");
}

#[tokio::test]
async fn handle_list_returns_every_builtin() {
    use serde_json::Map;

    let value = handle_list(DispatchCtx::process_default(), Map::new())
        .await
        .expect("handler must not fail");
    // Bare (no logs) or wrapped ({"result": ...}) — read through both.
    let commands = value
        .get("result")
        .unwrap_or(&value)
        .get("commands")
        .and_then(|v| v.as_array())
        .expect("commands array");
    let ids: Vec<&str> = commands
        .iter()
        .filter_map(|c| c.get("id").and_then(|v| v.as_str()))
        .collect();
    for expected in ["new", "clear", "plan", "build", "goal", "todo", "stop"] {
        assert!(
            ids.contains(&expected),
            "missing builtin {expected}: {ids:?}"
        );
    }
}
