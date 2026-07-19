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

#[test]
fn list_flow_connections_json_surfaces_platform_user_id() {
    use crate::openhuman::flows::types::FlowConnection;

    let with_identity = FlowConnection {
        connection_ref: "composio:slack:ca_slack1".to_string(),
        kind: "composio".to_string(),
        display: "Slack".to_string(),
        toolkit: Some("slack".to_string()),
        scheme: None,
        platform_user_id: Some("U123ABC".to_string()),
    };
    let json = flow_connection_to_json(&with_identity);
    assert_eq!(json["platform_user_id"], "U123ABC");

    let without_identity = FlowConnection {
        platform_user_id: None,
        ..with_identity
    };
    let json = flow_connection_to_json(&without_identity);
    assert!(json["platform_user_id"].is_null());
}

// ── search_tool_catalog / get_tool_contract ─────────────────────────────────
// The live-catalog cache is process-global (`LIVE_CATALOG_CACHE`) — every
// test below seeds the exact toolkit(s)/contract(s) it needs via
// `seed_live_catalog_cache` so none of this touches a live Composio backend,
// and keeps each toolkit's seeded contents self-consistent across tests that
// share a toolkit key (same discipline the pre-fix required-args/response-
// fields caches already required).

use crate::openhuman::tinyflows::caps::{
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

#[tokio::test]
async fn get_tool_contract_warns_on_an_uncurated_action_of_a_curated_toolkit() {
    let uncurated = ToolContract {
        slug: "SPOTIFY_OBSCURE_ACTION".to_string(),
        is_curated: false,
        ..spotify_curated_action()
    };
    seed_live_catalog_cache("spotify", vec![spotify_curated_action(), uncurated]);
    let tmp = TempDir::new().unwrap();
    let tool = GetToolContractTool::new(test_config(&tmp));

    // Uncurated action → runtime_gate present, FIRST in the payload, contract intact.
    let result = tool
        .execute(json!({ "slug": "SPOTIFY_OBSCURE_ACTION" }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let out = result.output();
    assert!(out.contains("runtime_gate"), "{out}");
    assert!(out.contains("REJECTED on every real run"), "{out}");
    let gate_pos = out.find("runtime_gate").expect("runtime_gate key");
    let slug_pos = out.find("\"slug\"").expect("slug key");
    assert!(
        gate_pos < slug_pos,
        "runtime_gate must serialize first (agents read top-down): {out}"
    );
    let parsed: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(parsed["slug"], "SPOTIFY_OBSCURE_ACTION");
    assert_eq!(parsed["is_curated"], false);

    // Curated action of the same toolkit → NO runtime_gate.
    let result = tool
        .execute(json!({ "slug": "SPOTIFY_START_PLAYBACK" }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    assert!(
        !result.output().contains("runtime_gate"),
        "{}",
        result.output()
    );
}

#[tokio::test]
async fn search_tool_catalog_flags_runtime_gated_uncurated_rows() {
    let curated = ToolContract {
        slug: "TELEGRAM_SEND_MESSAGE".to_string(),
        toolkit: "telegram".to_string(),
        description: Some("Send a message".to_string()),
        required_args: vec![],
        input_schema: None,
        output_fields: vec![],
        output_schema: None,
        primary_array_path: None,
        is_curated: true,
    };
    let uncurated = ToolContract {
        slug: "TELEGRAM_OBSCURE_SEND".to_string(),
        is_curated: false,
        ..curated.clone()
    };
    seed_live_catalog_cache("telegram", vec![curated, uncurated]);

    let config = Config::default();
    let results = search_live_catalog(&config, "send", Some("telegram"), 40).await;
    assert_eq!(results.len(), 2, "{results:?}");
    // Curated row: no `runtime_gated` key (only present when true).
    let curated_row = results.iter().find(|r| r["featured"] == true).unwrap();
    assert!(curated_row.get("runtime_gated").is_none(), "{curated_row}");
    // Uncurated row of a curated toolkit: `runtime_gated: true`.
    let uncurated_row = results.iter().find(|r| r["featured"] == false).unwrap();
    assert_eq!(uncurated_row["runtime_gated"], true);
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

#[tokio::test]
async fn search_catalog_multiword_miss_falls_back_to_per_keyword() {
    seed_live_catalog_cache("twtfallbacktest", vec![twt_lookup(), twt_replies()]);
    let config = Config::default();
    // Strict AND misses ("twitter"/"timeline" match nothing) but individual
    // tokens ("tweet", "replies", "lookup") hit — so the fallback fires.
    let outcome = search_catalog(
        &config,
        "twitter tweet replies lookup timeline",
        Some("twtfallbacktest"),
        40,
    )
    .await;
    assert!(
        outcome.fallback,
        "multi-word AND-miss must run the fallback"
    );
    assert_eq!(outcome.results.len(), 2, "{:?}", outcome.results);
    let note = outcome.note.expect("fallback carries an advisory note");
    assert!(
        note.contains("nearest per-keyword"),
        "note should explain the near-miss + single-keyword retry: {note}"
    );
    // Fallback rows carry the SAME shape as primary rows.
    for r in &outcome.results {
        assert_eq!(r["toolkit"], "twtfallbacktest");
        assert_eq!(r["featured"], true);
        assert!(r["required_args"].is_array());
    }
}

#[tokio::test]
async fn search_tool_catalog_tool_surfaces_fallback_note_with_nonzero_count() {
    seed_live_catalog_cache("twtfallbacktest", vec![twt_lookup(), twt_replies()]);
    let tmp = TempDir::new().unwrap();
    let tool = SearchToolCatalogTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({
            "query": "twitter tweet replies lookup timeline",
            "toolkit": "twtfallbacktest"
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    // `count` reflects the returned rows (non-zero) so an agent never reads a
    // fallback as "no such action".
    assert_eq!(parsed["count"], 2);
    assert!(parsed["results"].as_array().unwrap().len() == 2);
    assert!(parsed["note"].as_str().unwrap().contains("No exact match"));
}

#[tokio::test]
async fn search_catalog_single_word_behavior_unchanged() {
    seed_live_catalog_cache("onewordtest", vec![twt_lookup()]);
    let config = Config::default();
    // A hit: single-word query returns the primary match, no fallback, no note.
    let hit = search_catalog(&config, "tweet", Some("onewordtest"), 40).await;
    assert!(!hit.fallback);
    assert!(hit.note.is_none());
    assert_eq!(hit.results.len(), 1);
    // A miss: single-word query stays empty and does NOT run the fallback.
    let miss = search_catalog(&config, "zzznomatchzzz", Some("onewordtest"), 40).await;
    assert!(
        !miss.fallback,
        "single-token miss must not trigger fallback"
    );
    assert!(miss.results.is_empty());
}

#[tokio::test]
async fn search_catalog_multiword_zero_token_match_returns_note() {
    seed_live_catalog_cache("zerotoktest", vec![twt_lookup()]);
    let config = Config::default();
    // Multi-word query where NO token matches anything: still a note (not a bare
    // count: 0), but zero rows.
    let outcome = search_catalog(&config, "qqq www eeeeee", Some("zerotoktest"), 40).await;
    assert!(outcome.fallback, "multi-word miss ran the fallback pass");
    assert!(outcome.results.is_empty());
    let note = outcome
        .note
        .expect("zero-token multi-word miss still gets a note");
    assert!(
        note.contains("keyword-based"),
        "note should explain the keyword-based search: {note}"
    );
}

#[tokio::test]
async fn search_catalog_fallback_rows_flag_runtime_gated() {
    // Reuse the exact telegram seed of the runtime_gated primary test so a
    // concurrent run over the shared cache stays self-consistent; telegram is a
    // real curated toolkit, so its uncurated action is `runtime_gated`.
    let curated = ToolContract {
        slug: "TELEGRAM_SEND_MESSAGE".to_string(),
        toolkit: "telegram".to_string(),
        description: Some("Send a message".to_string()),
        required_args: vec![],
        input_schema: None,
        output_fields: vec![],
        output_schema: None,
        primary_array_path: None,
        is_curated: true,
    };
    let uncurated = ToolContract {
        slug: "TELEGRAM_OBSCURE_SEND".to_string(),
        is_curated: false,
        ..curated.clone()
    };
    seed_live_catalog_cache("telegram", vec![curated, uncurated]);

    let config = Config::default();
    // "obscure" hits only the uncurated slug; "lookup"/"replies" hit nothing;
    // "telegram" matches the toolkit of both — so strict AND misses and the
    // fallback ranks the OBSCURE row first (2 hits) over SEND_MESSAGE (1 hit).
    let outcome = search_catalog(
        &config,
        "telegram obscure lookup replies",
        Some("telegram"),
        40,
    )
    .await;
    assert!(outcome.fallback);
    assert_eq!(outcome.results.len(), 2, "{:?}", outcome.results);
    let gated = outcome
        .results
        .iter()
        .find(|r| r["featured"] == false)
        .expect("uncurated row present");
    assert_eq!(gated["runtime_gated"], true);
    let curated_row = outcome
        .results
        .iter()
        .find(|r| r["featured"] == true)
        .expect("curated row present");
    assert!(curated_row.get("runtime_gated").is_none());
}

/// B12: a cached real-output probe overrides `get_tool_contract`'s
/// schema-derived `primary_array_path`/`output_fields` — most relevant for a
/// slug whose live listing (like every GitHub action, verified live) has NO
/// output schema at all, so the schema-derived fields would otherwise be
/// permanently empty/null.
#[tokio::test]
async fn get_tool_contract_applies_a_cached_probe_override() {
    let contract = ToolContract {
        slug: "PROBEOVERRIDETEST_LIST_REPOSITORY_ISSUES".to_string(),
        toolkit: "probeoverridetest".to_string(),
        description: None,
        required_args: vec!["owner".to_string(), "repo".to_string()],
        input_schema: None,
        output_fields: vec![],
        output_schema: None,
        primary_array_path: None,
        is_curated: true,
    };
    seed_live_catalog_cache("probeoverridetest", vec![contract]);
    seed_probe_cache(
        "PROBEOVERRIDETEST_LIST_REPOSITORY_ISSUES",
        ProbedOutputSample {
            primary_array_path: Some("data.issues".to_string()),
            output_fields: vec!["issues".to_string(), "total_count".to_string()],
            sample: json!({ "data": { "issues": [], "total_count": 0 } }),
        },
    );
    let tmp = TempDir::new().unwrap();
    let tool = GetToolContractTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({ "slug": "PROBEOVERRIDETEST_LIST_REPOSITORY_ISSUES" }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["primary_array_path"], "data.issues");
    assert_eq!(parsed["output_fields"], json!(["issues", "total_count"]));
    // The schema-derived field stays null — the probe overrides the HINT
    // fields, it doesn't fabricate a schema that was never published.
    assert!(parsed["output_schema"].is_null());
}

// ── get_tool_output_sample (B12: the real-output probe) ─────────────────────

#[test]
fn get_tool_output_sample_is_read_only_permission_with_no_external_effect() {
    let tmp = TempDir::new().unwrap();
    let tool = GetToolOutputSampleTool::new(test_config(&tmp));
    assert_eq!(tool.name(), "get_tool_output_sample");
    assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
    assert!(!tool.external_effect());
}

#[tokio::test]
async fn get_tool_output_sample_missing_slug_is_error() {
    let tmp = TempDir::new().unwrap();
    let tool = GetToolOutputSampleTool::new(test_config(&tmp));
    let result = tool.execute(json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Missing 'slug'"));
}

/// The scope gate runs BEFORE any client/network call, so a Write-scope
/// action is refused entirely offline — this must never depend on a live
/// Composio backend to prove the probe can't perform a real mutation.
#[tokio::test]
async fn get_tool_output_sample_refuses_a_write_scope_action() {
    let tmp = TempDir::new().unwrap();
    let tool = GetToolOutputSampleTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({ "slug": "GMAIL_SEND_EMAIL" }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("READ-only"), "{}", result.output());
}

/// The connected-toolkit gate runs before the real call too — in a test
/// environment with no backend session, `fetch_connected_integrations`
/// degrades to empty (best-effort, per its own doc), so a Read-scope action
/// against an unconnected toolkit is refused without ever reaching a client.
#[tokio::test]
async fn get_tool_output_sample_refuses_an_unconnected_toolkit() {
    let tmp = TempDir::new().unwrap();
    let tool = GetToolOutputSampleTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({ "slug": "GITHUB_LIST_REPOSITORY_ISSUES" }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.output().contains("not connected") || result.output().contains("no active"),
        "{}",
        result.output()
    );
}

// ── dry_run_workflow ─────────────────────────────────────────────────────────

#[test]
fn dry_run_is_side_effect_free_and_ungated() {
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    assert_eq!(tool.name(), "dry_run_workflow");
    // Mock-only + side-effect-free → PermissionLevel::None, available on every
    // tier including read-only (audit F7).
    assert_eq!(tool.permission_level(), PermissionLevel::None);
    assert!(!tool.external_effect());
}

#[tokio::test]
async fn dry_run_allowed_under_readonly_tier() {
    // F7: dry_run is mock-only and side-effect-free, so a read-only agent must
    // be able to self-verify its own proposal (previously refused).
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::ReadOnly),
        test_config(&TempDir::new().unwrap()),
    );
    assert_eq!(tool.permission_level(), PermissionLevel::None);
    let result = tool
        .execute(json!({ "graph": valid_graph() }))
        .await
        .unwrap();
    // Not refused for tier reasons — it actually runs against the mocks.
    assert!(!result.is_error, "{}", result.output());
    assert!(!result.output().to_lowercase().contains("read-only"));
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
async fn dry_run_plain_agent_with_output_parser_schema_is_green() {
    // Regression for the transcript false-failure: a builder-generated `agent`
    // node carries NO `agent_ref`, so the vendored engine routes it to the
    // `llm` slot (not the `AgentRunner`). Before `SchemaAwareMockLlm` the plain
    // `MockLlm` echo (`{ completion, connection }`) failed the node's
    // `output_parser.schema` sub-port with `output_parser: value failed schema
    // validation after auto-fix: missing required property ...`, sinking a
    // correctly-built graph. Now the mock LLM synthesizes a schema-valid object,
    // and a downstream node binds the typed placeholders (non-null).
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Schedule",
              "config": { "trigger_kind": "schedule" } },
            { "id": "a", "kind": "agent", "name": "Extract",
              "config": { "prompt": "extract the fields",
                "output_parser": { "schema": { "type": "object",
                    "required": ["subject", "priority", "recipients"],
                    "properties": {
                        "subject": { "type": "string" },
                        "priority": { "type": "integer" },
                        "recipients": { "type": "array" }
                    } } } } },
            // Downstream node binds the schema'd agent fields: proves the
            // placeholders are addressable and resolve to typed (non-null)
            // values, not the vendored echo's opaque `{ completion, ... }`.
            { "id": "down", "kind": "transform", "name": "Route",
              "config": { "set": {
                  "subject": "=nodes.a.item.json.subject",
                  "priority": "=nodes.a.item.json.priority",
                  "recipients": "=nodes.a.item.json.recipients" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "a" },
            { "from_node": "a", "to_node": "down" }
        ]
    });
    let result = tool
        .execute(json!({ "graph": graph, "input": { "topic": "launch" } }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let out = result.output();
    assert!(
        !out.to_lowercase().contains("schema validation"),
        "plain agent with a valid schema must not hit the output_parser failure: {out}"
    );
    let parsed: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(parsed["sandbox"], true);
    assert_eq!(
        parsed["ok"], true,
        "plain-agent-with-schema dry-run must be green: {parsed}"
    );
    // The agent envelope's `json` carries the schema-synthesized placeholders.
    // (In the run OUTPUT each Item serializes as `{ json: <value> }`, and the
    // agent's value is the `{json,text,raw}` envelope — hence the double hop.)
    let agent_json = &parsed["output"]["nodes"]["a"]["items"][0]["json"]["json"];
    assert_eq!(agent_json["subject"], "", "{parsed}");
    assert_eq!(agent_json["priority"], 0, "{parsed}");
    assert_eq!(agent_json["recipients"], json!([]), "{parsed}");
    // The downstream node's bindings resolved to those typed placeholders —
    // none of them null.
    let down_json = &parsed["output"]["nodes"]["down"]["items"][0]["json"];
    assert!(!down_json["subject"].is_null(), "{parsed}");
    assert_eq!(down_json["priority"], 0, "{parsed}");
    assert_eq!(down_json["recipients"], json!([]), "{parsed}");
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
async fn dry_run_flags_composio_upstream_binding_as_unverifiable_not_a_wiring_bug() {
    // WS6: `post`'s `body` binds to the OUTPUT of an upstream Composio
    // `tool_call` (`get_me`). The echo sandbox renders `get_me` as
    // `{tool, args, connection}` and can NEVER produce `.item.json.data.username`,
    // so the binding resolves `null` here even when it's wired correctly. The
    // dry run still fails (`ok: false` — a null could hide a typo), but the
    // diagnostic must be HONEST: mark it `unverifiable` and point at
    // get_tool_contract / get_tool_output_sample rather than telling the agent
    // its (possibly-correct) wiring is broken — the exact false negative that
    // sent the transcript agent re-wiring an already-correct binding 3 times.
    // Seed bespoke toolkits (no other test touches `ws6up`/`ws6dl`) with NO
    // required args, so the required-arg preflight passes and the run settles
    // into the `null_resolutions` path deterministically — independent of the
    // process-global catalog cache other tests seed for gmail/slack/etc.
    seed_live_catalog_cache("ws6up", vec![seeded_ws6_contract("WS6UP_LOOKUP", "ws6up")]);
    seed_live_catalog_cache("ws6dl", vec![seeded_ws6_contract("WS6DL_SEND", "ws6dl")]);
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "get_me", "kind": "tool_call", "name": "Who am I",
              "config": { "slug": "WS6UP_LOOKUP", "args": {} } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "WS6DL_SEND",
                "args": { "recipient_email": "a@b.com", "subject": "hi",
                  "body": "=nodes.get_me.item.json.data.username" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "get_me" },
            { "from_node": "get_me", "to_node": "post" }
        ]
    });
    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["ok"], false, "{parsed}");
    let null_resolutions = parsed["null_resolutions"]
        .as_array()
        .expect("null_resolutions array");
    let entry = null_resolutions
        .iter()
        .find(|e| e["node_id"] == "post" && e["location"] == "args.body")
        .unwrap_or_else(|| panic!("expected a post.body null resolution: {parsed}"));
    assert_eq!(entry["unverifiable"], true, "{parsed}");
    assert_eq!(entry["upstream_tool_call"], "get_me", "{parsed}");
    let suggestion = entry["suggestion"].as_str().expect("suggestion string");
    assert!(suggestion.contains("UNVERIFIABLE"), "{suggestion}");
    assert!(suggestion.contains("get_tool_contract"), "{suggestion}");
    assert!(
        suggestion.contains("get_tool_output_sample"),
        "{suggestion}"
    );
}

#[tokio::test]
async fn dry_run_keeps_generic_null_text_for_a_non_tool_call_upstream_binding() {
    // WS6 contrast: `post`'s arg binds to a `transform` node's output (whose
    // real output the echo sandbox DOES produce), and the transform never sets
    // the referenced field, so the null IS a genuine wiring bug. This entry must
    // stay the plain `{ node_id, location, expression }` shape — no
    // `unverifiable` flag — so the honest-uncertainty treatment doesn't leak
    // onto real mistakes.
    seed_live_catalog_cache("ws6dl", vec![seeded_ws6_contract("WS6DL_SEND", "ws6dl")]);
    let tool = DryRunWorkflowTool::new(
        policy(AutonomyLevel::Supervised),
        test_config(&TempDir::new().unwrap()),
    );
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "build", "kind": "transform", "name": "Build",
              "config": { "set": { "unrelated": "x" } } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "WS6DL_SEND",
                "args": { "recipient_email": "a@b.com", "subject": "hi",
                  "body": "=nodes.build.item.json.missing" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "build" },
            { "from_node": "build", "to_node": "post" }
        ]
    });
    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["ok"], false, "{parsed}");
    let entry = parsed["null_resolutions"]
        .as_array()
        .expect("null_resolutions array")
        .iter()
        .find(|e| e["node_id"] == "post" && e["location"] == "args.body")
        .unwrap_or_else(|| panic!("expected a post.body null resolution: {parsed}"));
    assert!(
        entry.get("unverifiable").is_none(),
        "a non-tool_call upstream must keep the generic diagnostic: {parsed}"
    );
    assert!(
        entry.get("suggestion").is_none(),
        "generic entry carries no unverifiable suggestion: {parsed}"
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

#[tokio::test]
async fn save_workflow_surfaces_auto_disarm_warning_on_manual_to_automatic_transition() {
    // Regression for #4889 + the stale-docs issue that motivated this test:
    // `flows_update` auto-disables a flow whenever its trigger transitions
    // from manual to automatic on an already-enabled flow, but `save_workflow`
    // used to drop `flows_update`'s explanatory `RpcOutcome.logs` entirely —
    // the agent had no way to relay the disarm to the user. Assert both the
    // disarm itself and that its log now surfaces in `save_workflow`'s
    // `warnings`.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow_id = seed_flow(&config, "Manual flow").await;
    let seeded = ops::flows_get(&config, &flow_id).await.unwrap().value;
    assert!(
        seeded.enabled,
        "precondition: a manual-trigger flow persists enabled from create"
    );

    let tool = SaveWorkflowTool::new(config.clone());
    let result = tool
        .execute(json!({
            "flow_id": flow_id,
            "graph": schedule_trigger_graph(),
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());

    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["enabled"], false,
        "manual→automatic transition on an enabled flow must auto-disable it: {parsed}"
    );
    let warnings = parsed["warnings"]
        .as_array()
        .expect("warnings must be an array");
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap_or("").contains("auto-disabled")),
        "save_workflow must surface flows_update's disarm log as a warning, got: {parsed}"
    );
    let flow_updated_boilerplate = format!("flow updated: {flow_id}");
    assert!(
        warnings
            .iter()
            .all(|w| w.as_str().unwrap_or("") != flow_updated_boilerplate),
        "save_workflow must exclude the redundant \"flow updated: <id>\" boilerplate \
         from warnings, got: {parsed}"
    );

    // Persisted, not just returned in-memory.
    let reloaded = ops::flows_get(&config, &flow_id).await.unwrap().value;
    assert!(!reloaded.enabled);
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

#[tokio::test]
async fn list_node_kinds_tool_returns_all_twelve() {
    let tool = ListNodeKindsTool::new();
    let result = tool.execute(json!({})).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    let kinds = parsed["node_kinds"].as_array().unwrap();
    assert_eq!(kinds.len(), 12);
    // Each entry carries a kind + summary + the config-field name lists.
    assert!(kinds.iter().any(|k| k["kind"] == "tool_call"));
    assert!(kinds.iter().all(|k| k.get("summary").is_some()));
}

#[tokio::test]
async fn get_node_kind_contract_tool_returns_contract_and_rejects_unknown() {
    let tool = GetNodeKindContractTool::new();

    let ok = tool.execute(json!({ "kind": "tool_call" })).await.unwrap();
    assert!(!ok.is_error, "{}", ok.output());
    let parsed: Value = serde_json::from_str(&ok.output()).unwrap();
    assert_eq!(parsed["kind"], "tool_call");
    assert!(parsed["config_fields"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["name"] == "slug"));
    // Host overlay is present on the tool's output.
    assert!(parsed["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n.as_str().unwrap_or("").contains("Composio")));

    let bad = tool.execute(json!({ "kind": "nope" })).await.unwrap();
    assert!(bad.is_error);
    assert!(bad.output().contains("list_node_kinds"));

    let missing = tool.execute(json!({})).await.unwrap();
    assert!(missing.is_error);
}

// ── edit_workflow (F1: structured incremental edits) ─────────────────────────

#[tokio::test]
async fn edit_workflow_applies_ops_to_inline_graph_and_returns_proposal() {
    let tmp = TempDir::new().unwrap();
    let tool = EditWorkflowTool::new(test_config(&tmp));

    // Add a merge node `b` and wire the agent into it.
    let result = tool
        .execute(json!({
            "graph": valid_graph(),
            "name": "Edited flow",
            "instruction": "add a merge step",
            "ops": [
                { "op": "add_node", "node": { "id": "b", "kind": "merge", "name": "Join" } },
                { "op": "add_edge", "edge": { "from_node": "a", "to_node": "b" } }
            ]
        }))
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["type"], "workflow_proposal");
    assert_eq!(parsed["name"], "Edited flow");
    assert_eq!(parsed["graph"]["nodes"].as_array().unwrap().len(), 3);
    assert_eq!(parsed["graph"]["edges"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn edit_workflow_update_node_config_merge_patches() {
    let tmp = TempDir::new().unwrap();
    let tool = EditWorkflowTool::new(test_config(&tmp));

    let result = tool
        .execute(json!({
            "graph": valid_graph(),
            "ops": [
                { "op": "update_node_config", "id": "a", "config": { "prompt": "new instruction" } }
            ]
        }))
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    let nodes = parsed["graph"]["nodes"].as_array().unwrap();
    let agent = nodes.iter().find(|n| n["id"] == "a").unwrap();
    assert_eq!(agent["config"]["prompt"], "new instruction");
}

#[tokio::test]
async fn edit_workflow_requires_a_base() {
    let tmp = TempDir::new().unwrap();
    let tool = EditWorkflowTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({ "ops": [ { "op": "remove_node", "id": "a" } ] }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("flow_id"));
}

#[tokio::test]
async fn edit_workflow_reports_failing_op_with_guidance() {
    let tmp = TempDir::new().unwrap();
    let tool = EditWorkflowTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({
            "graph": valid_graph(),
            "ops": [ { "op": "remove_node", "id": "ghost" } ]
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    let out = result.output();
    assert!(out.contains("remove_node"), "{out}");
    assert!(out.contains("edit_workflow again"), "{out}");
}

#[tokio::test]
async fn edit_workflow_bad_op_reports_index_type_and_shape() {
    let tmp = TempDir::new().unwrap();
    let tool = EditWorkflowTool::new(test_config(&tmp));
    // ops 0 and 1 are well-formed; op 2 is an add_node missing its `node`.
    let result = tool
        .execute(json!({
            "graph": valid_graph(),
            "ops": [
                { "op": "set_node_name", "id": "a", "name": "One" },
                { "op": "set_node_name", "id": "a", "name": "Two" },
                { "op": "add_node", "id": "b" }
            ]
        }))
        .await
        .unwrap();
    assert!(result.is_error, "{}", result.output());
    let out = result.output();
    // Names the failing op index, its op type, and the expected shape for it.
    assert!(out.contains("op 2"), "{out}");
    assert!(out.contains("add_node"), "{out}");
    assert!(out.contains("node:"), "expected add_node shape in: {out}");
    assert!(out.contains("edit_workflow again"), "{out}");
}

#[tokio::test]
async fn edit_workflow_missing_op_field_lists_valid_types() {
    let tmp = TempDir::new().unwrap();
    let tool = EditWorkflowTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({
            "graph": valid_graph(),
            "ops": [ { "id": "a", "name": "No op tag" } ]
        }))
        .await
        .unwrap();
    assert!(result.is_error, "{}", result.output());
    let out = result.output();
    assert!(out.contains("op 0"), "{out}");
    assert!(out.contains("missing `op` field"), "{out}");
    assert!(out.contains("update_node_config"), "{out}");
}

#[tokio::test]
async fn edit_workflow_add_node_exists_carries_ordering_hint() {
    let tmp = TempDir::new().unwrap();
    let tool = EditWorkflowTool::new(test_config(&tmp));
    // Re-adding an existing node id fails in-order; the hint should point at the
    // remove-first / patch-in-place fix.
    let result = tool
        .execute(json!({
            "graph": valid_graph(),
            "ops": [
                { "op": "add_node", "node": { "id": "a", "kind": "merge", "name": "Dup" } }
            ]
        }))
        .await
        .unwrap();
    assert!(result.is_error, "{}", result.output());
    let out = result.output();
    assert!(out.contains("already exists"), "{out}");
    assert!(out.contains("array order"), "{out}");
    assert!(out.contains("remove_node"), "{out}");
    assert!(out.contains("update_node_config"), "{out}");
}

#[tokio::test]
async fn edit_workflow_accepts_node_id_aliases_end_to_end() {
    let tmp = TempDir::new().unwrap();
    let tool = EditWorkflowTool::new(test_config(&tmp));
    // A valid ops array using the `node_id` alias (the natural agent guess)
    // applies cleanly through edit_workflow.
    let result = tool
        .execute(json!({
            "graph": valid_graph(),
            "name": "Aliased edit",
            "ops": [
                { "op": "update_node_config", "node_id": "a", "config": { "prompt": "aliased" } },
                { "op": "set_node_name", "node_id": "a", "name": "Aliased step" }
            ]
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["type"], "workflow_proposal");
    let nodes = parsed["graph"]["nodes"].as_array().unwrap();
    let agent = nodes.iter().find(|n| n["id"] == "a").unwrap();
    assert_eq!(agent["config"]["prompt"], "aliased");
    assert_eq!(agent["name"], "Aliased step");
}

#[tokio::test]
async fn edit_workflow_rejects_a_result_that_is_structurally_invalid() {
    let tmp = TempDir::new().unwrap();
    let tool = EditWorkflowTool::new(test_config(&tmp));
    // Removing the only trigger leaves the graph structurally invalid.
    let result = tool
        .execute(json!({
            "graph": valid_graph(),
            "ops": [ { "op": "remove_node", "id": "t" } ]
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("trigger"), "{}", result.output());
}

#[tokio::test]
async fn edit_workflow_edits_a_saved_flow_by_id() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // Create a saved flow to edit.
    let flow = ops::flows_create(&config, "Base flow".to_string(), valid_graph(), false)
        .await
        .unwrap()
        .value;

    let tool = EditWorkflowTool::new(config.clone());
    let result = tool
        .execute(json!({
            "flow_id": flow.id,
            "ops": [ { "op": "set_node_name", "id": "a", "name": "Renamed step" } ]
        }))
        .await
        .unwrap();

    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    // Default name falls back to the base flow's name.
    assert_eq!(parsed["name"], "Base flow");
    let nodes = parsed["graph"]["nodes"].as_array().unwrap();
    let agent = nodes.iter().find(|n| n["id"] == "a").unwrap();
    assert_eq!(agent["name"], "Renamed step");
}

// ── validate_workflow (F3: standalone check) ─────────────────────────────────

#[tokio::test]
async fn validate_workflow_reports_ok_for_a_valid_graph() {
    let tmp = TempDir::new().unwrap();
    let tool = ValidateWorkflowTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({ "graph": valid_graph() }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["structurally_valid"], true);
    assert_eq!(parsed["errors"].as_array().unwrap().len(), 0);
    assert_eq!(parsed["gate_errors"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn validate_workflow_surfaces_all_structural_errors() {
    let tmp = TempDir::new().unwrap();
    let tool = ValidateWorkflowTool::new(test_config(&tmp));
    // No trigger + a dangling edge.
    let graph = json!({
        "nodes": [ { "id": "a", "kind": "agent", "name": "A", "config": { "prompt": "hi" } } ],
        "edges": [ { "from_node": "a", "to_node": "ghost" } ]
    });
    let result = tool.execute(json!({ "graph": graph })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["ok"], false);
    assert_eq!(parsed["structurally_valid"], false);
    let codes: Vec<&str> = parsed["error_details"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"missing_trigger"), "{codes:?}");
    assert!(codes.contains(&"unknown_node"), "{codes:?}");
}

#[tokio::test]
async fn validate_workflow_requires_a_base() {
    let tmp = TempDir::new().unwrap();
    let tool = ValidateWorkflowTool::new(test_config(&tmp));
    let result = tool.execute(json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("flow_id"));
}

#[tokio::test]
async fn edit_workflow_edits_a_draft_and_writes_back() {
    use crate::openhuman::flows::DraftOrigin;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // A draft holding the base graph.
    let draft = ops::flows_draft_create(
        &config,
        None,
        "Draft flow".to_string(),
        valid_graph(),
        DraftOrigin::Chat,
    )
    .unwrap()
    .value;

    let tool = EditWorkflowTool::new(config.clone());
    let result = tool
        .execute(json!({
            "draft_id": draft.id,
            "ops": [ { "op": "add_node", "node": { "id": "b", "kind": "merge", "name": "Join" } } ]
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["draft_id"], draft.id);
    assert_eq!(parsed["graph"]["nodes"].as_array().unwrap().len(), 3);

    // The edit was written back to the draft (survives for the next turn).
    let reloaded = ops::flows_draft_get(&config, &draft.id).unwrap().value;
    assert_eq!(reloaded.graph["nodes"].as_array().unwrap().len(), 3);
}

// ── Phase 4: gated create / duplicate / debug loop (F4) ──────────────────────

#[tokio::test]
async fn create_workflow_creates_a_disabled_flow() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let tool = CreateWorkflowTool::new(config.clone());
    // valid_graph has a manual trigger — flows_create would normally make it
    // enabled; create_workflow must force it DISABLED.
    let result = tool
        .execute(json!({ "name": "Agent-made", "graph": valid_graph() }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["type"], "workflow_created");
    assert_eq!(parsed["enabled"], false);
    // Persisted and really disabled.
    let flow_id = parsed["flow_id"].as_str().unwrap();
    let flow = ops::flows_get(&config, flow_id).await.unwrap().value;
    assert!(!flow.enabled, "agent-created flows are born disabled");
}

#[tokio::test]
async fn create_workflow_rejects_an_invalid_graph() {
    let tmp = TempDir::new().unwrap();
    let tool = CreateWorkflowTool::new(test_config(&tmp));
    let bad = json!({
        "nodes": [ { "id": "a", "kind": "output_parser", "name": "A" } ],
        "edges": []
    });
    let result = tool
        .execute(json!({ "name": "Bad", "graph": bad }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("create_workflow again"));
}

#[tokio::test]
async fn duplicate_flow_creates_a_disabled_copy() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = ops::flows_create(&config, "Original".to_string(), valid_graph(), false)
        .await
        .unwrap()
        .value;
    let tool = DuplicateFlowTool::new(config.clone());
    let result = tool.execute(json!({ "flow_id": flow.id })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["type"], "workflow_duplicated");
    assert_eq!(parsed["enabled"], false);
    assert_ne!(parsed["flow_id"].as_str().unwrap(), flow.id);
}

#[tokio::test]
async fn list_flow_runs_is_empty_for_a_fresh_flow() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = ops::flows_create(&config, "F".to_string(), valid_graph(), false)
        .await
        .unwrap()
        .value;
    let tool = ListFlowRunsTool::new(config.clone());
    let result = tool.execute(json!({ "flow_id": flow.id })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["runs"].as_array().unwrap().len(), 0);
}

#[test]
fn phase4_write_tools_have_the_right_permissions() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert_eq!(
        CreateWorkflowTool::new(config.clone()).permission_level(),
        PermissionLevel::Write
    );
    assert!(CreateWorkflowTool::new(config.clone()).external_effect());
    assert_eq!(
        CancelFlowRunTool::new(config.clone()).permission_level(),
        PermissionLevel::Write
    );
    assert_eq!(
        ResumeFlowRunTool::new(config.clone()).permission_level(),
        PermissionLevel::Execute
    );
    assert_eq!(
        ListFlowRunsTool::new(config.clone()).permission_level(),
        PermissionLevel::None
    );
}

// ── WS2: unified draft_id|flow_id|graph handles + explicit persistence state ──

#[tokio::test]
async fn edit_workflow_by_flow_id_seeds_a_retrievable_draft_and_marks_unpersisted() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // A saved flow to edit — editing it must NOT write onto the flow (the WS2
    // bug: a flow_id edit used to persist nothing and return no handle).
    let flow = ops::flows_create(&config, "Base flow".to_string(), valid_graph(), false)
        .await
        .unwrap()
        .value;

    let tool = EditWorkflowTool::new(config.clone());
    let result = tool
        .execute(json!({
            "flow_id": flow.id,
            "ops": [ { "op": "set_node_name", "id": "a", "name": "Renamed step" } ]
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();

    // The edit lives on a NEW draft, is explicitly NOT persisted, and echoes the
    // flow it derives from plus a `next` hint naming the draft.
    assert_eq!(parsed["persisted"], false);
    assert_eq!(parsed["flow_id"], flow.id.as_str());
    let draft_id = parsed["draft_id"]
        .as_str()
        .expect("edit_workflow by flow_id returns a draft_id")
        .to_string();
    assert!(parsed["next"].as_str().unwrap().contains(&draft_id));

    // The draft is retrievable via ops::flows_draft_get and holds the EDITED
    // graph, linked back to the source flow.
    let draft = ops::flows_draft_get(&config, &draft_id).unwrap().value;
    assert_eq!(draft.flow_id.as_deref(), Some(flow.id.as_str()));
    let agent = draft.graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "a")
        .unwrap();
    assert_eq!(agent["name"], "Renamed step");

    // The SAVED flow is untouched — the whole point of WS2.
    let saved = ops::flows_get(&config, &flow.id).await.unwrap().value;
    let saved_graph = serde_json::to_value(&saved.graph).unwrap();
    let saved_agent = saved_graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "a")
        .unwrap();
    assert_eq!(
        saved_agent["name"], "Summarize",
        "the flow must not be edited"
    );
}

#[tokio::test]
async fn dry_run_workflow_by_flow_id_runs_the_saved_flow_graph() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = ops::flows_create(&config, "Runnable".to_string(), valid_graph(), false)
        .await
        .unwrap()
        .value;
    let tool = DryRunWorkflowTool::new(policy(AutonomyLevel::Supervised), config.clone());
    let result = tool.execute(json!({ "flow_id": flow.id })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["sandbox"], true);
    assert_eq!(parsed["ok"], true);
}

#[tokio::test]
async fn validate_workflow_by_draft_id_checks_the_draft_graph() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let draft = ops::flows_draft_create(
        &config,
        None,
        "Draft".to_string(),
        valid_graph(),
        crate::openhuman::flows::DraftOrigin::Chat,
    )
    .unwrap()
    .value;
    let tool = ValidateWorkflowTool::new(config.clone());
    let result = tool.execute(json!({ "draft_id": draft.id })).await.unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["structurally_valid"], true);
}

#[tokio::test]
async fn save_workflow_by_draft_id_persists_the_draft_graph_onto_the_flow() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // A flow seeded with a bare 1-node graph.
    let flow_id = seed_flow(&config, "Blank flow").await;
    // A draft holding the richer 2-node valid graph, linked to that flow.
    let draft = ops::flows_draft_create(
        &config,
        Some(flow_id.clone()),
        "Draft".to_string(),
        valid_graph(),
        crate::openhuman::flows::DraftOrigin::Chat,
    )
    .unwrap()
    .value;

    let tool = SaveWorkflowTool::new(config.clone());
    let result = tool
        .execute(json!({ "flow_id": flow_id, "draft_id": draft.id }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["type"], "workflow_saved");
    assert_eq!(parsed["persisted"], true);
    assert_eq!(parsed["node_count"], 2);

    // The draft's graph really landed on the flow.
    let saved = ops::flows_get(&config, &flow_id).await.unwrap().value;
    assert_eq!(saved.graph.nodes.len(), 2);
}

#[tokio::test]
async fn revise_workflow_proposal_is_marked_unpersisted() {
    let tmp = TempDir::new().unwrap();
    let tool = ReviseWorkflowTool::new(test_config(&tmp));
    let result = tool
        .execute(json!({ "name": "R", "graph": valid_graph() }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());
    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["persisted"], false);
}
