//! Seam tests for `src/openhuman/tinyflows/`.
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
use tinyflows::caps::{CodeLanguage, CodeRunner, HttpClient, StateStore};
use tinyflows::model::{Edge, Node, NodeKind, WorkflowGraph};

use crate::openhuman::config::Config;
use crate::openhuman::security::SecurityPolicy;

use super::build_capabilities;
use super::caps::{FlowStateStore, OpenHumanCode, OpenHumanHttp};

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

// ── build_capabilities smoke ────────────────────────────────────────────

#[test]
fn build_capabilities_constructs_every_slot_without_panicking() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // Purely a construction smoke test — no capability is invoked here.
    let _caps = build_capabilities(config, "test:build");
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
    }
}

#[tokio::test]
async fn http_adapter_blocks_loopback_host_as_capability_error() {
    let adapter = http_adapter(vec![]); // open allowlist mode
    let err = adapter
        .request(
            json!({ "method": "GET", "url": "http://127.0.0.1:1/" }),
            None,
        )
        .await
        .expect_err("loopback host must be blocked by the SSRF guard");
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("private") || msg.to_lowercase().contains("local"),
        "expected an SSRF-guard message, got: {msg}"
    );
}

#[tokio::test]
async fn http_adapter_rejects_host_outside_strict_allowlist() {
    let adapter = http_adapter(vec!["example.com".to_string()]);
    let err = adapter
        .request(
            json!({ "method": "GET", "url": "https://not-allowed.test/" }),
            None,
        )
        .await
        .expect_err("host outside the strict allowlist must be rejected");
    assert!(
        err.to_string().contains("not-allowed.test")
            || err.to_string().to_lowercase().contains("allowed"),
        "expected an allowlist rejection message, got: {err}"
    );
}

// ── StateStore adapter ───────────────────────────────────────────────────

#[tokio::test]
async fn flow_state_store_round_trips_and_is_namespace_scoped() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let ns1 = FlowStateStore {
        config: config.clone(),
        namespace: "ns1".to_string(),
    };
    let ns2 = FlowStateStore {
        config: config.clone(),
        namespace: "ns2".to_string(),
    };

    assert!(ns1.load("k").await.unwrap().is_none());

    ns1.store("k", json!({ "v": 1 })).await.unwrap();
    assert_eq!(ns1.load("k").await.unwrap(), Some(json!({ "v": 1 })));

    // A different namespace never sees ns1's value.
    assert!(ns2.load("k").await.unwrap().is_none());

    // Overwrite.
    ns1.store("k", json!(2)).await.unwrap();
    assert_eq!(ns1.load("k").await.unwrap(), Some(json!(2)));
}

// ── Engine smoke: real seam end to end ───────────────────────────────────

#[tokio::test]
async fn engine_run_drives_trigger_to_http_request_through_the_real_seam() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let caps = build_capabilities(config, "test:smoke");

    // A deterministically-blocked loopback URL with `on_error: continue` so
    // the run completes even though the (real, SSRF-guarded) HTTP adapter
    // necessarily rejects it — see the module doc for why a real network
    // round-trip isn't testable here.
    let graph = WorkflowGraph {
        nodes: vec![
            node("t", NodeKind::Trigger, serde_json::Value::Null),
            node(
                "http",
                NodeKind::HttpRequest,
                json!({ "method": "GET", "url": "http://127.0.0.1:1/", "on_error": "continue" }),
            ),
        ],
        edges: vec![edge("t", "http")],
        ..Default::default()
    };
    let compiled = tinyflows::compiler::compile(&graph).expect("compile");

    let outcome = tinyflows::engine::run(&compiled, json!({ "seed": 1 }), &caps)
        .await
        .expect("run should complete (on_error: continue)");

    assert!(outcome.pending_approvals.is_empty());
    assert_eq!(
        outcome.output["nodes"]["http"]["items"][0]["json"]["error"]["node"],
        json!("http")
    );
}

// ── Code adapter ──────────────────────────────────────────────────────────

/// Requires `node` on `PATH`. Ignored by default (per the B1 test plan);
/// run explicitly with `cargo test -- --ignored` on a host with Node
/// installed.
#[tokio::test]
#[ignore = "requires a `node` binary on PATH"]
async fn code_adapter_javascript_passthrough_round_trips_json() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let runner = OpenHumanCode { config };

    let input = json!([{ "json": { "n": 7 } }]);
    let result = runner
        .run(CodeLanguage::JavaScript, "return input;", input.clone())
        .await
        .expect("javascript passthrough should succeed when node is present");
    assert_eq!(result, input);
}
