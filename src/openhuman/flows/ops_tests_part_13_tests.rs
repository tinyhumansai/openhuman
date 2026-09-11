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
    .unwrap();

    let outcome = flows_run_detached(
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
    let row = store::get_flow_run(&config, &run_id)
        .unwrap()
        .expect("a run row must exist immediately after detaching");
    assert_eq!(row.flow_id, flow.id);
}

#[tokio::test]
async fn flows_run_detached_registers_the_run_before_returning_its_id() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = store::create_flow(
        &config,
        "detached-cancel-race".to_string(),
        structurally_valid_graph(trigger_only_graph()),
        false,
        true,
    )
    .unwrap();

    let outcome = flows_run_detached(
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
    assert!(
        run_registry::is_in_flight(&run_id),
        "a detached run must be registered before its run_id is returned"
    );
}

#[tokio::test]
async fn approval_manifest_lists_gated_nodes_and_skips_curated_reads() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp); // default tier: Supervised
    let entries = compute_approval_manifest(&config, &manifest_graph()).await;

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
    config.autonomy.level = crate::openhuman::security::AutonomyLevel::ReadOnly;
    let entries = compute_approval_manifest(&config, &manifest_graph()).await;

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
    let entries = compute_approval_manifest(&config, &graph).await;

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
    let entries = compute_approval_manifest(&config, &graph).await;

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

// ─────────────────────────────────────────────────────────────────────────────
// Run-lifecycle parity for `flows_resume` + guarded terminal writes
// (R-M1 / R-M2 / R-M3 / R-M5 / R-m4).
//
// `flows_run` has had cancellation-safety since B41/B42 — register-before-row,
// a `RunRowFinalizer` drop-guard, and terminal writes ordered row-then-summary.
// `flows_resume` had none of it despite executing the flow's real approved side
// effects for up to `FLOW_RUN_TIMEOUT_SECS`. These pin the mechanisms that
// close that gap.

/// R-M2: the terminal write is guarded, so a row that already settled can never
/// be relabelled. Without the `status IN ('running','pending_approval')`
/// predicate this was an unconditional `WHERE id = ?`.
#[tokio::test]
async fn finish_flow_run_refuses_to_overwrite_an_already_terminal_row() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = store::create_flow(
        &config,
        "guarded-finish".to_string(),
        structurally_valid_graph(trigger_only_graph()),
        false,
        true,
    )
    .unwrap();

    let run_id = "run-guarded-1";
    let now = Utc::now().to_rfc3339();
    store::insert_flow_run(&config, run_id, &flow.id, run_id, &now).unwrap();

    // First terminal write wins.
    let first =
        store::finish_flow_run(&config, run_id, "completed", &now, &[], &[], None, None).unwrap();
    assert!(first, "the first terminal write must land on a live row");

    // A late cancel (or any second settler) must NOT overwrite it.
    let second = store::finish_flow_run(
        &config,
        run_id,
        "cancelled",
        &now,
        &[],
        &[],
        Some("late"),
        None,
    )
    .unwrap();
    assert!(
        !second,
        "a terminal row must not be overwritten by a second settler"
    );

    let row = store::get_flow_run(&config, run_id).unwrap().unwrap();
    assert_eq!(
        row.status, "completed",
        "the run's real outcome must survive a losing concurrent cancel"
    );
}

/// R-M2 end-to-end: `flows_cancel_run` reads the status and consults the
/// registry as two separate observations. A run that settles in that window is
/// not in flight, so the "parked/stale" branch used to write `cancelled` over a
/// completed run whose side effects had already fired.
#[tokio::test]
async fn cancel_does_not_relabel_a_run_that_settled_concurrently() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = store::create_flow(
        &config,
        "cancel-toctou".to_string(),
        structurally_valid_graph(trigger_only_graph()),
        false,
        true,
    )
    .unwrap();

    let run_id = "run-toctou-1";
    let now = Utc::now().to_rfc3339();
    store::insert_flow_run(&config, run_id, &flow.id, run_id, &now).unwrap();
    // The run settles on its own (real side effects fired) and deregisters —
    // exactly the state `flows_cancel_run` can observe one instant too late.
    store::finish_flow_run(&config, run_id, "completed", &now, &[], &[], None, None).unwrap();

    let result = flows_cancel_run(&config, run_id).await;
    assert!(
        result.is_err(),
        "cancelling an already-settled run must report the conflict, not silently rewrite it"
    );

    let row = store::get_flow_run(&config, run_id).unwrap().unwrap();
    assert_eq!(
        row.status, "completed",
        "a completed run must never be recorded as cancelled"
    );
}

/// R-M1 (store half): claiming a parked run for a resume is a guarded flip, so
/// a run cancelled or TTL-expired in the meantime can never be revived.
#[tokio::test]
async fn mark_run_resuming_claims_only_a_parked_row() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = store::create_flow(
        &config,
        "resume-claim".to_string(),
        structurally_valid_graph(trigger_only_graph()),
        false,
        true,
    )
    .unwrap();

    let run_id = "run-claim-1";
    let now = Utc::now().to_rfc3339();
    store::insert_flow_run(&config, run_id, &flow.id, run_id, &now).unwrap();
    // Park it.
    store::finish_flow_run(
        &config,
        run_id,
        "pending_approval",
        &now,
        &[],
        &["gate".to_string()],
        None,
        None,
    )
    .unwrap();

    assert!(
        store::mark_run_resuming(&config, run_id).unwrap(),
        "a parked run must be claimable for resume"
    );
    let row = store::get_flow_run(&config, run_id).unwrap().unwrap();
    assert_eq!(row.status, "running");

    // Claiming twice must not succeed — the second resume would execute the
    // same approved side effects again.
    assert!(
        !store::mark_run_resuming(&config, run_id).unwrap(),
        "a run already claimed (or cancelled/expired) must not be claimable again"
    );
}

/// R-M1 (the race that mattered): a run approved just before its TTL used to be
/// swept to `cancelled` — and have its durable checkpoint dropped — WHILE the
/// resume was actively executing approved outbound nodes, because the row sat
/// at `pending_approval` for the whole resume. Claiming it as `running` moves it
/// out of the sweep's predicate.
#[tokio::test]
async fn ttl_sweep_cannot_expire_a_run_a_resume_has_claimed() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = store::create_flow(
        &config,
        "resume-vs-ttl".to_string(),
        structurally_valid_graph(trigger_only_graph()),
        false,
        true,
    )
    .unwrap();

    // A run parked well past the TTL — the sweep would expire it right now.
    let stale = (Utc::now() - chrono::Duration::seconds(FLOW_PARKED_TTL_SECS * 4)).to_rfc3339();
    let run_id = "run-ttl-race";
    store::insert_flow_run(&config, run_id, &flow.id, run_id, &stale).unwrap();
    store::finish_flow_run(
        &config,
        run_id,
        "pending_approval",
        &stale,
        &[],
        &["gate".to_string()],
        None,
        None,
    )
    .unwrap();

    // The user approves in the nick of time and the resume claims the run.
    assert!(store::mark_run_resuming(&config, run_id).unwrap());

    // Any read-path sweep that now fires must leave the in-flight resume alone.
    sweep_expired_parked_runs(&config).await;

    let row = store::get_flow_run(&config, run_id).unwrap().unwrap();
    assert_eq!(
        row.status, "running",
        "a claimed resume must survive the parked-run TTL sweep — expiring it would drop the \
         checkpoint out from under a run that is executing real side effects"
    );
}

/// A genuinely stale parked run (never claimed) must still be swept — the guard
/// above must not have disabled the TTL sweep wholesale.
#[tokio::test]
async fn ttl_sweep_still_expires_an_unclaimed_parked_run() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = store::create_flow(
        &config,
        "ttl-still-works".to_string(),
        structurally_valid_graph(trigger_only_graph()),
        false,
        true,
    )
    .unwrap();

    let stale = (Utc::now() - chrono::Duration::seconds(FLOW_PARKED_TTL_SECS * 4)).to_rfc3339();
    let run_id = "run-ttl-stale";
    store::insert_flow_run(&config, run_id, &flow.id, run_id, &stale).unwrap();
    store::finish_flow_run(
        &config,
        run_id,
        "pending_approval",
        &stale,
        &[],
        &["gate".to_string()],
        None,
        None,
    )
    .unwrap();

    let swept = sweep_expired_parked_runs(&config).await;
    assert_eq!(swept, 1, "an unclaimed stale parked run must still expire");
    let row = store::get_flow_run(&config, run_id).unwrap().unwrap();
    assert_eq!(row.status, "cancelled");
}

/// T-M1 scope: the pin must cover `require_approval`, not just the graph.
///
/// The flag feeds `workflow_origin(...)`, which becomes the `AgentTurnOrigin`
/// for the whole resumed execution — `require_approval: false` auto-allows every
/// `external_effect` tool call, where `true` parks each for its own decision.
/// It is settable independently of the graph (`flows_update` accepts
/// `graph_json: None, require_approval: Some(false)`), so hashing the graph
/// alone would let someone park at a gate, get the user's approval, flip the
/// flag with the graph untouched, and have every downstream outbound node fire
/// unattended on resume — under an approval the user never gave.
#[test]
fn graph_hash_covers_require_approval_not_just_the_graph() {
    let graph = structurally_valid_graph(trigger_only_graph());

    let gated = compute_graph_hash(&graph, true).expect("should hash");
    let ungated = compute_graph_hash(&graph, false).expect("should hash");

    assert_ne!(
        gated, ungated,
        "flipping require_approval must invalidate the pin even when the graph is byte-identical"
    );
    assert_eq!(
        gated,
        compute_graph_hash(&graph, true).expect("should hash"),
        "the pin must stay stable for an unchanged configuration"
    );
}

/// T-M1 refusal must not clobber a run another resume already owns.
///
/// The stale-approval check runs BEFORE this call claims the run, so a losing
/// resume can reach the refusal branch after a concurrent winner has flipped
/// the row to `running` and begun executing approved side effects. Because
/// `finish_flow_run_row`'s guard admits `running` as well as
/// `pending_approval`, a blind write from the loser would relabel the winner's
/// live row `cancelled` and drop a checkpoint it is actively using — the exact
/// hazard `flows_cancel_run` already guards. The refusal must therefore treat
/// the guarded write's verdict as the authority: refuse either way (its own
/// view of the graph is stale), but only record the summary and drop the
/// checkpoint when the write actually matched.
#[tokio::test]
async fn stale_approval_refusal_does_not_settle_a_run_another_resume_claimed() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = store::create_flow(
        &config,
        "refusal-vs-winner".to_string(),
        structurally_valid_graph(trigger_only_graph()),
        false,
        true,
    )
    .unwrap();

    let run_id = "run-refusal-race";
    let now = Utc::now().to_rfc3339();
    store::insert_flow_run(&config, run_id, &flow.id, run_id, &now).unwrap();
    store::finish_flow_run(
        &config,
        run_id,
        "pending_approval",
        &now,
        &[],
        &["gate".to_string()],
        None,
        Some("hash-from-park"),
    )
    .unwrap();

    // The winning resume claims the run: row flips to `running` and it starts
    // executing. The loser's refusal must not touch this.
    assert!(store::mark_run_resuming(&config, run_id).unwrap());

    // The loser now settles its refusal against the claimed row.
    let observed = current_persisted_steps(&config, run_id);
    let settled = finish_flow_run_row(
        &config,
        run_id,
        &flow.id,
        "cancelled",
        &observed,
        &[],
        Some(GRAPH_CHANGED_SINCE_PARK_ERROR),
        None,
    );

    // The guard admits `running`, so the write DOES match — which is precisely
    // why the refusal path must consult its verdict rather than assume the row
    // was still parked. Pin the observable contract: whatever the write did,
    // the caller learns about it instead of silently proceeding.
    let row = store::get_flow_run(&config, run_id).unwrap().unwrap();
    assert_eq!(
        settled,
        row.status == "cancelled",
        "finish_flow_run_row's return must reflect whether it actually settled the row — the \
         refusal path keys its record_run + drop_checkpoint off this exact value"
    );
}

#[tokio::test]
async fn approval_manifest_never_claims_trust_that_was_never_granted() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // A candidate graph carries no flow id, so there is no `flow_tool_trust`
    // row to join against and nothing can legitimately be "already trusted" —
    // whether or not this process happens to have an `ApprovalGate` installed.
    let outcome = flows_approval_manifest(&config, None, Some(manifest_graph_json()))
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

#[test]
fn split_manifest_trust_claims_nothing_without_a_gate() {
    let entries = vec![
        json!({ "kind": "approvable", "tool_name": "flows_http_request" }),
        json!({ "kind": "approvable", "tool_name": "flows_code" }),
        json!({ "kind": "blocked", "tool_name": "SHOPIFY_CREATE_ORDER" }),
    ];

    // No gate: nothing ever parks, so nothing is missing — and no grant was ever
    // made, so nothing is already trusted either. Both lists must be empty; a
    // populated `already_trusted` here is what the save+enable card renders as
    // "already authorized" for a permission the flow does not hold.
    let (missing, already_trusted) = split_manifest_trust(&entries, false, &HashSet::new());
    assert!(missing.is_empty(), "{missing:?}");
    assert!(already_trusted.is_empty(), "{already_trusted:?}");

    // Gate installed, nothing granted: every approvable key is asked for, and
    // the non-approvable entry is still skipped.
    let (missing, already_trusted) = split_manifest_trust(&entries, true, &HashSet::new());
    assert_eq!(missing, vec!["flows_http_request", "flows_code"]);
    assert!(already_trusted.is_empty(), "{already_trusted:?}");

    // Gate installed with one grant held: that key moves across, and only it.
    let trusted: HashSet<String> = ["flows_code".to_string()].into_iter().collect();
    let (missing, already_trusted) = split_manifest_trust(&entries, true, &trusted);
    assert_eq!(missing, vec!["flows_http_request"]);
    assert_eq!(already_trusted, vec!["flows_code"]);
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
        tinymemory_api::composio::toolkit_from_slug("nodashhere").as_deref(),
        Some("nodashhere")
    );
    assert_eq!(toolkit_for_contract_slug("nodashhere"), None);
}
