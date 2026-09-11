use super::*;
use crate::openhuman::flows::Flow;
use serde_json::json;
use tinyflows::model::{Node, NodeKind, WorkflowGraph};

/// A directly-constructed, isolated [`Memory`] for the digest tests — NOT
/// the process-global `OnceLock` client. The global is one-shot, so an
/// earlier test in the same binary may already have bound it to a different
/// workspace, making `global::init(..)` here a silent no-op (see
/// `memory::global`'s own test notes). Injecting this instance into the
/// subscriber via [`FlowRunDigestSubscriber::with_memory`] makes writes and
/// read-backs go through the SAME store deterministically — the same shape
/// `flows::memory_tools`' tests use.
/// A guard over an in-memory store.
///
/// This used to build a real `UnifiedMemory` over `tmp` so writes and
/// read-backs went through one store. The digest writes through the guarded
/// driver now, so the fake sits behind a real `MemoryGuard` — same
/// determinism, same round trip, and the policy layer is on the path where
/// production has it.
fn digest_test_memory(
    _tmp: &tempfile::TempDir,
) -> Arc<crate::openhuman::memory::guard::MemoryGuard> {
    crate::openhuman::memory::guard::in_memory::guarded_in_memory().1
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
    // (`src/openhuman/flows/tinyflows/caps.rs`) — this test asserts the
    // subscriber collides with the SAME keys the engine's `dedup` node
    // itself reads/writes, not just "some" namespace.
    format!("flow:{flow_id}")
}

#[path = "bus_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "bus_tests_part_02_tests.rs"]
mod part_02_tests;
