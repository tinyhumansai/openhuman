use super::*;
use crate::openhuman::config::Config;
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

fn valid_graph() -> Value {
    json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Summarize", "config": { "prompt": "hi" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    })
}

// ── search_tool_catalog / get_tool_contract ─────────────────────────────────
// The live-catalog cache is process-global (`LIVE_CATALOG_CACHE`) — every
// test below seeds the exact toolkit(s)/contract(s) it needs via
// `seed_live_catalog_cache` so none of this touches a live Composio backend,
// and keeps each toolkit's seeded contents self-consistent across tests that
// share a toolkit key (same discipline the pre-fix required-args/response-
// fields caches already required).

use crate::openhuman::flows::tinyflows::caps::{
    seed_live_catalog_cache, seed_probe_cache, ProbedOutputSample, ToolContract,
};

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

/// A minimal seeded contract with NO required args, for WS6 dry-run tests: seeds
/// a bespoke toolkit so the required-arg preflight always passes and the sandbox
/// run settles into the `null_resolutions` path (rather than aborting), letting
/// the test assert the honest Composio-upstream diagnostic deterministically —
/// independent of whatever gmail/slack contracts other tests seed into the
/// process-global cache.
fn seeded_ws6_contract(slug: &str, toolkit: &str) -> ToolContract {
    ToolContract {
        slug: slug.to_string(),
        toolkit: toolkit.to_string(),
        description: Some("ws6 test action".to_string()),
        required_args: vec![],
        input_schema: Some(json!({ "type": "object", "additionalProperties": true })),
        output_fields: vec![],
        output_schema: None,
        primary_array_path: None,
        is_curated: true,
    }
}

// ── WS3: early runtime-gate warnings on uncurated actions ────────────────────
//
// Transcript failure #2: `get_tool_contract { slug: "TWITTER_USER_LOOKUP_ME" }`
// returned `is_curated: false` with no other signal; the agent built and wired
// the node and only ~15 tool calls later did `validate_workflow` reject it. A
// real-but-uncurated action of a toolkit that ships a curated catalog is a hard
// curated-only allowlist at RUNTIME, so surface the blocker at contract-fetch /
// search time. Uses `spotify` / `telegram` (real curated toolkits unused by
// other tests) so these seeds can't race with the shared `gmail`/`slack` keys.

fn spotify_curated_action() -> ToolContract {
    ToolContract {
        slug: "SPOTIFY_START_PLAYBACK".to_string(),
        toolkit: "spotify".to_string(),
        description: Some("Start playback".to_string()),
        required_args: vec![],
        input_schema: Some(json!({ "type": "object" })),
        output_fields: vec![],
        output_schema: None,
        primary_array_path: None,
        is_curated: true,
    }
}

// ── WS5: per-token fallback ranking for zero-result multi-word queries ───────
//
// Transcript failure: `search_tool_catalog` behaved like near-exact matching —
// multi-word natural-language queries ("twitter tweet replies lookup") returned
// `count: 0` even though the toolkit HAS matching actions, so the agent falsely
// concluded the action didn't exist. The primary pass is a strict case-
// insensitive AND (every token must match); when that misses for a multi-word
// query, a per-keyword OR fallback now returns the nearest matches + a note.

fn twt_lookup() -> ToolContract {
    ToolContract {
        slug: "TWTFALLBACKTEST_TWEET_LOOKUP".to_string(),
        toolkit: "twtfallbacktest".to_string(),
        description: Some("Look up a tweet".to_string()),
        required_args: vec!["id".to_string()],
        input_schema: None,
        output_fields: vec!["text".to_string()],
        output_schema: None,
        primary_array_path: None,
        is_curated: true,
    }
}

fn twt_replies() -> ToolContract {
    ToolContract {
        slug: "TWTFALLBACKTEST_LIST_REPLIES".to_string(),
        toolkit: "twtfallbacktest".to_string(),
        description: Some("List replies to a tweet".to_string()),
        required_args: vec!["tweet_id".to_string()],
        input_schema: None,
        output_fields: vec!["replies".to_string()],
        output_schema: None,
        primary_array_path: None,
        is_curated: true,
    }
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

/// A single-node graph with an automatic (schedule) trigger — enough to
/// exercise the manual→automatic transition without tripping any of
/// `run_builder_gates`' binding/connection/contract checks (no other nodes,
/// nothing to bind).
fn schedule_trigger_graph() -> Value {
    json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger",
              "config": { "trigger_kind": "schedule", "schedule": "0 8 * * *" } }
        ],
        "edges": []
    })
}

// ── save_workflow: enforcing binding-resolvability gate ─────────────────────

/// The proven live-failure shape (same as
/// `tools_tests::propose_workflow_rejects_agent_binding_missing_declared_field`):
/// a `summarize` agent whose declared output schema omits `channel`, and a
/// `notify` tool_call binding `args.channel` to that unaddressable output.
/// A schema-less agent is deliberately accepted by TinyFlows: its host-defined
/// output may contain structured JSON, so the field is unverifiable rather
/// than certainly absent.
fn unresolvable_binding_graph() -> Value {
    json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "agent_ref": "researcher", "prompt": "summarize",
                "output_parser": { "schema": { "type": "object",
                  "properties": { "summary": { "type": "string" } } } } } },
            { "id": "notify", "kind": "tool_call", "name": "Notify",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "=nodes.summarize.item.json.channel", "text": "A notification" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "notify" }
        ]
    })
}

// ── cancel_flow_run ownership check (T-M3) ────────────────────────────────

/// A graph that pauses at a `pending_approval` gate, so the run it produces
/// stays non-terminal (cancellable) — mirrors `ops_tests::approval_gated_graph`.
fn cancel_test_approval_gated_graph() -> Value {
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

#[path = "builder_tools_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "builder_tools_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "builder_tools_tests_part_03_tests.rs"]
mod part_03_tests;
#[path = "builder_tools_tests_part_04_tests.rs"]
mod part_04_tests;
#[path = "builder_tools_tests_part_05_tests.rs"]
mod part_05_tests;
