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
async fn revise_workflow_omitted_require_approval_defaults_true() {
    let tmp = TempDir::new().unwrap();
    let tool = ReviseWorkflowTool::new(test_config(&tmp));

    let result = tool
        .execute(json!({ "name": "Revised flow", "graph": valid_graph() }))
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["require_approval"], true);
}

#[tokio::test]
async fn revise_workflow_explicit_require_approval_true_is_respected() {
    let tmp = TempDir::new().unwrap();
    let tool = ReviseWorkflowTool::new(test_config(&tmp));

    let result = tool
        .execute(json!({
            "name": "Revised flow",
            "graph": valid_graph(),
            "require_approval": true
        }))
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["require_approval"], true);
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

// ── search_tool_catalog / get_tool_contract ─────────────────────────────────
// The live-catalog cache is process-global (`LIVE_CATALOG_CACHE`) — every
// test below seeds the exact toolkit(s)/contract(s) it needs via
// `seed_live_catalog_cache` so none of this touches a live Composio backend,
// and keeps each toolkit's seeded contents self-consistent across tests that
// share a toolkit key (same discipline the pre-fix required-args/response-
// fields caches already required).

use crate::openhuman::tinyflows::caps::{seed_live_catalog_cache, ToolContract};

fn seeded_gmail_send_contract() -> ToolContract {
    ToolContract {
        slug: "GMAIL_SEND_EMAIL".to_string(),
        toolkit: "gmail".to_string(),
        description: Some("Send an email".to_string()),
        required_args: vec!["to".to_string(), "body".to_string()],
        input_schema: Some(json!({ "type": "object", "required": ["to", "body"] })),
        output_fields: vec!["id".to_string(), "threadId".to_string()],
        output_schema: Some(json!({
            "type": "object",
            "properties": { "id": {"type": "string"}, "threadId": {"type": "string"} }
        })),
        primary_array_path: None,
        is_curated: true,
    }
}

#[tokio::test]
async fn search_live_catalog_finds_a_seeded_real_gmail_slug() {
    seed_live_catalog_cache("gmail", vec![seeded_gmail_send_contract()]);
    let config = Config::default();
    let results = search_live_catalog(&config, "send", Some("gmail"), 40).await;
    assert!(!results.is_empty(), "gmail catalog should have entries");
    for r in &results {
        assert_eq!(r["toolkit"], "gmail");
        assert!(r["slug"]
            .as_str()
            .unwrap()
            .to_ascii_uppercase()
            .starts_with("GMAIL"));
        assert_eq!(r["featured"], true);
    }
}

#[tokio::test]
async fn search_live_catalog_all_terms_must_match() {
    seed_live_catalog_cache("gmail", vec![seeded_gmail_send_contract()]);
    let config = Config::default();
    // A nonsense term matches nothing.
    let results = search_live_catalog(&config, "zzz_no_such_slug_zzz", Some("gmail"), 40).await;
    assert!(results.is_empty());
}

#[tokio::test]
async fn search_live_catalog_ranks_curated_before_uncurated_without_hiding_either() {
    // Uses its own cache key (never `"gmail"`) — the process-global
    // `LIVE_CATALOG_CACHE` is shared with every other `#[tokio::test]` in
    // this file, most of which seed `"gmail"` with a single curated entry.
    // This test's 2-item, exact-order assertion would be flaky if a
    // concurrently-running test's `seed_live_catalog_cache("gmail", ..)`
    // replaced the entry between this seed and the query below.
    let mut uncurated = seeded_gmail_send_contract();
    uncurated.slug = "GMAIL_UNCURATED_SEND".to_string();
    uncurated.is_curated = false;
    seed_live_catalog_cache(
        "gmailranktest",
        vec![uncurated, seeded_gmail_send_contract()],
    );

    let config = Config::default();
    let results = search_live_catalog(&config, "send", Some("gmailranktest"), 40).await;
    assert_eq!(results.len(), 2, "a real, uncurated action is never hidden");
    assert_eq!(results[0]["featured"], true, "curated match ranks first");
    assert_eq!(results[1]["featured"], false);
}

#[tokio::test]
async fn search_tool_catalog_tool_is_read_only_and_grounds() {
    seed_live_catalog_cache("gmail", vec![seeded_gmail_send_contract()]);
    let tmp = TempDir::new().unwrap();
    let tool = SearchToolCatalogTool::new(test_config(&tmp));
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
    let tmp = TempDir::new().unwrap();
    let tool = SearchToolCatalogTool::new(test_config(&tmp));
    let result = tool.execute(json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Missing 'query'"));
}

#[tokio::test]
async fn search_tool_catalog_grounds_output_fields_from_the_live_catalog() {
    // A known action's real output schema (seeded, standing in for a live
    // Composio fetch) surfaces as real `output_fields`/`required_args` on
    // the match — no separate per-slug lookup needed anymore.
    seed_live_catalog_cache("gmail", vec![seeded_gmail_send_contract()]);
    let tmp = TempDir::new().unwrap();
    let tool = SearchToolCatalogTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({ "query": "send", "toolkit": "gmail" }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    let results = parsed["results"].as_array().unwrap();
    let send_email = results
        .iter()
        .find(|r| r["slug"] == "GMAIL_SEND_EMAIL")
        .expect("GMAIL_SEND_EMAIL should be in the live catalog");
    let fields: Vec<&str> = send_email["output_fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(fields, vec!["id", "threadId"]);
    assert_eq!(send_email["required_args"], json!(["to", "body"]));
}

#[tokio::test]
async fn search_tool_catalog_degrades_gracefully_when_output_schema_unknown() {
    // The seeded action has no output schema — the tool must still succeed,
    // with an empty `output_fields` list rather than erroring. Uses its own
    // fictional toolkit key (never the real `"slack"` key) — `slack` is a
    // statically-catalogued toolkit elsewhere in this test suite (e.g.
    // `ops_tests.rs`'s `validate_tool_contracts` tests), and this fixture's
    // `is_curated: false` would otherwise race with those tests over the
    // shared process-global `LIVE_CATALOG_CACHE` entry for `"slack"`.
    seed_live_catalog_cache(
        "slackschematest",
        vec![ToolContract {
            slug: "SLACKSCHEMATEST_SEND_MESSAGE".to_string(),
            toolkit: "slackschematest".to_string(),
            description: None,
            required_args: vec!["channel".to_string()],
            input_schema: None,
            output_fields: Vec::new(),
            output_schema: None,
            primary_array_path: None,
            is_curated: false,
        }],
    );

    let tmp = TempDir::new().unwrap();
    let tool = SearchToolCatalogTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({ "query": "send", "toolkit": "slackschematest" }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    let results = parsed["results"].as_array().unwrap();
    assert!(!results.is_empty(), "slack catalog should have entries");
    for r in results {
        assert!(r["output_fields"].as_array().unwrap().is_empty());
        assert_eq!(r["featured"], false);
    }
}

// ── get_tool_contract ────────────────────────────────────────────────────────

#[tokio::test]
async fn get_tool_contract_returns_the_full_seeded_contract() {
    seed_live_catalog_cache("gmail", vec![seeded_gmail_send_contract()]);
    let tmp = TempDir::new().unwrap();
    let tool = GetToolContractTool::new(test_config(&tmp));
    assert_eq!(tool.name(), "get_tool_contract");
    assert_eq!(tool.permission_level(), PermissionLevel::None);
    assert!(!tool.external_effect());

    let result = tool
        .execute(json!({ "slug": "GMAIL_SEND_EMAIL" }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["slug"], "GMAIL_SEND_EMAIL");
    assert_eq!(parsed["toolkit"], "gmail");
    assert_eq!(parsed["required_args"], json!(["to", "body"]));
    assert_eq!(parsed["output_fields"], json!(["id", "threadId"]));
    assert!(parsed["output_schema"].is_object());
    assert!(parsed["input_schema"].is_object());
}

#[tokio::test]
async fn get_tool_contract_missing_slug_is_error() {
    let tmp = TempDir::new().unwrap();
    let tool = GetToolContractTool::new(test_config(&tmp));
    let result = tool.execute(json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Missing 'slug'"));
}

#[tokio::test]
async fn get_tool_contract_rejects_a_hallucinated_slug() {
    seed_live_catalog_cache("gmail", vec![seeded_gmail_send_contract()]);
    let tmp = TempDir::new().unwrap();
    let tool = GetToolContractTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({ "slug": "GMAIL_DOES_NOT_EXIST" }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("not a real action"));
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
    seed_live_catalog_cache("gmail", vec![seeded_gmail_send_contract()]);

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
    //
    // The graph must give `post`'s `error` port a real destination: vendored
    // tinyflows' author-time `validate()` (added alongside per-node error
    // handling — a graph with `on_error: "route"` but no outgoing `error`-port
    // edge is now rejected up front, since a route with nowhere to go is
    // always a dead-end) would otherwise reject this graph before the sandbox
    // run ever starts, which is a different failure mode than the one this
    // test targets. `recover` is a no-op sink, same convention as
    // `dry_run_passes_when_tool_call_binds_to_upstream_tool_output` above.
    seed_live_catalog_cache("gmail", vec![seeded_gmail_send_contract()]);

    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Send email",
              "config": { "slug": "GMAIL_SEND_EMAIL", "on_error": "route",
                "args": { "to": "=item.email", "body": "hello" } } },
            { "id": "recover", "kind": "tool_call", "name": "Recover",
              "config": { "slug": "oh:noop", "args": {} } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "from_port": "error", "to_node": "recover" }
        ]
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
    seed_live_catalog_cache("gmail", vec![seeded_gmail_send_contract()]);

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
async fn dry_run_flags_null_resolved_agent_prompt() {
    // The exact root-cause bug PR A/B/C exist to catch: `prompt` itself is a
    // `=`-expression that reads as prose, not a valid jq program — the
    // vendored engine's own `resolve_traced` records it as a null resolution
    // at `location: "prompt"`, meaning the agent would run with an EMPTY
    // prompt. Unlike other agent-config nulls, this one must fail the dry run.
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "classify", "kind": "agent", "name": "Classify",
              "config": { "prompt": "=You are given an email: .item. Classify the following \
                  email as urgent/normal/low priority." } }
        ],
        "edges": [ { "from_node": "t", "to_node": "classify" } ]
    });

    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["ok"], false,
        "a null-resolved agent prompt must fail the dry run: {parsed}"
    );
    let agent_prompt_nulls = parsed["agent_prompt_nulls"]
        .as_array()
        .expect("agent_prompt_nulls array");
    assert_eq!(agent_prompt_nulls.len(), 1, "{parsed}");
    assert_eq!(agent_prompt_nulls[0]["node_id"], "classify");
    assert_eq!(agent_prompt_nulls[0]["location"], "prompt");
    assert!(
        agent_prompt_nulls[0]["suggestion"]
            .as_str()
            .unwrap()
            .contains("input_context"),
        "{parsed}"
    );
    assert!(
        parsed["message"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("input_context"),
        "{parsed}"
    );
}

#[tokio::test]
async fn dry_run_flags_null_resolved_agent_input_context() {
    // The B7 counterpart to `dry_run_flags_null_resolved_agent_prompt`:
    // `input_context` has been the agent's primary upstream-data channel
    // since #4590, so a null-resolved `input_context` is just as
    // execution-breaking as a null `prompt` — the agent runs with no
    // upstream data at all. Must fail the dry run the same way.
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "classify", "kind": "agent", "name": "Classify",
              "config": { "prompt": "Classify the email as urgent, normal, or low priority.",
                "input_context": "=nodes.missing.item.json.body" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "classify" } ]
    });

    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["ok"], false,
        "a null-resolved agent input_context must fail the dry run: {parsed}"
    );
    let agent_input_context_nulls = parsed["agent_input_context_nulls"]
        .as_array()
        .expect("agent_input_context_nulls array");
    assert_eq!(agent_input_context_nulls.len(), 1, "{parsed}");
    assert_eq!(agent_input_context_nulls[0]["node_id"], "classify");
    assert_eq!(agent_input_context_nulls[0]["location"], "input_context");
    assert!(
        agent_input_context_nulls[0]["suggestion"]
            .as_str()
            .unwrap()
            .contains("upstream"),
        "{parsed}"
    );
}

#[tokio::test]
async fn dry_run_passes_when_agent_uses_input_context_instead_of_prompt_expression() {
    // The FALSE-POSITIVE-PREVENTION case: the same data need, wired the
    // correct way — `input_context` carries the upstream item, `prompt`
    // stays a plain instruction with no leading `=`. This must dry-run green.
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "classify", "kind": "agent", "name": "Classify",
              "config": { "prompt": "Classify the email as urgent, normal, or low priority.",
                "input_context": "=item" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "classify" } ]
    });

    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["ok"], true, "{parsed}");
    assert!(
        parsed["agent_prompt_nulls"].as_array().unwrap().is_empty(),
        "{parsed}"
    );
    assert!(
        parsed["agent_input_context_nulls"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{parsed}"
    );
}

#[tokio::test]
async fn dry_run_warns_on_unexercised_agent_after_condition() {
    // B15's dry-run blind spot: `gate` is a `condition` wired with only a
    // `true` edge to `classify`. The dry run's default trigger input is `{}`
    // (no `input` param passed), so `gate`'s configured field ("active") is
    // absent — falsey — and the condition emits `false`. Since `false` has no
    // outgoing edge, `classify` never executes at all: not a null resolution,
    // not a node error, just silently unexercised. A real trigger's payload
    // could easily carry `active: true` and take the other branch, so the
    // dry run must still surface this as a warning even though `ok` stays
    // `true` — there's nothing here that flips it to a hard reject.
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "gate", "kind": "condition", "name": "Gate",
              "config": { "field": "active" } },
            { "id": "classify", "kind": "agent", "name": "Classify",
              "config": { "prompt": "Classify the item.", "input_context": "=item" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "gate" },
            { "from_node": "gate", "from_port": "true", "to_node": "classify" }
        ]
    });

    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["ok"], true,
        "an unexercised branch is a warning, not a hard reject: {parsed}"
    );
    let warnings = parsed["routing_divergence_warnings"]
        .as_array()
        .expect("routing_divergence_warnings array");
    assert_eq!(warnings.len(), 1, "{parsed}");
    assert_eq!(warnings[0]["node_id"], "classify");
    assert_eq!(warnings[0]["condition_node_id"], "gate");
    assert!(
        warnings[0]["message"]
            .as_str()
            .unwrap()
            .contains("classify"),
        "{parsed}"
    );
}

#[tokio::test]
async fn dry_run_no_routing_divergence_warning_when_every_node_executes() {
    // FALSE-POSITIVE-PREVENTION: a condition whose taken branch under the
    // default mock input DOES reach the downstream agent must not warn.
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "gate", "kind": "condition", "name": "Gate",
              "config": { "field": "active" } },
            { "id": "classify", "kind": "agent", "name": "Classify",
              "config": { "prompt": "Classify the item.", "input_context": "=item" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "gate" },
            { "from_node": "gate", "from_port": "false", "to_node": "classify" }
        ]
    });

    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["ok"], true, "{parsed}");
    assert!(
        parsed["routing_divergence_warnings"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{parsed}"
    );
}

/// (systemic tool-contract fix, Part 2b) A missing required Composio arg is
/// now a HARD REJECT at `revise_workflow` — `validate_tool_contracts` runs
/// ahead of the older advisory `graph_wiring_warnings` check and catches the
/// exact same condition first, so the graph never gets far enough to merely
/// warn about it. `graph_wiring_warnings`'s own required-arg warning (still
/// exercised directly in `ops_tests.rs`) stays as a defense-in-depth
/// fallback for any caller that doesn't also run `validate_tool_contracts`.
#[tokio::test]
async fn revise_workflow_rejects_a_missing_required_composio_arg() {
    seed_live_catalog_cache("gmail", vec![seeded_gmail_send_contract()]);

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

    assert!(
        result.is_error,
        "a missing required arg must now hard-reject"
    );
    let output = result.output();
    assert!(output.contains("send"), "{output}");
    assert!(output.contains("`to`"), "{output}");
    // `body` is wired (expression) — never named as missing.
    assert!(!output.contains("`body`"), "{output}");
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
