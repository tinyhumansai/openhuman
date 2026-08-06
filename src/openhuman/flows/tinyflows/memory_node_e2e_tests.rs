//! End-to-end tests proving the `memory` node round-trips through the REAL
//! stack: the `tinyflows` engine executing a *compiled graph*, dispatching
//! through the real [`OpenHumanMemory`](super::memory_adapter::OpenHumanMemory)
//! host adapter — wired via [`build_capabilities`], NOT
//! `tinyflows::caps::mock::MockMemory` — against a real, on-disk `Memory`
//! store.
//!
//! **The gap this closes.** Two layers of unit coverage already exist:
//! - `memory_adapter_tests.rs` calls `OpenHumanMemory`'s methods directly
//!   (scope lockdown, tier gate, trust boundary) but never through a compiled
//!   graph or the engine, and never against the real store (every test there
//!   uses a fresh empty `Config::workspace_dir` and only exercises the
//!   error paths).
//! - `vendor/tinyflows/src/nodes/integration/memory.rs`'s own tests drive
//!   `MemoryNode` through `compile`/`run`, but always against
//!   `tinyflows::caps::mock::MockMemory` — the crate's own mock, not this
//!   host's real adapter.
//!
//! Neither proves the actual wiring a saved flow run uses in production:
//! `flows::ops::flows_run` compiles a graph, calls
//! [`build_capabilities`] for a real `Capabilities` bundle, and hands both to
//! `tinyflows::engine::run`. This file reproduces exactly that path (see
//! [`trigger_to_memory`] / [`workflow_origin`]) so a regression in the
//! engine → `Capabilities::memory` → `OpenHumanMemory` → `Memory` store chain
//! fails a test, not just a live flow run.
//!
//! **Layer entered: full engine-run**, not a node-executor shortcut. Every
//! test below drives `tinyflows::compiler::compile` + `tinyflows::engine::run`
//! over a real [`build_capabilities`] bundle, under the same
//! `AgentTurnOrigin::TrustedAutomation { source: Workflow { .. }, .. }`
//! task-local scope `flows::ops::workflow_origin` scopes around a real
//! `flows_run` — the ONLY thing not reproduced is `flows::ops`'s own
//! run-bookkeeping (draft persistence, checkpointing, run-history rows),
//! which has nothing to do with whether the `memory` node's adapter wiring
//! works and is covered separately by `flows::ops_tests`.
//!
//! **Real store, not a stub.** `memory` here is the process-global
//! `MemoryClient` (`crate::openhuman::memory::global`), bound to the shared
//! temp workspace `memory::ops::test_support::ensure_shared_memory_client`
//! already uses for every other `memory::ops` real-store test — the SAME
//! on-disk `UnifiedMemory`-backed store `flows_run` writes to in production.
//! Serialized against sibling tests with `GLOBAL_MEMORY_TEST_LOCK`, exactly
//! like `memory::ops::documents`/`memory::ops::sync`/etc already do (see
//! [`lock_shared_memory`]).

use std::sync::Arc;

use serde_json::{json, Value};
use tinyflows::model::{Edge, Node, NodeKind, WorkflowGraph};

use crate::openhuman::agent::turn_origin::{self, AgentTurnOrigin, TrustedAutomationSource};
use crate::openhuman::config::Config;
use crate::openhuman::flows::flow_namespace;
use crate::openhuman::flows::memory_tools::FlowMemoryRecallTool;
use crate::openhuman::security::AutonomyLevel;
use crate::openhuman::tools::traits::Tool;

use super::build_capabilities;

// ── fixtures ────────────────────────────────────────────────────────────

/// Binds the process-global memory client to the shared test workspace and
/// holds the cross-test serialization lock for the caller's whole test body.
///
/// One on-disk SQLite store is shared across every test thread in this
/// binary (`memory::global` is a process-global `OnceLock`), so concurrent
/// `init`/read/write from sibling tests races on schema init and can bleed
/// data across tests. `GLOBAL_MEMORY_TEST_LOCK` + `ensure_shared_memory_client`
/// is the crate's existing, proven pattern for this (see
/// `memory::ops::documents::tests::ensure_memory_client` and
/// `composio::ops_tests::init_memory_client`) — reused verbatim here rather
/// than inventing a third variant.
async fn lock_shared_memory() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    crate::openhuman::memory::ops::ensure_shared_memory_client();
    guard
}

/// A `Config` rooted at its own fresh tempdir, with autonomy raised to
/// `Full`.
///
/// `remember`/`forget` are `CommandClass::Write` in
/// `OpenHumanMemory::tier_gate_write`, which needs a non-`Block` tier
/// decision to clear `enforce_node_tier_gate` at all. `Full` also keeps the
/// write from depending on whether some earlier test in this shared binary
/// has installed a global `ApprovalGate` — see [`workflow_origin`]'s doc for
/// why `require_approval: false` makes this safe either way.
fn full_autonomy_config() -> (tempfile::TempDir, Arc<Config>) {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut cfg = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    cfg.autonomy.level = AutonomyLevel::Full;
    (tmp, Arc::new(cfg))
}

/// A trusted, saved-flow run origin for `flow_id` — the ONLY source
/// `OpenHumanMemory::trusted_flow_id` (and `flow_memory_recall`/
/// `flow_memory_remember`'s own `trusted_flow_id`) accepts for
/// `scope: "flow"`/`"flows"`. Mirrors what `flows::ops::flows_run` scopes
/// around a real run.
///
/// `require_approval: false` matches a saved flow's default. It also means
/// this test's write can never park on approval even if some other test in
/// this shared binary has installed a global `ApprovalGate`:
/// `ApprovalGate::intercept_audited` allows a
/// `Workflow { require_approval: false }` origin unconditionally (see
/// `caps::gate_call_for_tier`'s doc comment) — so this test's outcome does
/// not depend on test execution order.
fn workflow_origin(flow_id: &str) -> AgentTurnOrigin {
    AgentTurnOrigin::TrustedAutomation {
        job_id: flow_id.to_string(),
        source: TrustedAutomationSource::Workflow {
            require_approval: false,
        },
    }
}

fn node(id: &str, kind: NodeKind, config: Value) -> Node {
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

/// A minimal `trigger -> memory` graph — the same two-node shape
/// `vendor/tinyflows/src/nodes/integration/memory.rs`'s own tests use, kept
/// identical here so any behavioral difference between running against
/// `MockMemory` and running against the real `OpenHumanMemory` adapter is
/// attributable to the adapter, not to graph-shape differences.
fn trigger_to_memory(config: Value) -> WorkflowGraph {
    WorkflowGraph {
        nodes: vec![
            node("t", NodeKind::Trigger, Value::Null),
            node("mem", NodeKind::Memory, config),
        ],
        edges: vec![edge("t", "mem")],
        ..Default::default()
    }
}

fn unique_flow_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}

// ── 1 & 2. flow-scope round-trip through the real engine, plus coherence
// with the sibling `flow_memory_recall` agent tool ─────────────────────────

#[tokio::test]
async fn memory_node_remember_then_recall_round_trips_through_the_real_engine_and_adapter() {
    let _serial = lock_shared_memory().await;
    let (_tmp, config) = full_autonomy_config();
    let flow_id = unique_flow_id("e2e-roundtrip");
    let caps = build_capabilities(config, format!("flow:{flow_id}"));

    // ── remember: a real graph, compiled by the real validator/compiler and
    // run through the real engine, dispatching through the real
    // OpenHumanMemory adapter (build_capabilities' `memory` slot). ──
    let remember_graph = trigger_to_memory(json!({
        "operation": "remember",
        "scope": "flow",
        "key": "item-42",
        "value": "Digest already sent item-42 to subscribers"
    }));
    let compiled_remember =
        tinyflows::compiler::compile(&remember_graph).expect("compile remember graph");

    let remember_outcome = turn_origin::with_origin(
        workflow_origin(&flow_id),
        tinyflows::engine::run(&compiled_remember, Value::Null, &caps),
    )
    .await
    .expect("remember run should complete against the real store");

    assert!(
        remember_outcome.pending_approvals.is_empty(),
        "Full autonomy + require_approval:false must never park this write"
    );
    assert_eq!(
        remember_outcome.output["nodes"]["mem"]["items"][0]["json"]["json"]["ok"],
        json!(true),
        "unexpected remember output shape: {}",
        remember_outcome.output
    );
    assert_eq!(
        remember_outcome.output["nodes"]["mem"]["items"][0]["json"]["json"]["key"],
        json!("item-42")
    );

    // ── recall: a SECOND compiled graph, a SECOND engine run, same flow_id
    // — proving the write from the first run is durably visible via the real
    // on-disk store, not merely echoed back within one call. ──
    let recall_graph = trigger_to_memory(json!({
        "operation": "recall",
        "scope": "flow",
        "query": "item-42"
    }));
    let compiled_recall =
        tinyflows::compiler::compile(&recall_graph).expect("compile recall graph");

    let recall_outcome = turn_origin::with_origin(
        workflow_origin(&flow_id),
        tinyflows::engine::run(&compiled_recall, Value::Null, &caps),
    )
    .await
    .expect("recall run should complete against the real store");

    assert!(recall_outcome.pending_approvals.is_empty());
    let results = recall_outcome.output["nodes"]["mem"]["items"][0]["json"]["json"]["results"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        results
            .iter()
            .any(|hit| hit["text"].as_str().is_some_and(|t| t.contains("item-42"))),
        "expected the remembered content back from the real flow_<id> namespace, got: {results:?}"
    );

    // ── coherence (#5176): the SAME write is visible through the sibling
    // `flow_memory_recall` agent tool for the same flow_id — proving one
    // shared store, not two namespace conventions that happen to overlap by
    // convention (see memory_adapter.rs's module doc). ──
    let memory = crate::openhuman::memory::global::client_if_ready()
        .expect("global memory client must be initialized by lock_shared_memory")
        .memory_handle();
    let recall_tool = FlowMemoryRecallTool::new(memory);
    let tool_result = turn_origin::with_origin(
        workflow_origin(&flow_id),
        recall_tool.execute(json!({ "query": "item-42", "flow_id": flow_id })),
    )
    .await
    .expect("flow_memory_recall tool call should not error at the harness level");
    assert!(
        !tool_result.is_error,
        "flow_memory_recall reported an error: {}",
        tool_result.output()
    );
    assert!(
        tool_result.output().contains("item-42"),
        "expected the memory-node's write to be visible via flow_memory_recall \
         (same underlying store), got: {}",
        tool_result.output()
    );
}

// ── 3. security invariant end-to-end: scope:"user" writes are rejected,
// and the user's real memory store is never touched ────────────────────────

#[tokio::test]
async fn memory_node_remember_user_scope_is_rejected_and_never_touches_user_memory() {
    let _serial = lock_shared_memory().await;
    let (_tmp, config) = full_autonomy_config();
    let flow_id = unique_flow_id("e2e-security");
    let caps = build_capabilities(config, format!("flow:{flow_id}"));

    // A unique key so this assertion can't collide with real content any
    // other test in this shared workspace may have written under
    // GLOBAL_NAMESPACE.
    let forbidden_key = format!("forbidden-{}", uuid::Uuid::new_v4());

    // ── (a) validate-time rejection: tinyflows' own structural validator
    // rejects a `remember`/`scope: "user"` node BEFORE compile ever
    // succeeds, so a graph shaped this way can never even reach a run. ──
    let user_scope_graph = trigger_to_memory(json!({
        "operation": "remember",
        "scope": "user",
        "key": forbidden_key,
        "value": "must never be written to user memory"
    }));
    let compile_err = tinyflows::compiler::compile(&user_scope_graph)
        .expect_err("scope: \"user\" on a remember node must be rejected at validate/compile time");
    assert!(
        compile_err.to_string().to_lowercase().contains("user"),
        "expected the validator's scope:\"user\" rejection, got: {compile_err}"
    );

    // ── (b) defense-in-depth: even bypassing tinyflows' validator entirely
    // and calling straight through to the adapter build_capabilities wired
    // (the exact instance a real run would dispatch to), OpenHumanMemory's
    // own remember() hard-refuses anything but scope: "flow". ──
    let direct_err = turn_origin::with_origin(
        workflow_origin(&flow_id),
        caps.memory
            .as_ref()
            .expect("build_capabilities must wire a memory capability")
            .remember("user", &forbidden_key, json!("must never be written")),
    )
    .await
    .expect_err("the adapter itself must independently refuse scope: \"user\"");
    assert!(direct_err
        .to_string()
        .contains("only supports scope \"flow\""));

    // ── (c) the user's real, durable GLOBAL_NAMESPACE store is untouched by
    // either attempt above. ──
    let memory = crate::openhuman::memory::global::client_if_ready()
        .expect("global memory client must be initialized by lock_shared_memory")
        .memory_handle();
    let entry = memory
        .get(tinycortex::memory::GLOBAL_NAMESPACE, &forbidden_key)
        .await
        .expect("get should not error");
    assert!(
        entry.is_none(),
        "the memory node must never write to the user's GLOBAL_NAMESPACE store, found: {entry:?}"
    );
}

// ── 4. dry_run_workflow still works with a memory node: MockMemory returns
// shaped data without ever touching the real store ─────────────────────────

#[tokio::test]
async fn memory_node_dry_run_uses_mock_memory_and_never_touches_the_real_store() {
    let _serial = lock_shared_memory().await;
    let flow_id = unique_flow_id("e2e-dryrun");

    let memory = crate::openhuman::memory::global::client_if_ready()
        .expect("global memory client must be initialized by lock_shared_memory")
        .memory_handle();

    // Nothing under this flow's namespace exists yet.
    assert!(memory
        .get(&flow_namespace(&flow_id), "item-42")
        .await
        .unwrap()
        .is_none());

    // The SAME round-trip graph shape as the real-adapter test above, but
    // run against `tinyflows::caps::mock::mock_capabilities()` — exactly
    // what `DryRunWorkflowTool::execute` wires (`Capabilities::memory`
    // defaults to `MockMemory` there; see the crate's own doc comment on
    // `mock_capabilities`).
    let mock_caps = tinyflows::caps::mock::mock_capabilities();

    let remember_graph = trigger_to_memory(json!({
        "operation": "remember",
        "scope": "flow",
        "key": "item-42",
        "value": "should never reach the real store in a dry run"
    }));
    let compiled_remember =
        tinyflows::compiler::compile(&remember_graph).expect("compile remember graph");
    let remember_outcome = tinyflows::engine::run(&compiled_remember, Value::Null, &mock_caps)
        .await
        .expect("dry-run remember should succeed against MockMemory");
    assert_eq!(
        remember_outcome.output["nodes"]["mem"]["items"][0]["json"]["json"]["ok"],
        json!(true)
    );

    // The real store never saw this write — MockMemory::remember is a no-op.
    assert!(
        memory
            .get(&flow_namespace(&flow_id), "item-42")
            .await
            .unwrap()
            .is_none(),
        "dry_run_workflow's MockMemory must never touch the real on-disk store"
    );

    // recall through the same mock returns MockMemory's fixed shaped echo —
    // proving a graph containing a `memory` node still dry-runs cleanly end
    // to end, without ever reaching the real adapter or store.
    let recall_graph = trigger_to_memory(json!({
        "operation": "recall",
        "scope": "flow",
        "query": "item-42"
    }));
    let compiled_recall =
        tinyflows::compiler::compile(&recall_graph).expect("compile recall graph");
    let recall_outcome = tinyflows::engine::run(&compiled_recall, Value::Null, &mock_caps)
        .await
        .expect("dry-run recall should succeed against MockMemory");
    let results = recall_outcome.output["nodes"]["mem"]["items"][0]["json"]["json"]["results"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        results.iter().any(|hit| hit["id"] == json!("mem_1")),
        "expected MockMemory's fixed shaped echo (not real-store content), got: {results:?}"
    );
}
