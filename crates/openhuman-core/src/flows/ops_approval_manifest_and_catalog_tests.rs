use super::*;

#[tokio::test]
async fn flows_run_detached_returns_running_run_id_and_inserts_row() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = store::create_flow(
        &config,
        "detached".to_string(),
        structurally_valid_graph(trigger_only_graph()),
        false,
        true,
    )
    .unwrap()
    .workflow;

    let outcome = flows_run_detached(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &flow.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .expect("detached run must start");

    assert_eq!(outcome.value["status"], json!("running"));
    assert_eq!(outcome.value["detached"], json!(true));
    let run_id = outcome.value["run_id"]
        .as_str()
        .expect("run_id must be a string")
        .to_string();
    assert!(
        run_id.starts_with(&format!("flow:{}:", flow.id)),
        "run_id: {run_id}"
    );

    // The `running` row is inserted synchronously before the background task is
    // spawned, so the copilot's immediate `get_flow_run(run_id)` poll finds it.
    let row = flows_get_run(&config, &run_id)
        .await
        .expect("a run row must exist immediately after detaching")
        .value;
    assert_eq!(row.flow_id, flow.id);
}

#[tokio::test]
async fn flows_run_detached_registers_the_run_before_returning_its_id() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = store::create_flow(
        &config,
        "detached-cancel-race".to_string(),
        structurally_valid_graph(approval_gated_graph()),
        false,
        true,
    )
    .unwrap()
    .workflow;

    let outcome = flows_run_detached(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &flow.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .expect("detached run must start");
    let run_id = outcome.value["run_id"].as_str().unwrap().to_string();

    // The moment the agent can see this `run_id` it can be cancelled. If
    // registration happened inside the spawned task instead, this would be
    // false until the task was first polled — and `flows_cancel_run` would take
    // its "parked/stale" branch, writing a terminal `cancelled` row and
    // dropping the checkpoint while the background run went on to execute the
    // flow's real side effects and overwrite that status.
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let storage = crate::flows::catalog_storage::register(&ctx, &config)
        .await
        .expect("the detached run's owner-scoped CatalogStorage must be available")
        .storage();
    assert!(
        crate::flows::runs_adapter::run_bindings(&storage, &run_id).is_some(),
        "the detached run must retain its owner and CapsHost bindings before returning its run_id"
    );
}

#[tokio::test]
async fn approval_manifest_lists_gated_nodes_and_skips_curated_reads() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp); // default tier: Supervised
    let entries = compute_approval_manifest(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &manifest_graph(),
    )
    .await
    .unwrap();

    let kinds = entry_kinds_by_tool(&entries);
    // Supervised prompts on every acting class → all three are approvable.
    assert!(kinds.contains(&("flows_http_request".into(), "approvable".into())));
    assert!(kinds.contains(&("flows_code".into(), "approvable".into())));
    assert!(kinds.contains(&("SHOPIFY_CREATE_ORDER".into(), "approvable".into())));
    // A curated Read action never reaches the gate — must NOT be listed.
    assert!(
        !kinds.iter().any(|(t, _)| t == "SHOPIFY_COUNT_PRODUCTS"),
        "curated Read slug must be excluded from the manifest: {kinds:?}"
    );
    assert_eq!(entries.len(), 3, "{entries:?}");
}

#[tokio::test]
async fn approval_manifest_marks_blocked_classes_under_readonly_tier() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.autonomy.level = crate::security::AutonomyLevel::ReadOnly;
    let entries = compute_approval_manifest(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &manifest_graph(),
    )
    .await
    .unwrap();

    let kinds = entry_kinds_by_tool(&entries);
    // Read-only blocks every non-Read class: informational, never approvable.
    assert!(kinds.contains(&("flows_http_request".into(), "blocked".into())));
    assert!(kinds.contains(&("flows_code".into(), "blocked".into())));
    assert!(kinds.contains(&("SHOPIFY_CREATE_ORDER".into(), "blocked".into())));
    assert!(!kinds.iter().any(|(_, k)| k == "approvable"), "{kinds:?}");
}

#[tokio::test]
async fn approval_manifest_dedupes_repeated_tools_and_flags_dynamic_slugs() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let graph = structurally_valid_graph(json!({
        "name": "dedupe-dynamic",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "h1", "kind": "http_request", "name": "One",
              "config": { "url": "https://a.example.com", "method": "GET" } },
            { "id": "h2", "kind": "http_request", "name": "Two",
              "config": { "url": "https://b.example.com", "method": "POST" } },
            { "id": "d", "kind": "tool_call", "name": "Dynamic",
              "config": { "slug": "={{ $json.slug }}" } }
        ],
        "edges": [
            { "from_node": "t", "from_port": "main", "to_node": "h1" },
            { "from_node": "h1", "from_port": "main", "to_node": "h2" },
            { "from_node": "h2", "from_port": "main", "to_node": "d" }
        ]
    }));
    let entries = compute_approval_manifest(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &graph,
    )
    .await
    .unwrap();

    // Two http nodes share one trust key → exactly one row.
    let http_rows = entries
        .iter()
        .filter(|e| e.get("tool_name").and_then(Value::as_str) == Some("flows_http_request"))
        .count();
    assert_eq!(http_rows, 1, "{entries:?}");
    // The `=` slug cannot be pre-approved; it is disclosed as dynamic.
    assert!(
        entries
            .iter()
            .any(|e| e.get("kind").and_then(Value::as_str) == Some("dynamic")),
        "{entries:?}"
    );
}

#[tokio::test]
async fn approval_manifest_discloses_agent_ref_nodes_only() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let graph = structurally_valid_graph(json!({
        "name": "agent-disclosure",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "plain", "kind": "agent", "name": "Plain LLM",
              "config": { "prompt": "Summarize {{input}}" } },
            { "id": "harness", "kind": "agent", "name": "Full agent",
              "config": { "prompt": "Do things", "agent_ref": "orchestrator" } }
        ],
        "edges": [
            { "from_node": "t", "from_port": "main", "to_node": "plain" },
            { "from_node": "plain", "from_port": "main", "to_node": "harness" }
        ]
    }));
    let entries = compute_approval_manifest(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &graph,
    )
    .await
    .unwrap();

    let agent_rows: Vec<_> = entries
        .iter()
        .filter(|e| e.get("kind").and_then(Value::as_str) == Some("agent"))
        .collect();
    // Only the harness-backed agent node is disclosed; a plain LLM node has
    // no acting side effect and must not scare the user with a row.
    assert_eq!(agent_rows.len(), 1, "{entries:?}");
    assert_eq!(
        agent_rows[0].get("node_id").and_then(Value::as_str),
        Some("harness")
    );
}

#[tokio::test]
async fn approval_manifest_never_claims_trust_that_was_never_granted() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // A candidate graph carries no flow id, so there is no `flow_tool_trust`
    // row to join against and nothing can legitimately be "already trusted" —
    // whether or not this process happens to have an `ApprovalGate` installed.
    let outcome = flows_approval_manifest(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        None,
        Some(manifest_graph_json()),
    )
    .await
    .expect("manifest must compute for a candidate graph");

    let already_trusted = outcome.value["already_trusted"]
        .as_array()
        .expect("already_trusted must be an array");
    assert!(
        already_trusted.is_empty(),
        "already_trusted must never name a permission no grant was ever made for; \
         the save+enable card renders these as authorized: {outcome:?}"
    );

    let gate_installed = outcome.value["gate_installed"]
        .as_bool()
        .expect("gate_installed must be a bool");
    let missing: Vec<&str> = outcome.value["missing"]
        .as_array()
        .expect("missing must be an array")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    if gate_installed {
        // Gate installed, no grants held: every approvable key is asked for.
        assert!(
            missing.contains(&"flows_http_request") && missing.contains(&"flows_code"),
            "{missing:?}"
        );
    } else {
        // No gate: nothing ever parks, so nothing is missing either — the two
        // lists are empty together and `gate_installed` is the only signal.
        assert!(missing.is_empty(), "{missing:?}");
    }
}

#[tokio::test]
async fn get_tool_contract_rejects_a_malformed_slug_before_any_catalog_fetch() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Each of these fails the `<TOOLKIT>_<ACTION>` shape, so the guard must
    // fire and no live catalog round trip may be attempted (these tests run
    // without Composio credentials or network).
    for slug in ["nodashhere", "   ", "_LEADING_UNDERSCORE", "GMAIL_"] {
        let err = flows_get_tool_contract(&config, slug)
            .await
            .expect_err("a malformed slug must be rejected");
        assert!(
            err.contains("'<TOOLKIT>_<ACTION>'"),
            "slug {slug:?} must get the malformed-slug message, got: {err}"
        );
    }

    // The message names what the caller sent, not the trimmed form — a blank
    // slug used to be reported as `slug ''`, naming nothing the caller typed.
    let err = flows_get_tool_contract(&config, " nodashhere ")
        .await
        .expect_err("a malformed slug must be rejected");
    assert!(
        err.contains("slug ' nodashhere '"),
        "the error must quote the caller's own slug, got: {err}"
    );
}

#[test]
fn toolkit_for_contract_slug_accepts_real_action_slugs() {
    // Ordinary single-segment toolkit.
    assert_eq!(
        toolkit_for_contract_slug("GMAIL_SEND_EMAIL").as_deref(),
        Some("gmail")
    );
    // Surrounding whitespace is tolerated, as it was before the guard.
    assert_eq!(
        toolkit_for_contract_slug("  GMAIL_SEND_EMAIL  ").as_deref(),
        Some("gmail")
    );
    // Every multi-segment toolkit prefix ends in `_`, so a real action under
    // one of them always has non-empty segments either side of its first `_`.
    assert_eq!(
        toolkit_for_contract_slug("ZOHO_MAIL_SEND_EMAIL").as_deref(),
        Some("zoho_mail")
    );
    assert_eq!(
        toolkit_for_contract_slug("ONE_DRIVE_UPLOAD_FILE").as_deref(),
        Some("one_drive")
    );
    assert_eq!(
        toolkit_for_contract_slug("MICROSOFT_TEAMS_SEND_MESSAGE").as_deref(),
        Some("microsoft_teams")
    );

    // The shared helper stays permissive for its other callers — this stricter
    // rule is this controller's, not `toolkit_from_slug`'s.
    assert_eq!(
        crate::integrations::composio::contract::toolkit_from_slug("nodashhere").as_deref(),
        Some("nodashhere")
    );
    assert_eq!(toolkit_for_contract_slug("nodashhere"), None);
}
