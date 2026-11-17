//! TinyFlows uses the workspace fallback when the host has not installed a
//! configured storage backend.

use openhuman_core::config::Config;
use openhuman_core::flows;
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flow_catalog_uses_the_workspace_fallback_without_bridge_reentry() {
    let workspace = tempfile::tempdir().unwrap();
    let config = Config {
        workspace_dir: workspace.path().to_path_buf(),
        ..Config::default()
    };

    assert!(openhuman_core::storage::installed().is_none());
    let graph = json!({
        "nodes": [
            { "id": "trigger", "kind": "trigger", "name": "Manual" },
            { "id": "agent", "kind": "agent", "name": "Summarize", "config": { "prompt": "hi" } }
        ],
        "edges": [{ "from_node": "trigger", "to_node": "agent" }]
    });

    let created = flows::ops::flows_create(&config, "Fallback".to_string(), graph, false)
        .await
        .expect("create a flow in the workspace fallback")
        .value;
    let listed = flows::ops::flows_list(&config)
        .await
        .expect("list flows in the workspace fallback")
        .value;

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, created.id);
    assert!(workspace.path().join("storage").exists());
}
