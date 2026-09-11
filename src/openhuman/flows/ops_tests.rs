use super::*;
use crate::openhuman::config::Config;
use serde_json::json;
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

fn trigger_only_graph() -> Value {
    json!({
        "name": "trigger-only",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" }
        ],
        "edges": []
    })
}

fn nested_conditional_fan_in_graph() -> Value {
    json!({
        "name": "nested-conditional-fan-in",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Trigger" },
            { "id": "outer", "kind": "condition", "name": "Outer", "config": { "field": "outer" } },
            { "id": "inner", "kind": "condition", "name": "Inner", "config": { "field": "inner" } },
            { "id": "outer_else", "kind": "output_parser", "name": "Outer else" },
            { "id": "inner_else", "kind": "output_parser", "name": "Inner else" },
            { "id": "a", "kind": "output_parser", "name": "A" },
            { "id": "c", "kind": "output_parser", "name": "C" },
            { "id": "m", "kind": "merge", "name": "Merge" }
        ],
        "edges": [
            { "from_node": "start", "from_port": "main", "to_node": "outer" },
            { "from_node": "start", "from_port": "main", "to_node": "c" },
            { "from_node": "outer", "from_port": "true", "to_node": "inner" },
            { "from_node": "outer", "from_port": "false", "to_node": "outer_else" },
            { "from_node": "inner", "from_port": "true", "to_node": "a" },
            { "from_node": "inner", "from_port": "false", "to_node": "inner_else" },
            { "from_node": "a", "from_port": "main", "to_node": "m" },
            { "from_node": "c", "from_port": "main", "to_node": "m" }
        ]
    })
}

fn main_port_conditional_fan_in_graph() -> Value {
    json!({
        "name": "main-port-conditional-fan-in",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Trigger" },
            { "id": "route", "kind": "switch", "name": "Route", "config": { "field": "kind" } },
            { "id": "a", "kind": "output_parser", "name": "A" },
            { "id": "other", "kind": "output_parser", "name": "Other" },
            { "id": "c", "kind": "output_parser", "name": "C" },
            { "id": "m", "kind": "merge", "name": "Merge" }
        ],
        "edges": [
            { "from_node": "start", "from_port": "main", "to_node": "route" },
            { "from_node": "start", "from_port": "main", "to_node": "c" },
            { "from_node": "route", "from_port": "main", "to_node": "a" },
            { "from_node": "route", "from_port": "other", "to_node": "other" },
            { "from_node": "a", "from_port": "main", "to_node": "m" },
            { "from_node": "c", "from_port": "main", "to_node": "m" }
        ]
    })
}

fn referenced_child_graph(workflow_id: &str) -> Value {
    json!({
        "name": "parent-with-saved-child",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Trigger" },
            {
                "id": "saved-child",
                "kind": "sub_workflow",
                "name": "Saved child",
                "config": { "workflow_id": workflow_id }
            }
        ],
        "edges": [
            { "from_node": "start", "from_port": "main", "to_node": "saved-child" }
        ]
    })
}

fn structurally_valid_graph(value: Value) -> WorkflowGraph {
    let graph = migrate_and_deserialize_graph(value).expect("graph should deserialize");
    tinyflows::validate::validate(&graph).expect("fixture should be structurally valid");
    graph
}

fn nested_router_reconvergence_graph(inner_kind: &str, inner_ports: &[&str]) -> WorkflowGraph {
    let mut edges = vec![
        json!({ "from_node": "start", "from_port": "main", "to_node": "outer" }),
        json!({ "from_node": "start", "from_port": "main", "to_node": "c" }),
        json!({ "from_node": "outer", "from_port": "true", "to_node": "inner" }),
        json!({ "from_node": "outer", "from_port": "false", "to_node": "outer_else" }),
    ];
    edges.extend(
        inner_ports
            .iter()
            .map(|port| json!({ "from_node": "inner", "from_port": port, "to_node": "a" })),
    );
    edges.extend([
        json!({ "from_node": "a", "from_port": "main", "to_node": "m" }),
        json!({ "from_node": "c", "from_port": "main", "to_node": "m" }),
    ]);

    structurally_valid_graph(json!({
        "name": "nested-router-reconvergence",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Trigger" },
            { "id": "outer", "kind": "condition", "name": "Outer", "config": { "field": "outer" } },
            { "id": "inner", "kind": inner_kind, "name": "Inner", "config": { "field": "inner" } },
            { "id": "outer_else", "kind": "output_parser", "name": "Outer else" },
            { "id": "a", "kind": "output_parser", "name": "A" },
            { "id": "c", "kind": "output_parser", "name": "C" },
            { "id": "m", "kind": "merge", "name": "Merge" }
        ],
        "edges": edges
    }))
}

/// A graph declaring `repo` (required) and `depth` (defaulted), whose single
/// `transform` node copies both out via `=inputs.<name>`.
fn parameterized_graph() -> Value {
    json!({
        "name": "parameterized",
        "inputs": [
            { "name": "repo", "type": "string", "required": true, "description": "Repo to review" },
            { "name": "depth", "type": "number", "default": 3 }
        ],
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "shape", "kind": "transform", "name": "Shape",
              "config": { "set": { "repo": "=inputs.repo", "depth": "=inputs.depth" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "shape" } ]
    })
}

/// Collects `pairs` into the supplied-values map `flows_run` takes.
fn input_values(pairs: &[(&str, Value)]) -> serde_json::Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

// ── automatic-dispatch binding (issue B2 finding #1, revised by B29) ──────
//
// Live testing found that `flows_create` persisted a freshly-created,
// `enabled = true` schedule flow WITHOUT registering its cron job — only
// `flows_set_enabled` bound it. So a brand-new enabled schedule flow would
// silently never fire until an app restart (boot reconcile) or a manual
// disable→enable toggle.
//
// Issue B29 (save/enable safety) then found the OTHER half of that same bug:
// `flows_create` used to default a schedule flow straight to `enabled: true`
// on create, arming it live before the user ever saw a toggle. Rule 1 now
// creates an automatic-trigger flow DISABLED — so these tests explicitly
// enable via `flows_set_enabled` (the real caller-facing arming path) before
// exercising the cron-binding behavior below, against the real `cron` store
// (not a mock), the same way `bind_schedule_trigger` itself does.

fn schedule_trigger_graph(cron_expr: &str) -> Value {
    json!({
        "name": "scheduled",
        "nodes": [
            {
                "id": "t",
                "kind": "trigger",
                "name": "Trigger",
                "config": { "trigger_kind": "schedule", "schedule": cron_expr }
            }
        ],
        "edges": []
    })
}

// ── flows_resume (issue B2) ───────────────────────────────────────────────

fn approval_gated_graph() -> Value {
    json!({
        "name": "approval-gated",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "gate", "kind": "output_parser", "name": "Gate", "config": { "requires_approval": true } },
            { "id": "downstream", "kind": "output_parser", "name": "Downstream" }
        ],
        "edges": [
            { "from_node": "t", "to_node": "gate" },
            { "from_node": "gate", "to_node": "downstream" }
        ]
    })
}

// ── flows_resume deny semantics (issue G4) ────────────────────────────────

/// A gate with BOTH a `main` edge (to `downstream`) and an `error` edge (to
/// `recover`): denying the gate routes to `recover`, not `downstream`.
fn approval_gated_graph_with_error_port() -> Value {
    json!({
        "name": "approval-gated-error-port",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "gate", "kind": "output_parser", "name": "Gate", "config": { "requires_approval": true } },
            { "id": "downstream", "kind": "output_parser", "name": "Downstream" },
            { "id": "recover", "kind": "output_parser", "name": "Recover" }
        ],
        "edges": [
            { "from_node": "t", "to_node": "gate" },
            { "from_node": "gate", "from_port": "main", "to_node": "downstream" },
            { "from_node": "gate", "from_port": "error", "to_node": "recover" }
        ]
    })
}

// ── Live run observation (issue G2) ───────────────────────────────────────

use crate::openhuman::flows::tinyflows::observability::FlowRunObserver;
use std::sync::Arc as StdArc;
// `RunObserver` must be in scope to call `on_step_finish` on the observer.
use tinyflows::observability::{ExecutionStep, RunObserver as _, StepStatus};

/// trigger -> output_parser passthrough: the parser is a non-trigger node, so
/// the engine fires `on_step_finish` for it, exercising live persistence.
fn passthrough_graph() -> Value {
    json!({
        "name": "passthrough",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "p", "kind": "output_parser", "name": "Parse" }
        ],
        "edges": [ { "from_node": "t", "to_node": "p" } ]
    })
}

// ---------------------------------------------------------------------------
// Unfired-trigger-kind warnings (PHASE 1a validation + PHASE 3c flows_validate)
// ---------------------------------------------------------------------------

fn webhook_trigger_graph() -> Value {
    json!({
        "name": "hooked",
        "nodes": [
            {
                "id": "t",
                "kind": "trigger",
                "name": "Trigger",
                "config": { "trigger_kind": "webhook" }
            }
        ],
        "edges": []
    })
}

// ── flows_list_connections (picker source) ──────────────────────────────

use crate::openhuman::integrations::composio::ComposioConnection;
use crate::openhuman::security::credentials::{
    HttpCredential, HttpCredentialSummary, HttpCredentialsStore,
};

fn composio_conn(id: &str, toolkit: &str, status: &str, email: Option<&str>) -> ComposioConnection {
    ComposioConnection {
        id: id.to_string(),
        toolkit: toolkit.to_string(),
        status: status.to_string(),
        created_at: None,
        account_email: email.map(str::to_string),
        workspace: None,
        username: None,
    }
}

fn http_summary(name: &str, scheme: &str) -> HttpCredentialSummary {
    HttpCredentialSummary {
        name: name.to_string(),
        scheme: scheme.to_string(),
        header_name: None,
        username: None,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

// ── Flow Scout suggestion lifecycle ──────────────────────────────────────────

fn seed_suggestion(config: &Config, id: &str) {
    let s = crate::openhuman::flows::FlowSuggestion {
        id: id.to_string(),
        title: format!("Idea {id}"),
        one_liner: "does a thing".to_string(),
        rationale: "grounded".to_string(),
        trigger_hint: Some("schedule".to_string()),
        steps_outline: vec!["a".to_string()],
        suggested_connections: vec![],
        suggested_slugs: vec![],
        build_prompt: "Build a workflow…".to_string(),
        confidence: 0.5,
        status: crate::openhuman::flows::SuggestionStatus::New,
        created_at: "2026-07-05T00:00:00Z".to_string(),
        source_run_id: None,
    };
    crate::openhuman::flows::store::upsert_suggestions(config, &[s]).unwrap();
}

// ── validate_binding_resolvability ──────────────────────────────────────────

/// Runs a candidate graph `Value` through the exact same migrate/validate
/// path the builder tools use, for a [`WorkflowGraph`] test fixture.
fn graph(value: Value) -> WorkflowGraph {
    validate_and_migrate_graph(value).expect("structurally valid test graph")
}

// ── validate_inference_readiness (provider-connectivity author gate, B45) ──
//
// An `agent` node needs a working LLM inference provider the same way a
// `tool_call` node needs a real Composio connection — but no author-time gate
// previously checked it at all, so a signed-in user with no provider API key
// configured on the managed backend only found out mid-run. These tests never
// touch the network AND never install the process-global
// `test_provider_override` seam (which would race any other test in this
// binary that also installs it): the "construction succeeds" case points the
// role at a local runtime (`ollama:...`), which `resolves_to_managed_backend`
// correctly identifies as non-managed, so `probe_inference_readiness` never
// reaches for the network; the construction-error case is engineered to fail
// purely on a config lookup (`resolve_cloud_slug`'s "no cloud provider
// configured for slug" branch), before any HTTP client is built.

fn seed_app_session_for_gate_test(tmp: &TempDir) {
    use crate::openhuman::security::credentials::{
        AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
    };
    // `verify_session_active` reads from `config.config_path.parent()`, which
    // `test_config` sets to `tmp.path()` itself (distinct from
    // `tmp.path()/workspace`) — seed the session there.
    AuthService::new(tmp.path(), false)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "test.session.jwt",
            std::collections::HashMap::new(),
            true,
        )
        .expect("seed app-session token");
}

// ── validate_tool_contracts (systemic tool-contract fix, Part 2) ───────────
//
// The live-catalog cache is process-global (`LIVE_CATALOG_CACHE`) — every
// test below seeds the exact toolkit it needs via `seed_live_catalog_cache`
// so none of this touches a live Composio backend.

use crate::openhuman::flows::tinyflows::caps::{
    seed_live_catalog_cache, seed_probe_cache, ProbedOutputSample, ToolContract,
};

fn seeded_slack_send_contract() -> ToolContract {
    ToolContract {
        slug: "SLACK_SEND_MESSAGE".to_string(),
        toolkit: "slack".to_string(),
        description: None,
        required_args: vec!["channel".to_string(), "text".to_string()],
        input_schema: None,
        output_fields: vec!["ts".to_string(), "channel".to_string()],
        output_schema: Some(json!({
            "type": "object",
            "properties": { "ts": {"type": "string"}, "channel": {"type": "string"} }
        })),
        primary_array_path: None,
        // `slack` ships a static curated catalog (`catalog_for_toolkit`), so
        // `validate_tool_contracts` now enforces the same curated-only bar
        // `flow_tool_allowed`'s Path A does at runtime (Codex feedback on
        // this PR) — this fixture models a real curated Slack action, not
        // an uncurated one, since these tests exercise the required-arg /
        // hallucinated-slug checks rather than the curation gate itself.
        is_curated: true,
    }
}

// ── validate_connection_refs (WS3) ──────────────────────────────────────────
//
// The transcript bug: the user's connections were twitter →
// `composio:twitter:ca_JX6QU88UfSk4`, gmail → `composio:gmail:ca_vX_WA8FsqNmE`,
// tiktok → `composio:tiktok:ca_LPCp3WQpaDma`. The agent wired
// `composio:twitter:ca_LPCp3WQpaDma` (the TIKTOK id) onto a Twitter node and
// every author-time gate returned ok. These tests exercise the pure matcher so
// no live Composio backend is touched.

/// Build a composio `FlowConnection` fixture (the exact shape
/// `build_flow_connections` produces).
fn ws3_flow_conn(toolkit: &str, id: &str) -> FlowConnection {
    FlowConnection {
        connection_ref: format!("composio:{toolkit}:{id}"),
        kind: "composio".to_string(),
        display: toolkit.to_string(),
        toolkit: Some(toolkit.to_string()),
        scheme: None,
        platform_user_id: None,
    }
}

/// The user's real connected set from the transcript.
fn ws3_transcript_connections() -> Vec<FlowConnection> {
    vec![
        ws3_flow_conn("twitter", "ca_JX6QU88UfSk4"),
        ws3_flow_conn("gmail", "ca_vX_WA8FsqNmE"),
        ws3_flow_conn("tiktok", "ca_LPCp3WQpaDma"),
    ]
}

/// A single tool_call node graph with `slug` + optional `connection_ref`.
fn ws3_tool_call_graph(slug: &str, connection_ref: Option<&str>) -> WorkflowGraph {
    let mut config = json!({ "slug": slug, "args": {} });
    if let Some(cr) = connection_ref {
        config["connection_ref"] = json!(cr);
    }
    graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "act", "kind": "tool_call", "name": "Act", "config": config }
        ],
        "edges": [ { "from_node": "t", "to_node": "act" } ]
    }))
}

fn upload_graph(path: Value) -> WorkflowGraph {
    graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "up", "kind": "tool_call", "name": "Upload",
              "config": { "slug": "oh:storage_upload_file", "args": { "path": path } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "up" } ]
    }))
}

// ── validate_tool_contracts: arg-NAME validation against the input schema
//    (B13 — a misnamed/unsupported field, e.g. `text` instead of
//    `markdown_text` for `SLACK_SEND_MESSAGE`, used to sail through
//    `missing_required_args` because SOME value was present, just under the
//    wrong key) ────────────────────────────────────────────────────────────

/// Models `SLACK_SEND_MESSAGE`'s real `input_schema` (naming `channel` and
/// `markdown_text` — the live bug this fixes: `markdown_text` is the real
/// field, `text` is not) but under a **fictional toolkit key**
/// (`slackargnametest`), never the real `"slack"` key: `seeded_slack_send_contract`
/// above (input_schema: `None`) also seeds `"slack"` and is used by several
/// sibling tests in this file whose `args` still carry `text` — sharing the
/// real key would race those tests over the process-global
/// `LIVE_CATALOG_CACHE` entry for `"slack"` (same discipline
/// `builder_tools_tests.rs` already applies for its own `slack`/`gmail`
/// fixtures that don't match the shared-key contract byte-for-byte).
fn seeded_slack_send_message_contract_with_schema() -> ToolContract {
    ToolContract {
        slug: "SLACKARGNAMETEST_SEND_MESSAGE".to_string(),
        toolkit: "slackargnametest".to_string(),
        description: None,
        required_args: vec![],
        input_schema: Some(json!({
            "type": "object",
            "properties": {
                "channel": { "type": "string" },
                "markdown_text": { "type": "string" }
            }
        })),
        output_fields: vec![],
        output_schema: None,
        primary_array_path: None,
        is_curated: false,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// degrade_completed_status (PR2 — run honesty)
// ─────────────────────────────────────────────────────────────────────────────

fn clean_step(node_id: &str) -> FlowRunStep {
    FlowRunStep {
        node_id: node_id.to_string(),
        output: Value::Null,
        port: None,
        status: Some("success".to_string()),
        duration_ms: Some(1),
        diagnostics: Vec::new(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// B23/B24 — condition node branch label must be on `from_port`, not `to_port`
// ─────────────────────────────────────────────────────────────────────────────

fn condition_graph(
    true_from_port: &str,
    true_to_port: &str,
    false_from_port: &str,
    false_to_port: &str,
) -> Value {
    json!({
        "name": "condition-routing",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "gate", "kind": "condition", "name": "Gate", "config": { "field": "has_important" } },
            { "id": "send_summary", "kind": "output_parser", "name": "Send" },
            { "id": "done", "kind": "output_parser", "name": "Done" }
        ],
        "edges": [
            { "from_node": "t", "from_port": "main", "to_node": "gate", "to_port": "main" },
            { "from_node": "gate", "from_port": true_from_port, "to_node": "send_summary", "to_port": true_to_port },
            { "from_node": "gate", "from_port": false_from_port, "to_node": "done", "to_port": false_to_port }
        ]
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Issue B29 — save/enable safety: `flows_create` gating (Rule 1 + Rule 2)
// ─────────────────────────────────────────────────────────────────────────────
//
// Saving a scheduled/automatic flow used to silently arm it live and
// unattended: `store::create_flow` hardcoded `enabled: true`, and
// `require_approval` defaulted to `false` on most creation paths. These
// tests exercise the two server-side rules `flows_create` now enforces,
// regardless of what the caller passed.

fn app_event_trigger_graph() -> Value {
    json!({
        "name": "app-event",
        "nodes": [
            {
                "id": "t",
                "kind": "trigger",
                "name": "Trigger",
                "config": { "trigger_kind": "app_event", "toolkit": "gmail", "event": "GMAIL_NEW_GMAIL_MESSAGE" }
            }
        ],
        "edges": []
    })
}

fn manual_trigger_graph() -> Value {
    json!({
        "name": "manual",
        "nodes": [
            {
                "id": "t",
                "kind": "trigger",
                "name": "Trigger",
                "config": { "trigger_kind": "manual" }
            }
        ],
        "edges": []
    })
}

fn tool_call_graph() -> Value {
    json!({
        "name": "with-tool-call",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            {
                "id": "post",
                "kind": "tool_call",
                "name": "Post",
                "config": { "slug": "SLACK_SEND_MESSAGE", "args": { "channel": "general" } }
            }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    })
}

fn http_request_graph() -> Value {
    json!({
        "name": "with-http",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            {
                "id": "call",
                "kind": "http_request",
                "name": "Call",
                "config": { "method": "GET", "url": "https://example.com" }
            }
        ],
        "edges": [ { "from_node": "t", "to_node": "call" } ]
    })
}

fn code_graph() -> Value {
    json!({
        "name": "with-code",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            {
                "id": "run",
                "kind": "code",
                "name": "Run",
                "config": { "language": "javascript", "source": "return {};" }
            }
        ],
        "edges": [ { "from_node": "t", "to_node": "run" } ]
    })
}

fn readonly_graph() -> Value {
    json!({
        "name": "readonly",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "a", "kind": "agent", "name": "Summarize", "config": { "prompt": "hi" } },
            { "id": "x", "kind": "transform", "name": "Reshape", "config": { "expression": "=item" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "a" },
            { "from_node": "a", "to_node": "x" }
        ]
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Builder convergence fix — trail-off backstop (`flows_build`'s terminal-state
// guarantee: every turn ends in a proposal or a real question, never silence).
// ─────────────────────────────────────────────────────────────────────────────

fn builder_tool_call(
    id: &str,
    name: &str,
) -> crate::openhuman::agent::messages::ConversationMessage {
    use crate::openhuman::agent::messages::ConversationMessage;
    use crate::openhuman::inference::provider::ToolCall;
    ConversationMessage::AssistantToolCalls {
        text: None,
        tool_calls: vec![ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: "{}".to_string(),
            extra_content: None,
        }],
        reasoning_content: None,
        extra_metadata: None,
    }
}

fn builder_tool_result(
    call_id: &str,
    content: &str,
) -> crate::openhuman::agent::messages::ConversationMessage {
    use crate::openhuman::agent::messages::{ConversationMessage, ToolResultMessage};
    ConversationMessage::ToolResults(vec![ToolResultMessage {
        tool_call_id: call_id.to_string(),
        content: content.to_string(),
    }])
}

// ── Live-run reliability: drop-guard + boot sweep + detach (bugs B41/B42) ───

/// Seeds a real flow plus an already-inserted `running` `flow_runs` row, and
/// returns `(config, flow_id, run_id)`. The `TempDir` is returned so the caller
/// keeps the on-disk store alive for the duration of the test.
fn seed_running_run(tmp: &TempDir) -> (Config, String, String) {
    let config = test_config(tmp);
    let flow = store::create_flow(
        &config,
        "reliability".to_string(),
        structurally_valid_graph(trigger_only_graph()),
        false,
        true,
    )
    .unwrap();
    let run_id = format!("flow:{}:{}", flow.id, uuid::Uuid::new_v4());
    // Stamped well before `PROCESS_RUN_FLOOR` so this row models what the boot
    // sweep actually targets: a `running` row left behind by a *prior* process.
    // Using `Utc::now()` here would make the sweep tests order-dependent — the
    // floor is a process-wide `LazyLock`, so a sibling test that ran a real
    // flow first would push it past a "now" seed and the row would (correctly)
    // fall out of the candidate set.
    store::insert_flow_run(
        &config,
        &run_id,
        &flow.id,
        &run_id,
        PRIOR_PROCESS_STARTED_AT,
    )
    .unwrap();
    (config, flow.id, run_id)
}

/// A `started_at` that provably predates this process's `PROCESS_RUN_FLOOR`.
const PRIOR_PROCESS_STARTED_AT: &str = "2020-01-01T00:00:00+00:00";

#[path = "ops_support_tests.rs"]
mod support_tests;
use support_tests::*;

#[path = "ops_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "ops_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "ops_tests_part_03_tests.rs"]
mod part_03_tests;
#[path = "ops_tests_part_04_tests.rs"]
mod part_04_tests;
#[path = "ops_tests_part_05_tests.rs"]
mod part_05_tests;
#[path = "ops_tests_part_06_tests.rs"]
mod part_06_tests;
#[path = "ops_tests_part_07_tests.rs"]
mod part_07_tests;
#[path = "ops_tests_part_08_tests.rs"]
mod part_08_tests;
#[path = "ops_tests_part_09_tests.rs"]
mod part_09_tests;
#[path = "ops_tests_part_10_tests.rs"]
mod part_10_tests;
#[path = "ops_tests_part_11_tests.rs"]
mod part_11_tests;
#[path = "ops_tests_part_12_tests.rs"]
mod part_12_tests;
#[path = "ops_tests_part_13_tests.rs"]
mod part_13_tests;
