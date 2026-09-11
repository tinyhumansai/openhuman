//! Seam tests for `src/openhuman/flows/tinyflows/`.
//!
//! **Deviation from the original test plan** (see
//! `my_docs/ohxtf/b1-engine-seam-domain/09-testing-and-verification.md` item 2
//! and commons/11): the plan called for pointing `HttpRequestTool` at a local
//! mock HTTP server and asserting a success round-trip. That is not possible
//! against the REAL `HttpRequestTool` — unlike `tinyflows`' own mock
//! `HttpClient`, OpenHuman's `url_guard` unconditionally blocks
//! loopback/private hosts as an SSRF guard (`is_private_or_local_host`),
//! before the allowlist is even consulted, and any locally-hosted mock server
//! is necessarily loopback. So instead:
//! - the HTTP adapter tests assert the SSRF guard and the strict-allowlist
//!   rejection both surface as `EngineError::Capability` (proving the adapter
//!   correctly propagates `HttpRequestTool`'s real security behavior), and
//! - the engine smoke test drives `trigger -> http_request` against a
//!   deterministically-blocked loopback URL with `on_error: continue`, which
//!   exercises the full real stack (build_capabilities -> engine -> compiled
//!   graph -> `OpenHumanHttp` -> real `HttpRequestTool` -> SSRF guard ->
//!   `EngineError::Capability` -> the crate's `on_error: continue` policy ->
//!   error item) without any network dependency.

use std::sync::Arc;

use serde_json::json;
use tempfile::TempDir;
use tinyflows::caps::{CodeLanguage, CodeRunner, HttpClient, StateStore, ToolInvoker};
use tinyflows::model::{Edge, Node, NodeKind, WorkflowGraph};

use crate::openhuman::config::Config;
use crate::openhuman::security::SecurityPolicy;

use super::build_capabilities;
use super::caps::{FlowStateStore, OpenHumanCode, OpenHumanHttp, OpenHumanTools};

fn test_config(tmp: &TempDir) -> Arc<Config> {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    Arc::new(config)
}

fn node(id: &str, kind: NodeKind, config: serde_json::Value) -> Node {
    Node {
        id: id.to_string(),
        kind,
        type_version: 1,
        name: id.to_string(),
        config,
        ports: Vec::new(),
        position: None,
    }
}

fn edge(from: &str, to: &str) -> Edge {
    Edge {
        from_node: from.to_string(),
        from_port: "main".to_string(),
        to_node: to.to_string(),
        to_port: "main".to_string(),
    }
}

// ── HTTP adapter ─────────────────────────────────────────────────────────

fn http_adapter(allowed_domains: Vec<String>) -> OpenHumanHttp {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    OpenHumanHttp {
        security,
        http_config: crate::openhuman::config::HttpRequestConfig {
            allowed_domains,
            ..Default::default()
        },
        http_creds: Arc::new(
            crate::openhuman::security::credentials::HttpCredentialsStore::from_config(&config),
        ),
    }
}

// ── Tool curation / scope + connection_ref (issue B2) ─────────────────────
//
// No `ApprovalGate` is installed in this test binary (see the module doc on
// `flows::bus`'s tests and the trust-model tests in `approval::gate` for the
// gate-level behavior) — these tests exercise the *curation* gate, which is
// independent of the approval gate and runs first, so they stay deterministic
// without any global state.

fn tools_adapter(config: Arc<Config>) -> OpenHumanTools {
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    OpenHumanTools { config, security }
}

/// Minimal seeded [`ToolContract`](super::caps::ToolContract) for the tests
/// below — only `required_args` matters for the preflight, so every other
/// field is left at its "unknown" default.
fn seeded_required_args_contract(
    slug: &str,
    toolkit: &str,
    required: &[&str],
) -> super::caps::ToolContract {
    super::caps::ToolContract {
        slug: slug.to_string(),
        toolkit: toolkit.to_string(),
        description: None,
        required_args: required.iter().map(|s| s.to_string()).collect(),
        input_schema: None,
        output_fields: Vec::new(),
        output_schema: None,
        primary_array_path: None,
        is_curated: false,
    }
}

// ── OpenHumanAgentRunner: routing + request/model mapping (Phase A) ───────────

use super::caps::{
    build_agent_result, clamp_run_timeout_secs, harness_model_default_override,
    node_request_to_prompt, resolve_node_model, route_custom_entry_lookup, route_for_agent_ref,
    structured_output_instruction, AgentRoute,
};

// ── B38 (Gap 2): a custom agent_ref must route to the harness (real tools),
// not the persona-only completion fallback ─────────────────────────────────

fn custom_registry_entry(enabled: bool) -> crate::openhuman::agent::registry::AgentRegistryEntry {
    use crate::openhuman::agent::registry::types::{AgentRegistrySource, AgentSubagentPolicy};
    crate::openhuman::agent::registry::AgentRegistryEntry {
        id: "finance_analyst".to_string(),
        name: "Finance Analyst".to_string(),
        description: "Reviews spend and drafts finance summaries.".to_string(),
        source: AgentRegistrySource::Custom,
        enabled,
        model: Some("hint:reasoning".to_string()),
        system_prompt: Some("You are a meticulous finance analyst.".to_string()),
        tool_allowlist: vec!["memory_search".to_string()],
        tool_denylist: Vec::new(),
        subagents: AgentSubagentPolicy::default(),
        tags: Vec::new(),
        metadata: json!(null),
    }
}

#[path = "tinyflows_tests_part_01_tests.rs"]
mod part_01_tests;
