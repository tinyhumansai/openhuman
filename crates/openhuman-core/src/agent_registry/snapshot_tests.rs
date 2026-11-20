use super::*;

#[test]
fn registry_snapshot_schema_keeps_its_wire_identity() {
    let schema = registry_snapshot_schema();
    assert_eq!(schema.namespace, "agent");
    assert_eq!(schema.function, "registry_snapshot");
    assert!(schema.inputs.is_empty());
}

#[tokio::test]
async fn registry_snapshot_omits_retired_graphs() {
    let result = handle_registry_snapshot(DispatchCtx::process_default(), Map::new())
        .await
        .expect("registry snapshot is available");
    let components = result
        .get("components")
        .and_then(Value::as_array)
        .expect("component snapshot");
    let ids: Vec<_> = components
        .iter()
        .filter_map(|component| component.get("id").and_then(Value::as_str))
        .collect();
    assert!(
        !ids.contains(&"delegation"),
        "snapshot includes the retired delegation graph: {ids:?}"
    );
    assert!(
        !ids.iter().any(|id| id.starts_with("agent_teams:")),
        "snapshot includes a retired team graph: {ids:?}"
    );
    assert!(
        !ids.iter().any(|id| id.starts_with("workflow_runs:")),
        "snapshot includes a retired workflow graph: {ids:?}"
    );
}
