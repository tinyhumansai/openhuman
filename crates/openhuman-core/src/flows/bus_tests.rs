use super::*;
use serde_json::json;
use tinyflows::model::{Node, NodeKind};
use tinyflows_bus::catalog::Workflow as Flow;

/// Binds a fresh in-memory reference engine to `config`'s workspace and
/// returns it, so the digest tests write and read back through one store.
fn digest_test_engine(config: &Config) -> Arc<tinymemory_api::conformance::ReferenceEngine> {
    let engine = Arc::new(tinymemory_api::conformance::ReferenceEngine::new());
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());
    engine
}

/// The run digests stored for `flow_id`.
async fn stored_digests(config: &Config, flow_id: &str) -> Vec<tinymemory_api::Hit> {
    crate::memory::ops::items_list(
        config,
        crate::memory::types::ItemsListParams {
            filter: Some(digest_filter(flow_id)),
            limit: Some(100),
            cursor: None,
            path: Vec::new(),
            preview: false,
        },
    )
    .await
    .unwrap()
    .items
}

fn test_config(tmp: &tempfile::TempDir) -> Arc<Config> {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    Arc::new(config)
}

fn trigger_node(config: Value) -> Node {
    Node {
        id: "t".to_string(),
        kind: NodeKind::Trigger,
        type_version: 1,
        name: "Trigger".to_string(),
        config,
        ports: Vec::new(),
        position: None,
    }
}

fn flow_with_trigger_config(id: &str, enabled: bool, trigger_config: Value) -> Flow {
    Flow {
        id: id.to_string(),
        name: id.to_string(),
        enabled,
        graph: json!({
            "nodes": [serde_json::to_value(trigger_node(trigger_config)).unwrap()],
            "edges": []
        }),
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        last_run_at: None,
        last_status: None,
        require_approval: false,
    }
}

#[path = "bus_subscriber_tests.rs"]
mod subscriber_tests;
#[path = "bus_trigger_queue_tests.rs"]
mod trigger_queue_tests;

#[tokio::test]
async fn a_trigger_handled_as_an_agent_uses_the_agent_config() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    let registered = crate::config::Config {
        workspace_dir: std::path::PathBuf::from("/tmp/flows-trigger-registered"),
        ..crate::config::Config::default()
    };
    let subscriber = FlowTriggerSubscriber::new(std::sync::Arc::new(registered));
    assert_eq!(
        subscriber
            .config_for_scope(&crate::core::runtime::DispatchCtx::unbound())
            .workspace_dir,
        std::path::PathBuf::from("/tmp/flows-trigger-registered"),
        "outside an agent the registered config is used"
    );
    let agent_config = crate::config::Config {
        workspace_dir: std::path::PathBuf::from("/tmp/flows-trigger-agent"),
        ..crate::config::Config::default()
    };
    let agent = CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(agent_config, DomainSet::full(), Default::default())
            .session_agent("flows-trigger-agent"),
    );
    let seen = subscriber
        .config_for_scope(&crate::core::runtime::DispatchCtx::new(agent))
        .workspace_dir
        .clone();
    assert_eq!(seen, std::path::PathBuf::from("/tmp/flows-trigger-agent"));
}
