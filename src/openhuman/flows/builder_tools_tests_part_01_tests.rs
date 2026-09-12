use super::*;

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
async fn get_tool_contract_rejects_a_slug_with_no_action_segment() {
    let tmp = TempDir::new().unwrap();
    let tool = GetToolContractTool::new(test_config(&tmp));

    // A bare toolkit-ish token names no action, so this endpoint cannot return
    // anything for it. Before the shape guard, `toolkit_from_slug` fell back to
    // the whole string and the model got the catalog error instead — which tells
    // it to retry a fetch that can never succeed.
    for slug in ["nodashhere", "_LEADING_UNDERSCORE", "GMAIL_"] {
        let result = tool.execute(json!({ "slug": slug })).await.unwrap();
        assert!(result.is_error, "slug {slug:?} must be rejected");
        assert!(
            result.output().contains("'<TOOLKIT>_<ACTION>'"),
            "slug {slug:?} must get the malformed-slug message, got: {}",
            result.output()
        );
    }
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
