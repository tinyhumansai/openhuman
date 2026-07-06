use super::*;
use crate::openhuman::config::Config;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};
use serde_json::json;
use tempfile::TempDir;

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

fn policy(level: AutonomyLevel) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy: level,
        ..SecurityPolicy::default()
    })
}

fn valid_graph() -> Value {
    json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Summarize", "config": { "prompt": "hi" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    })
}

// ── revise_workflow ──────────────────────────────────────────────────────────

#[tokio::test]
async fn revise_workflow_validates_and_returns_revision_proposal() {
    let tmp = TempDir::new().unwrap();
    let tool = ReviseWorkflowTool::new(test_config(&tmp));

    let result = tool
        .execute(json!({
            "name": "Revised flow",
            "graph": valid_graph(),
            "instruction": "add a summarize step"
        }))
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["type"], "workflow_proposal");
    assert_eq!(parsed["revision"], true);
    assert_eq!(parsed["name"], "Revised flow");
    assert_eq!(parsed["instruction"], "add a summarize step");
    assert_eq!(parsed["graph"]["nodes"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn revise_workflow_rejects_invalid_graph() {
    let tmp = TempDir::new().unwrap();
    let tool = ReviseWorkflowTool::new(test_config(&tmp));

    let result = tool
        .execute(json!({
            "name": "bad",
            "graph": { "nodes": [ { "id": "a", "kind": "agent", "name": "A" } ], "edges": [] }
        }))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().to_lowercase().contains("invalid"));
}

#[test]
fn revise_workflow_never_persists() {
    // The revise tool shares propose_workflow's human-in-the-loop invariant:
    // no side effect, no permission gate — it only validates and returns.
    let tmp = TempDir::new().unwrap();
    let tool = ReviseWorkflowTool::new(test_config(&tmp));
    assert_eq!(tool.name(), "revise_workflow");
    assert_eq!(tool.permission_level(), PermissionLevel::None);
    assert!(!tool.external_effect());
}

// ── read-only tools ──────────────────────────────────────────────────────────

#[tokio::test]
async fn list_flows_is_read_only_and_lists() {
    let tmp = TempDir::new().unwrap();
    let tool = ListFlowsTool::new(test_config(&tmp));
    assert_eq!(tool.permission_level(), PermissionLevel::None);
    assert!(!tool.external_effect());

    let result = tool.execute(json!({})).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    // No flows saved in a fresh workspace.
    assert!(parsed["flows"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn get_flow_missing_id_is_error() {
    let tmp = TempDir::new().unwrap();
    let tool = GetFlowTool::new(test_config(&tmp));
    assert_eq!(tool.permission_level(), PermissionLevel::None);

    let result = tool.execute(json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Missing 'id'"));
}

#[tokio::test]
async fn get_flow_unknown_id_is_error() {
    let tmp = TempDir::new().unwrap();
    let tool = GetFlowTool::new(test_config(&tmp));

    let result = tool.execute(json!({ "id": "nope" })).await.unwrap();
    assert!(result.is_error);
    assert!(
        result.output().to_lowercase().contains("not found") || result.output().contains("nope")
    );
}

#[tokio::test]
async fn get_flow_run_missing_id_is_error() {
    let tmp = TempDir::new().unwrap();
    let tool = GetFlowRunTool::new(test_config(&tmp));
    assert_eq!(tool.permission_level(), PermissionLevel::None);

    let result = tool.execute(json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Missing 'run_id'"));
}

#[tokio::test]
async fn list_flow_connections_is_read_only() {
    let tmp = TempDir::new().unwrap();
    let tool = ListFlowConnectionsTool::new(test_config(&tmp));
    assert_eq!(tool.permission_level(), PermissionLevel::None);
    assert!(!tool.external_effect());

    let result = tool.execute(json!({})).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert!(parsed["connections"].is_array());
}

// ── search_tool_catalog ──────────────────────────────────────────────────────

#[test]
fn search_curated_catalog_finds_real_gmail_slug() {
    // Grounded search over the curated catalog returns a real slug/scope.
    let results = search_curated_catalog("gmail", Some("gmail"), 40);
    assert!(!results.is_empty(), "gmail catalog should have entries");
    for r in &results {
        assert_eq!(r["toolkit"], "gmail");
        assert!(r["slug"]
            .as_str()
            .unwrap()
            .to_ascii_uppercase()
            .starts_with("GMAIL"));
        assert!(r["scope"].is_string());
    }
}

#[test]
fn search_curated_catalog_all_terms_must_match() {
    // A nonsense term matches nothing.
    let results = search_curated_catalog("zzz_no_such_slug_zzz", None, 40);
    assert!(results.is_empty());
}

#[tokio::test]
async fn search_tool_catalog_tool_is_read_only_and_grounds() {
    let tool = SearchToolCatalogTool::new();
    assert_eq!(tool.name(), "search_tool_catalog");
    assert_eq!(tool.permission_level(), PermissionLevel::None);
    assert!(!tool.external_effect());

    let result = tool
        .execute(json!({ "query": "send", "toolkit": "gmail" }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert!(parsed["count"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn search_tool_catalog_missing_query_is_error() {
    let tool = SearchToolCatalogTool::new();
    let result = tool.execute(json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Missing 'query'"));
}

// ── dry_run_workflow ─────────────────────────────────────────────────────────

#[test]
fn dry_run_is_execute_permission() {
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    assert_eq!(tool.name(), "dry_run_workflow");
    assert_eq!(tool.permission_level(), PermissionLevel::Execute);
    // Mock-backed: no real outbound effect.
    assert!(!tool.external_effect());
}

#[tokio::test]
async fn dry_run_refused_under_readonly_tier() {
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::ReadOnly),
        test_config(&TempDir::new().unwrap()),
    );
    let result = tool
        .execute(json!({ "graph": valid_graph() }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().to_lowercase().contains("read-only"));
}

#[tokio::test]
async fn dry_run_supervised_runs_against_mock_and_labels_sandbox() {
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    let result = tool
        .execute(json!({ "graph": valid_graph(), "input": { "x": 1 } }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["sandbox"], true);
    assert_eq!(parsed["ok"], true);
    assert!(parsed["note"]
        .as_str()
        .unwrap()
        .to_lowercase()
        .contains("sandbox"));
}

#[tokio::test]
async fn dry_run_exercises_agent_ref_node_via_mock_agent_runner() {
    // A draft whose `agent` node selects a named agent kind (`agent_ref`) routes
    // to the `AgentRunner` capability, not the plain LLM. Before wiring the mock
    // runner the sandbox left `agent: None`, so such a draft errored on a missing
    // capability; now `mock_capabilities_with_agent(MockAgentRunner)` echoes the
    // ref and the dry run goes green — proving the builder can self-test drafts
    // that use agent-kind nodes.
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Plan",
              "config": { "agent_ref": "researcher", "prompt": "outline it" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    });
    let result = tool
        .execute(json!({ "graph": graph, "input": { "topic": "x" } }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["sandbox"], true);
    assert_eq!(
        parsed["ok"], true,
        "agent_ref dry-run must be green: {parsed}"
    );
}

#[tokio::test]
async fn dry_run_invalid_graph_is_error() {
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Full),
        test_config(&TempDir::new().unwrap()),
    );
    let result = tool
        .execute(json!({ "graph": { "nodes": [], "edges": [] } }))
        .await
        .unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn dry_run_catches_unwired_required_composio_arg() {
    // Seed the preflight schema cache so no live Composio backend is needed.
    // NOTE: the cache is process-global and other tests seed the `gmail`
    // toolkit too — keep every seeding of GMAIL_SEND_EMAIL identical
    // (`to` + `body`) so test order can't change the outcome.
    let mut entries = std::collections::HashMap::new();
    entries.insert(
        "GMAIL_SEND_EMAIL".to_string(),
        vec!["to".to_string(), "body".to_string()],
    );
    crate::openhuman::tinyflows::caps::seed_required_args_cache("gmail", entries);

    let tmp = TempDir::new().unwrap();
    let tool = DryRunWorkflowTool::new(policy(AutonomyLevel::Supervised), test_config(&tmp));

    let graph_with = |args: Value| {
        json!({
            "nodes": [
                { "id": "t", "kind": "trigger", "name": "Manual" },
                { "id": "send", "kind": "tool_call", "name": "Send email",
                  "config": { "slug": "GMAIL_SEND_EMAIL", "args": args } }
            ],
            "edges": [ { "from_node": "t", "to_node": "send" } ]
        })
    };

    // `to` is a `=`-expression that misses (trigger input has no `email`):
    // the dry run must fail BEFORE the (mock) tool call, naming the field.
    let result = tool
        .execute(json!({
            "graph": graph_with(json!({ "to": "=item.email", "body": "hello" })),
            "input": {}
        }))
        .await
        .unwrap();
    let out = result.output();
    assert!(
        out.contains("`to`") && out.contains("required"),
        "dry run must name the unwired required arg: {out}"
    );

    // The same flow with `to` wired from the trigger passes the preflight.
    let result = tool
        .execute(json!({
            "graph": graph_with(json!({ "to": "=item.email", "body": "hello" })),
            "input": { "email": "a@b.com" }
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["sandbox"], true);
    assert_eq!(
        parsed["ok"], true,
        "wired flow must dry-run green: {parsed}"
    );
}

// ── dry_run_workflow: null-resolution check ─────────────────────────────────

#[tokio::test]
async fn dry_run_flags_tool_call_arg_null_resolved_from_unschemad_agent() {
    // The `summarize` agent has no `output_parser.schema`, so (via the
    // schema-aware mock agent) its structured output has no `channel` field —
    // the exact "builds but does nothing" shape this check exists to catch.
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "agent_ref": "researcher", "prompt": "summarize" } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "oh:noop",
                "args": { "channel": "=nodes.summarize.item.json.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "post" }
        ]
    });

    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["sandbox"], true,
        "still labeled a sandbox result: {parsed}"
    );
    assert_eq!(
        parsed["ok"], false,
        "a null-resolved tool_call arg must fail the dry run: {parsed}"
    );
    let null_resolutions = parsed["null_resolutions"]
        .as_array()
        .expect("null_resolutions array");
    assert_eq!(null_resolutions.len(), 1, "{parsed}");
    assert_eq!(null_resolutions[0]["node_id"], "post");
    assert_eq!(null_resolutions[0]["location"], "args.channel");
    assert_eq!(
        null_resolutions[0]["expression"],
        "=nodes.summarize.item.json.channel"
    );
    assert!(
        parsed["message"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("output_parser"),
        "{parsed}"
    );
}

#[tokio::test]
async fn dry_run_passes_when_agent_schema_matches_tool_call_binding() {
    // The FALSE-POSITIVE-PREVENTION case: `summarize` DOES declare a schema
    // covering `channel`, and `post` binds exactly that field. Without the
    // schema-aware mock agent (i.e. with the vendored `MockAgentRunner`, which
    // always echoes `{ agent, request, connection }` regardless of schema)
    // this would incorrectly fail — proving the mock is what makes the check
    // accurate rather than perpetually red for correctly-built graphs.
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "agent_ref": "researcher", "prompt": "summarize",
                "output_parser": { "schema": { "type": "object",
                    "required": ["channel"],
                    "properties": { "channel": { "type": "string" } } } } } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "oh:noop",
                "args": { "channel": "=nodes.summarize.item.json.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "post" }
        ]
    });

    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["ok"], true,
        "schema-aware mock must satisfy the declared schema: {parsed}"
    );
    assert!(
        parsed["null_resolutions"].as_array().unwrap().is_empty(),
        "{parsed}"
    );
}

#[tokio::test]
async fn dry_run_passes_when_tool_call_binds_to_upstream_tool_output() {
    // A `tool_call` binding to another `tool_call`'s real output (not an
    // agent at all) must not be affected by the agent-schema machinery above.
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "lookup", "kind": "tool_call", "name": "Lookup",
              "config": { "slug": "oh:lookup", "args": {} } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "oh:noop",
                "args": { "channel": "=nodes.lookup.item.json.tool" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "lookup" },
            { "from_node": "lookup", "to_node": "post" }
        ]
    });

    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["ok"], true, "{parsed}");
    assert!(
        parsed["null_resolutions"].as_array().unwrap().is_empty(),
        "{parsed}"
    );
}

#[tokio::test]
async fn dry_run_flags_tool_call_error_when_on_error_is_route() {
    // `on_error: "route"` converts the preflight failure into a routed error
    // ITEM so the SANDBOX RUN as a whole still completes (`Ok(outcome)`) —
    // exactly the case the naive `null_resolutions`-only check would miss,
    // because the failing node's diagnostics stay empty (the engine never
    // got far enough to trace an `=`-expression before the preflight error).
    // Seed the same schema as `dry_run_catches_unwired_required_composio_arg`
    // (process-global cache; keep the arg list identical across tests).
    let mut entries = std::collections::HashMap::new();
    entries.insert(
        "GMAIL_SEND_EMAIL".to_string(),
        vec!["to".to_string(), "body".to_string()],
    );
    crate::openhuman::tinyflows::caps::seed_required_args_cache("gmail", entries);

    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Send email",
              "config": { "slug": "GMAIL_SEND_EMAIL", "on_error": "route",
                "args": { "to": "=item.email", "body": "hello" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    });

    // `to` misses (trigger input has no `email`) — a real run would fail the
    // preflight; `on_error: "route"` must not let that slip through as `ok: true`.
    let result = tool
        .execute(json!({ "graph": graph, "input": {} }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["ok"], false,
        "on_error: route must not mask a real tool_call failure: {parsed}"
    );
    let node_errors = parsed["node_errors"].as_array().expect("node_errors array");
    assert_eq!(node_errors.len(), 1, "{parsed}");
    assert_eq!(node_errors[0]["node_id"], "post");
    assert!(
        node_errors[0]["error"].as_str().unwrap().contains("to"),
        "error must name the missing field: {parsed}"
    );
}

#[tokio::test]
async fn dry_run_flags_tool_call_error_when_on_error_is_continue() {
    // Same case as above, but `on_error: "continue"` — the other policy that
    // converts a node failure into routed data instead of failing the run.
    let mut entries = std::collections::HashMap::new();
    entries.insert(
        "GMAIL_SEND_EMAIL".to_string(),
        vec!["to".to_string(), "body".to_string()],
    );
    crate::openhuman::tinyflows::caps::seed_required_args_cache("gmail", entries);

    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Send email",
              "config": { "slug": "GMAIL_SEND_EMAIL", "on_error": "continue",
                "args": { "to": "=item.email", "body": "hello" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    });

    let result = tool
        .execute(json!({ "graph": graph, "input": {} }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["ok"], false,
        "on_error: continue must not mask a real tool_call failure: {parsed}"
    );
    assert_eq!(
        parsed["node_errors"].as_array().unwrap().len(),
        1,
        "{parsed}"
    );
}

#[tokio::test]
async fn dry_run_passes_when_agent_enum_schema_binds_to_tool_call() {
    // The agent declares an `enum`-constrained field; the schema-aware mock
    // must synthesize an ALLOWED value (not a generic `""` placeholder, which
    // would fail the vendored validator's `enum` check) so a correctly-built
    // graph using an enum schema dry-runs green instead of false-positiving.
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "triage", "kind": "agent", "name": "Triage",
              "config": { "agent_ref": "researcher", "prompt": "triage this",
                "output_parser": { "schema": { "type": "object",
                    "required": ["priority"],
                    "properties": {
                        "priority": { "type": "string", "enum": ["urgent", "normal"] }
                    } } } } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "oh:noop",
                "args": { "priority": "=nodes.triage.item.json.priority" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "triage" },
            { "from_node": "triage", "to_node": "post" }
        ]
    });

    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["ok"], true,
        "enum-schema agent must dry-run green: {parsed}"
    );
    assert!(parsed["null_resolutions"].as_array().unwrap().is_empty());
    assert!(parsed["node_errors"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn revise_workflow_warns_on_unwired_required_composio_arg() {
    let mut entries = std::collections::HashMap::new();
    entries.insert(
        "GMAIL_SEND_EMAIL".to_string(),
        vec!["to".to_string(), "body".to_string()],
    );
    crate::openhuman::tinyflows::caps::seed_required_args_cache("gmail", entries);

    let tmp = TempDir::new().unwrap();
    let tool = ReviseWorkflowTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({
            "name": "Send mail",
            "graph": {
                "nodes": [
                    { "id": "t", "kind": "trigger", "name": "Manual" },
                    { "id": "send", "kind": "tool_call", "name": "Send",
                      // `body` wired via expression (counts as wired); `to` absent.
                      "config": { "slug": "GMAIL_SEND_EMAIL",
                                  "args": { "body": "=item.text" } } }
                ],
                "edges": [ { "from_node": "t", "to_node": "send" } ]
            }
        }))
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    let warnings = parsed["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|w| {
            let w = w.as_str().unwrap_or_default();
            w.contains("`to`") && w.contains("send")
        }),
        "expected a warning naming node `send` and arg `to`: {warnings:?}"
    );
    // `body` is wired (expression) — no warning for it.
    assert!(
        !warnings
            .iter()
            .any(|w| w.as_str().unwrap_or_default().contains("`body`")),
        "wired arg must not warn: {warnings:?}"
    );
}

// ── save_workflow ────────────────────────────────────────────────────────────

/// Seed a saved flow to write into (the instant-create path does this via
/// `flows_create` before delegating to the builder).
async fn seed_flow(config: &Arc<Config>, name: &str) -> String {
    let outcome = ops::flows_create(
        config,
        name.to_string(),
        json!({
            "nodes": [ { "id": "t", "kind": "trigger", "name": "Manual" } ],
            "edges": []
        }),
        true,
    )
    .await
    .unwrap();
    outcome.value.id
}

#[tokio::test]
async fn save_workflow_missing_flow_id_is_error() {
    let tmp = TempDir::new().unwrap();
    let tool = SaveWorkflowTool::new(test_config(&tmp));
    // Persisting a definition is a Write-class action (no external effect at
    // save time — the flow's own runs govern that).
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
    assert!(!tool.external_effect());

    let result = tool
        .execute(json!({ "graph": valid_graph() }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Missing 'flow_id'"));
}

#[tokio::test]
async fn save_workflow_unknown_flow_is_error() {
    let tmp = TempDir::new().unwrap();
    let tool = SaveWorkflowTool::new(test_config(&tmp));

    let result = tool
        .execute(json!({ "flow_id": "nope", "graph": valid_graph() }))
        .await
        .unwrap();
    assert!(result.is_error, "save onto a nonexistent flow must fail");
    assert!(result.output().contains("nope"));
}

#[tokio::test]
async fn save_workflow_persists_graph_and_name_onto_existing_flow() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow_id = seed_flow(&config, "Blank flow").await;
    let tool = SaveWorkflowTool::new(config.clone());

    let result = tool
        .execute(json!({
            "flow_id": flow_id,
            "graph": valid_graph(),
            "name": "AI News Digest"
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());

    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["type"], "workflow_saved");
    assert_eq!(parsed["flow_id"], flow_id.as_str());
    assert_eq!(parsed["name"], "AI News Digest");
    assert_eq!(parsed["node_count"], 2);
    // Enablement / approval gate are NOT touched by the tool.
    assert_eq!(parsed["require_approval"], true);

    // The graph + name really persisted.
    let saved = ops::flows_get(&config, &flow_id).await.unwrap().value;
    assert_eq!(saved.name, "AI News Digest");
    assert_eq!(saved.graph.nodes.len(), 2);
}

#[tokio::test]
async fn save_workflow_rejects_invalid_graph_and_leaves_flow_intact() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow_id = seed_flow(&config, "Blank flow").await;
    let tool = SaveWorkflowTool::new(config.clone());

    let result = tool
        .execute(json!({
            "flow_id": flow_id,
            // No trigger node — fails tinyflows validation.
            "graph": { "nodes": [ { "id": "a", "kind": "agent", "name": "A" } ], "edges": [] }
        }))
        .await
        .unwrap();
    assert!(result.is_error);

    let saved = ops::flows_get(&config, &flow_id).await.unwrap().value;
    assert_eq!(saved.name, "Blank flow");
    assert_eq!(
        saved.graph.nodes.len(),
        1,
        "original graph must be untouched"
    );
}

// ── save_workflow: enforcing binding-resolvability gate ─────────────────────

/// The proven live-failure shape (same as
/// `tools_tests::propose_workflow_rejects_unschemad_agent_binding`): a
/// `summarize` agent with no `output_parser.schema`, and a `notify` tool_call
/// binding `args.channel` to its (unschemad, therefore unresolvable) output.
fn unresolvable_binding_graph() -> Value {
    json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "agent_ref": "researcher", "prompt": "summarize" } },
            { "id": "notify", "kind": "tool_call", "name": "Notify",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "=nodes.summarize.item.json.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "notify" }
        ]
    })
}

#[tokio::test]
async fn save_workflow_rejects_unschemad_agent_binding() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow_id = seed_flow(&config, "Blank flow").await;
    let tool = SaveWorkflowTool::new(config.clone());

    let result = tool
        .execute(json!({ "flow_id": flow_id, "graph": unresolvable_binding_graph() }))
        .await
        .unwrap();

    assert!(result.is_error, "must be rejected: {}", result.output());
    let output = result.output();
    assert!(output.contains("notify"), "{output}");
    assert!(output.contains("channel"), "{output}");
    assert!(output.contains("summarize"), "{output}");
    assert!(output.contains("output_parser.schema"), "{output}");

    // The flow it tried to save onto must be untouched.
    let saved = ops::flows_get(&config, &flow_id).await.unwrap().value;
    assert_eq!(saved.name, "Blank flow");
    assert_eq!(
        saved.graph.nodes.len(),
        1,
        "original graph must be untouched"
    );
}

#[tokio::test]
async fn save_workflow_accepts_correctly_schemad_graph() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow_id = seed_flow(&config, "Blank flow").await;
    let tool = SaveWorkflowTool::new(config.clone());

    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "agent_ref": "researcher", "prompt": "summarize",
                "output_parser": { "schema": { "type": "object",
                    "required": ["channel"],
                    "properties": { "channel": { "type": "string" } } } } } },
            { "id": "notify", "kind": "tool_call", "name": "Notify",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "=nodes.summarize.item.json.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "notify" }
        ]
    });

    let result = tool
        .execute(json!({ "flow_id": flow_id, "graph": graph, "name": "Summarize and notify" }))
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["type"], "workflow_saved");
    assert_eq!(parsed["node_count"], 3);

    let saved = ops::flows_get(&config, &flow_id).await.unwrap().value;
    assert_eq!(saved.name, "Summarize and notify");
    assert_eq!(saved.graph.nodes.len(), 3);
}
