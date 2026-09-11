use super::*;
use crate::openhuman::security::AutonomyLevel;
use tempfile::TempDir;

// Seeding still goes through the engine handle (`UnifiedMemory` above),
// but the value types are the CONTRACT's: `tinymemory_core` re-exports
// `tinymemory_api::types::{MemoryCategory, MemoryTaint}` verbatim
// (tinymemory#18 §A1 moved them onto the contract so a second engine could
// be bound without translating). Naming them at the contract keeps the
// alias honest and is one fewer reason this file holds the crate (#5560).
use crate::openhuman::memory::api::types::{
    MemoryCategory as EngineCategory, MemoryTaint as EngineTaint,
};

// ── Why `UnifiedMemory` is still here, and what replacing it costs (#5560)
//
// This whole module is `#[cfg(test)]`, so the engine is not linked into a
// production build from this file; the crate survives #5560 as a
// dev-dependency and this is the kind of fixture that keeps it. What the
// fixture cannot do is move to the bound driver in a one-line swap, and
// there are two independent reasons worth recording before someone tries.
//
// 1. **The seam is a different trait.** `test_mem` hands back
//    `Arc<dyn memory::Memory>` (= `tinymemory_api::traits::Memory`), and
//    `Arc<dyn MemoryProvider>` does not coerce to it: `MemoryProvider`'s
//    supertrait is `MemoryCore`, which is a *different* trait with taint as
//    an argument rather than a second method (see `provider/mandatory.rs`,
//    which says so at the definition). Rebinding the fixture onto
//    `memory::test_support::install_memory_driver_for_test` therefore rewrites
//    every `mem.store_with_taint(..)` / `mem.get(..)` in this module, not
//    just its two lines.
// 2. **The backend choice is load-bearing.** `FLOW_MEMORY_NAMESPACE_PREFIX`'s
//    doc above turns on `UnifiedMemory::sanitize_namespace` disagreeing with
//    `Memory::forget`'s raw `WHERE namespace = ?1`. A volatile stand-in such
//    as `tinycortex::memory::store::InMemoryMemoryStore` implements the same
//    `Memory` trait and would compile, but it does not reproduce that
//    inconsistency — so it would quietly stop testing the property the
//    prefix exists to satisfy.
//
// Every test below that touches `test_mem` is already `#[ignore]`d, and
// their ignore reason is the deeper problem: the tools resolve the *bound
// driver*, so a store seeded here is not the store they read. Making them
// coherent again is a fixture rewrite against the binding — a change worth
// making on its own, with the tests un-ignored so the result is verified,
// not folded into a dependency-shedding pass.

fn test_security() -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy::default())
}

fn test_mem() -> (TempDir, Arc<dyn crate::openhuman::memory::Memory>) {
    let tmp = TempDir::new().unwrap();
    let mem = crate::openhuman::memory::tool_memory::test_helpers::MockMemory::default();
    (tmp, Arc::new(mem))
}

// ── flow_namespace / FLOW_MEMORY_NAMESPACE_PREFIX ───────────────
// (relocated from `flows::mod` — see that module's re-export comment)

#[test]
fn flow_namespace_uses_the_shared_root_prefix() {
    assert_eq!(flow_namespace("abc-123"), "flow_abc-123");
    assert!(flow_namespace("abc-123").starts_with(FLOW_MEMORY_NAMESPACE_PREFIX));
}

#[test]
fn flow_namespace_is_distinct_per_flow() {
    assert_ne!(flow_namespace("a"), flow_namespace("b"));
}

// ── FlowMemoryRecallTool ────────────────────────────────────────

#[test]
fn recall_name_and_schema() {
    let tool = FlowMemoryRecallTool::new();
    assert_eq!(tool.name(), "flow_memory_recall");
    let schema = tool.parameters_schema();
    assert!(schema["properties"]["query"].is_object());
    assert!(schema["properties"]["flow_id"].is_object());
    assert!(schema["properties"]["scope"].is_object());
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn recall_empty_returns_no_results() {
    let (_tmp, _mem) = test_mem();
    let tool = FlowMemoryRecallTool::new();
    let result = tool
        .execute(json!({"query": "anything", "flow_id": "f1"}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.output().contains("No memories found"));
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn store_then_recall_matches() {
    let (_tmp, mem) = test_mem();
    mem.store_with_taint(
        &flow_namespace("f1"),
        "sent_item_42",
        "Sent newsletter item 42 to subscribers",
        EngineCategory::Core,
        None,
        EngineTaint::ExternalSync,
    )
    .await
    .unwrap();

    let tool = FlowMemoryRecallTool::new();
    let result = tool
        .execute(json!({"query": "newsletter item 42", "flow_id": "f1"}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.output().contains("newsletter item 42") || result.output().contains("42"));
    assert!(result.output().contains("Found 1"));
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn scope_flow_isolates_to_own_namespace() {
    let (_tmp, mem) = test_mem();
    mem.store_with_taint(
        &flow_namespace("f1"),
        "k",
        "shared keyword hit",
        EngineCategory::Core,
        None,
        EngineTaint::ExternalSync,
    )
    .await
    .unwrap();
    mem.store_with_taint(
        &flow_namespace("f2"),
        "k",
        "shared keyword hit",
        EngineCategory::Core,
        None,
        EngineTaint::ExternalSync,
    )
    .await
    .unwrap();

    let tool = FlowMemoryRecallTool::new();
    let result = tool
        .execute(json!({"query": "shared keyword", "flow_id": "f1", "scope": "flow"}))
        .await
        .unwrap();
    assert!(!result.is_error);
    // Only f1's own entry, not f2's.
    assert!(result.output().contains("Found 1"));
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn scope_flows_crosses_namespaces() {
    let (_tmp, mem) = test_mem();
    mem.store_with_taint(
        &flow_namespace("f1"),
        "k",
        "shared keyword hit from f1",
        EngineCategory::Core,
        None,
        EngineTaint::ExternalSync,
    )
    .await
    .unwrap();
    mem.store_with_taint(
        &flow_namespace("f2"),
        "k",
        "shared keyword hit from f2",
        EngineCategory::Core,
        None,
        EngineTaint::ExternalSync,
    )
    .await
    .unwrap();

    let tool = FlowMemoryRecallTool::new();
    let result = tool
        .execute(json!({"query": "shared keyword", "flow_id": "f1", "scope": "flows"}))
        .await
        .unwrap();
    assert!(!result.is_error);
    // Both f1's and f2's namespaces are visible under scope="flows".
    assert!(result.output().contains("Found 2"));
}

// T-m5: a missing/invalid input param reports via `ToolResult::error`
// (an `Ok(..)` the model can read and react to in-turn), never
// `Err(anyhow!)` (a hard tool-invocation failure) — matching every other
// input-validation problem on this belt (see the scope/empty-value
// tests above, which already used this channel before the fix).
#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn recall_missing_query_errs() {
    let (_tmp, _mem) = test_mem();
    let tool = FlowMemoryRecallTool::new();
    let result = tool.execute(json!({"flow_id": "f1"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Missing 'query'"));
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn recall_missing_flow_id_errs() {
    let (_tmp, _mem) = test_mem();
    let tool = FlowMemoryRecallTool::new();
    let result = tool.execute(json!({"query": "anything"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Missing 'flow_id'"));
}

// ── FlowMemoryRememberTool ──────────────────────────────────────

#[test]
fn remember_name_and_schema() {
    let (_tmp, _mem) = test_mem();
    let tool = FlowMemoryRememberTool::new(test_security());
    assert_eq!(tool.name(), "flow_memory_remember");
    let schema = tool.parameters_schema();
    assert!(schema["properties"]["flow_id"].is_object());
    assert!(schema["properties"]["key"].is_object());
    assert!(schema["properties"]["content"].is_object());
    // No `namespace` parameter exists — the security invariant that a
    // flow can never target another namespace.
    assert!(schema["properties"]["namespace"].is_null());
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
}

/// Helper: a trusted `TrustedAutomation { Workflow }` origin scoped to
/// `job_id`, the only source `flow_memory_remember` will act on since the
/// T-M2 fix.
fn trusted_workflow_origin(job_id: &str) -> AgentTurnOrigin {
    AgentTurnOrigin::TrustedAutomation {
        job_id: job_id.to_string(),
        source: TrustedAutomationSource::Workflow {
            require_approval: false,
        },
    }
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn remember_stores_with_external_sync_taint() {
    let (_tmp, mem) = test_mem();
    let tool = FlowMemoryRememberTool::new(test_security());
    let result = turn_origin::with_origin(
        trusted_workflow_origin("f1"),
        tool.execute(json!({"flow_id": "f1", "key": "sent_item_42", "content": "Sent item 42"})),
    )
    .await
    .unwrap();
    assert!(!result.is_error);

    let entry = mem
        .get(&flow_namespace("f1"), "sent_item_42")
        .await
        .unwrap()
        .expect("entry should be stored");
    assert_eq!(entry.content, "Sent item 42");
    assert_eq!(entry.taint, EngineTaint::ExternalSync);
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn remember_writes_only_to_own_flow_namespace() {
    let (_tmp, mem) = test_mem();
    let tool = FlowMemoryRememberTool::new(test_security());
    turn_origin::with_origin(
        trusted_workflow_origin("f1"),
        tool.execute(json!({"flow_id": "f1", "key": "k", "content": "f1 content"})),
    )
    .await
    .unwrap();

    // Never lands in another flow's namespace, the shared "flows" scope
    // namespace, or global/user memory.
    assert!(mem.get(&flow_namespace("f2"), "k").await.unwrap().is_none());
    assert!(mem.get("global", "k").await.unwrap().is_none());
    assert!(mem
        .get("f1", "k") // raw flow_id, not the derived namespace
        .await
        .unwrap()
        .is_none());
}

/// SECURITY (T-M2): the primary fix under test — a chat/orchestrator turn
/// (no trusted `Workflow` run origin) must be refused outright, never
/// routed to the model-supplied `flow_id`. This is the exact
/// prompt-injection scenario: an attacker-controlled chat turn calling
/// `flow_memory_remember` with another flow's id to poison its dedup
/// memory (e.g. mark an item as already-sent so a digest flow skips it
/// forever).
#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn remember_refuses_outside_a_trusted_workflow_run() {
    let (_tmp, mem) = test_mem();
    let tool = FlowMemoryRememberTool::new(test_security());

    // No `turn_origin::with_origin` wrapper — this call has no trusted
    // Workflow run origin, exactly like every chat/orchestrator turn.
    let result = tool
        .execute(json!({
            "flow_id": "digest-flow-victim",
            "key": "sent_item_42",
            "content": "Sent newsletter item 42 to subscribers"
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result
        .output()
        .contains("only available inside a workflow run"));
    // Nothing was written to the targeted namespace — or anywhere else.
    assert!(mem
        .get(&flow_namespace("digest-flow-victim"), "sent_item_42")
        .await
        .unwrap()
        .is_none());
}

/// SECURITY (Fix 1): a running flow's own id — carried by the run's
/// `TrustedAutomation { Workflow }` origin — is authoritative. A
/// mismatched, model-supplied `flow_id` arg must be silently ignored,
/// never allowed to redirect the write into a different flow's
/// namespace.
#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn remember_ignores_mismatched_flow_id_arg_inside_trusted_workflow_run() {
    let (_tmp, mem) = test_mem();
    let tool = FlowMemoryRememberTool::new(test_security());

    let origin = AgentTurnOrigin::TrustedAutomation {
        job_id: "f-real".to_string(),
        source: TrustedAutomationSource::Workflow {
            require_approval: false,
        },
    };
    let result = turn_origin::with_origin(
        origin,
        tool.execute(json!({
            "flow_id": "f-other",
            "key": "sent_item_1",
            "content": "Sent item 1"
        })),
    )
    .await
    .unwrap();
    assert!(!result.is_error);

    // Landed in the trusted run's own namespace...
    assert!(mem
        .get(&flow_namespace("f-real"), "sent_item_1")
        .await
        .unwrap()
        .is_some());
    // ...and NOT the mismatched, model-supplied namespace.
    assert!(mem
        .get(&flow_namespace("f-other"), "sent_item_1")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn remember_blocked_in_readonly_autonomy() {
    let (_tmp, mem) = test_mem();
    let readonly = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    });
    let tool = FlowMemoryRememberTool::new(readonly);
    let result = tool
        .execute(json!({"flow_id": "f1", "key": "k", "content": "blocked"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("read-only mode"));
    assert!(mem.get(&flow_namespace("f1"), "k").await.unwrap().is_none());
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn remember_rejects_secret_like_content() {
    let (_tmp, mem) = test_mem();
    let tool = FlowMemoryRememberTool::new(test_security());
    let result = turn_origin::with_origin(
        trusted_workflow_origin("f1"),
        tool.execute(json!({
            "flow_id": "f1",
            "key": "api",
            "content": "api_key=sk-123456789012345678901234567890"
        })),
    )
    .await
    .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("looks like a secret"));
    assert!(mem
        .get(&flow_namespace("f1"), "api")
        .await
        .unwrap()
        .is_none());
}

/// Outside a trusted run, `flow_id` no longer matters — the T-M2 refusal
/// fires regardless of whether it was supplied. (Inside a trusted run the
/// arg is informational only and ignored either way — see
/// `remember_ignores_mismatched_flow_id_arg_inside_trusted_workflow_run`.)
#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn remember_missing_flow_id_outside_trusted_run_is_refused() {
    let (_tmp, _mem) = test_mem();
    let tool = FlowMemoryRememberTool::new(test_security());
    let result = tool
        .execute(json!({"key": "k", "content": "c"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result
        .output()
        .contains("only available inside a workflow run"));
}

// T-m5 (retained through the T-M2 merge): the missing-param checks run
// BEFORE the trusted-origin resolution, so they are still reachable outside
// a run and still assert the `ToolResult::error` channel rather than `Err`.
#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn remember_missing_key_errs() {
    let (_tmp, _mem) = test_mem();
    let tool = FlowMemoryRememberTool::new(test_security());
    let result = tool
        .execute(json!({"flow_id": "f1", "content": "c"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Missing 'key'"));
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn remember_missing_content_errs() {
    let (_tmp, _mem) = test_mem();
    let tool = FlowMemoryRememberTool::new(test_security());
    let result = tool
        .execute(json!({"flow_id": "f1", "key": "k"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Missing 'content'"));
}
