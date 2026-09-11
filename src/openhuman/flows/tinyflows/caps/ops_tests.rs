use super::*;
use crate::openhuman::agent::prompts::types::IntegrationConnection;
use crate::openhuman::integrations::composio::{ComposioExecuteResponse, ConnectedIntegration};
use crate::openhuman::skills::types::{ToolContent, ToolResult};

fn integration(
    toolkit: &str,
    connected: bool,
    connections: Vec<IntegrationConnection>,
) -> ConnectedIntegration {
    ConnectedIntegration {
        toolkit: toolkit.to_string(),
        description: String::new(),
        tools: Vec::new(),
        gated_tools: Vec::new(),
        connected,
        connections,
        non_active_status: None,
    }
}

fn connection(id: &str, label: Option<&str>, is_default: bool) -> IntegrationConnection {
    IntegrationConnection {
        connection_id: id.to_string(),
        label: label.map(str::to_string),
        is_default,
    }
}

fn http_cred_store() -> (tempfile::TempDir, HttpCredentialsStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    // encrypt=true exercises the ChaCha20-Poly1305 at-rest path.
    let store = HttpCredentialsStore::new(dir.path(), true);
    (dir, store)
}

// ── Phase 2: autonomy-tier gating of acting nodes ──────────────────────

fn policy(level: crate::openhuman::security::AutonomyLevel) -> SecurityPolicy {
    SecurityPolicy {
        autonomy: level,
        ..SecurityPolicy::default()
    }
}

// ── Codex P1: Prompt-tier decisions must escalate past a workflow's own
// require_approval=false default, never silently auto-allow ────────────

use crate::openhuman::agent::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};

fn workflow_origin(job_id: &str, require_approval: bool) -> AgentTurnOrigin {
    AgentTurnOrigin::TrustedAutomation {
        job_id: job_id.to_string(),
        source: TrustedAutomationSource::Workflow { require_approval },
    }
}

// ── Phase 7: sub_workflow-by-id resolver ───────────────────────────────

fn resolver_test_config(tmp: &tempfile::TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

fn trigger_only_graph() -> WorkflowGraph {
    use tinyflows::model::{Node, NodeKind};
    WorkflowGraph {
        nodes: vec![Node {
            id: "t".to_string(),
            kind: NodeKind::Trigger,
            type_version: 1,
            name: "Trigger".to_string(),
            config: Value::Null,
            ports: Vec::new(),
            position: None,
        }],
        ..Default::default()
    }
}

#[path = "ops_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "ops_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "ops_tests_part_03_tests.rs"]
mod part_03_tests;
