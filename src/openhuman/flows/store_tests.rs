use super::*;
use crate::openhuman::config::Config;
use tempfile::TempDir;
use tinyflows::model::{Node, NodeKind, WorkflowGraph};

fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

fn trigger_graph() -> WorkflowGraph {
    WorkflowGraph {
        nodes: vec![Node {
            id: "t".to_string(),
            kind: NodeKind::Trigger,
            type_version: 1,
            name: "Trigger".to_string(),
            config: serde_json::Value::Null,
            ports: Vec::new(),
            position: None,
        }],
        ..Default::default()
    }
}

#[test]
fn create_get_list_delete_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let flow = create_flow(&config, "demo".to_string(), trigger_graph()).unwrap();
    assert_eq!(flow.name, "demo");
    assert!(flow.enabled);

    let fetched = get_flow(&config, &flow.id).unwrap().expect("flow present");
    assert_eq!(fetched.id, flow.id);
    assert_eq!(fetched.graph, flow.graph);

    let listed = list_flows(&config).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, flow.id);

    remove_flow(&config, &flow.id).unwrap();
    assert!(get_flow(&config, &flow.id).unwrap().is_none());
    assert!(list_flows(&config).unwrap().is_empty());
}

#[test]
fn get_flow_returns_none_for_unknown_id() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert!(get_flow(&config, "missing").unwrap().is_none());
}

#[test]
fn remove_flow_errors_when_not_found() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = remove_flow(&config, "missing").unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[test]
fn set_enabled_toggles_and_persists() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph()).unwrap();
    assert!(flow.enabled);

    let disabled = set_enabled(&config, &flow.id, false).unwrap();
    assert!(!disabled.enabled);

    let reloaded = get_flow(&config, &flow.id).unwrap().unwrap();
    assert!(!reloaded.enabled);

    let enabled = set_enabled(&config, &flow.id, true).unwrap();
    assert!(enabled.enabled);
}

#[test]
fn update_flow_graph_bumps_updated_at_and_preserves_created_at() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph()).unwrap();

    let mut new_graph = trigger_graph();
    new_graph.name = "renamed-graph".to_string();
    let updated = update_flow_graph(&config, &flow.id, "renamed".to_string(), new_graph).unwrap();

    assert_eq!(updated.name, "renamed");
    assert_eq!(updated.created_at, flow.created_at);
    assert_eq!(updated.graph.name, "renamed-graph");
}

#[test]
fn record_run_sets_last_run_fields() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = create_flow(&config, "demo".to_string(), trigger_graph()).unwrap();
    assert!(flow.last_run_at.is_none());

    record_run(&config, &flow.id, "completed").unwrap();
    let reloaded = get_flow(&config, &flow.id).unwrap().unwrap();
    assert!(reloaded.last_run_at.is_some());
    assert_eq!(reloaded.last_status.as_deref(), Some("completed"));
}

#[test]
fn stored_graph_older_than_current_schema_is_migrated_on_read() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Insert a raw, versionless graph row directly (bypassing create_flow's
    // typed path) to simulate a definition persisted by an older crate build.
    let legacy_graph_json = serde_json::json!({
        "name": "legacy",
        "nodes": [{ "id": "t", "kind": "trigger", "name": "Trigger" }],
        "edges": []
    })
    .to_string();

    with_connection(&config, |conn| {
        conn.execute(
            "INSERT INTO flow_definitions
                (id, name, graph_json, enabled, created_at, updated_at, last_run_at, last_status)
             VALUES ('legacy-1', 'legacy', ?1, 1, '2020-01-01T00:00:00Z', '2020-01-01T00:00:00Z', NULL, NULL)",
            rusqlite::params![legacy_graph_json],
        )?;
        Ok(())
    })
    .unwrap();

    let loaded = get_flow(&config, "legacy-1").unwrap().expect("row present");
    assert_eq!(
        loaded.graph.schema_version,
        tinyflows::model::CURRENT_SCHEMA_VERSION
    );
    assert_eq!(loaded.graph.nodes.len(), 1);
}

#[test]
fn kv_get_set_round_trips_and_is_namespace_scoped() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    assert!(kv_get(&config, "ns1", "k").unwrap().is_none());

    kv_set(&config, "ns1", "k", &serde_json::json!({"v": 1})).unwrap();
    assert_eq!(
        kv_get(&config, "ns1", "k").unwrap(),
        Some(serde_json::json!({"v": 1}))
    );

    // A different namespace does not see ns1's value.
    assert!(kv_get(&config, "ns2", "k").unwrap().is_none());

    // Overwrite.
    kv_set(&config, "ns1", "k", &serde_json::json!(2)).unwrap();
    assert_eq!(
        kv_get(&config, "ns1", "k").unwrap(),
        Some(serde_json::json!(2))
    );
}
