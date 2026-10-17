use super::*;
use serde_json::json;
use tinyflows::model::{Node, NodeKind, WorkflowGraph};
use tinyflows_catalog::Flow;

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
        graph: WorkflowGraph {
            nodes: vec![trigger_node(trigger_config)],
            ..Default::default()
        },
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        last_run_at: None,
        last_status: None,
        require_approval: false,
    }
}

fn dedup_node(id: &str) -> Node {
    Node {
        id: id.to_string(),
        kind: NodeKind::Dedup,
        type_version: 1,
        name: id.to_string(),
        config: json!({ "key": "=item.id" }),
        ports: Vec::new(),
        position: None,
    }
}

/// A saved flow with a `trigger` node plus one `dedup` node with id
/// `dedup_id` — the minimal graph [`DedupCommitSubscriber::dedup_node_ids`]
/// needs to find something to settle.
fn flow_with_dedup_node(id: &str, dedup_id: &str) -> Flow {
    Flow {
        id: id.to_string(),
        name: id.to_string(),
        enabled: true,
        graph: WorkflowGraph {
            nodes: vec![trigger_node(json!({})), dedup_node(dedup_id)],
            ..Default::default()
        },
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        last_run_at: None,
        last_status: None,
        require_approval: false,
    }
}

// ── DedupCommitSubscriber ────────────────────────────────────────

fn dedup_state_namespace(flow_id: &str) -> String {
    // MUST match `tinyflows::build_capabilities`'s `state_namespace`
    // (`crates/openhuman-core/src/flows/tinyflows/caps.rs`) — this test asserts the
    // subscriber collides with the SAME keys the engine's `dedup` node
    // itself reads/writes, not just "some" namespace.
    format!("flow:{flow_id}")
}

#[path = "bus_dedup_commit_lock_tests.rs"]
mod dedup_commit_lock_tests;
#[path = "bus_subscriber_tests.rs"]
mod subscriber_tests;

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

/// The dedup settle path reads the flow from the config its explicit context
/// carries, concurrently with another tenant's, across an await and in a
/// spawned task, with no task scope anywhere.
#[tokio::test]
async fn dedup_nodes_are_read_from_the_explicit_contexts_config() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    let registered_tmp = tempfile::tempdir().unwrap();
    let registered = test_config(&registered_tmp);
    let subscriber = Arc::new(DedupCommitSubscriber::new(Arc::clone(&registered)));

    let mut contexts = Vec::new();
    for (name, dedup_id) in [("tenant-a", "dedup-a"), ("tenant-b", "dedup-b")] {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        store::upsert_flow(&config, &flow_with_dedup_node("shared-flow", dedup_id)).unwrap();
        let ctx = DispatchCtx::new(
            CoreContext::for_test(DomainSet::full(), None).derive_with(
                ContextOverlay::new((*config).clone(), DomainSet::full(), Default::default())
                    .session_agent(name),
            ),
        );
        contexts.push((tmp, ctx, dedup_id));
    }

    let mut tasks = Vec::new();
    for (tmp, ctx, dedup_id) in contexts {
        let subscriber = Arc::clone(&subscriber);
        tasks.push(tokio::spawn(async move {
            tokio::task::yield_now().await;
            let ids = subscriber.dedup_node_ids(&ctx, "shared-flow");
            drop(tmp);
            (ids, dedup_id)
        }));
    }
    for task in tasks {
        let (ids, expected) = task.await.unwrap();
        assert_eq!(ids, vec![expected.to_string()]);
    }
    // The registered config has no such flow: it is never consulted.
    assert!(subscriber
        .dedup_node_ids(&DispatchCtx::unbound(), "shared-flow")
        .is_empty());
}
