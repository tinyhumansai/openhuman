use super::*;

/// The success-path mirror of the refusal test above: when nothing rewrites
/// the flow between park and resume, the recomputed hash matches the pinned
/// one and the resume proceeds exactly as it did before this guard existed.
#[tokio::test]
async fn flows_resume_succeeds_when_the_graph_is_unchanged() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();

    let run = flows_run(
        &config,
        &created.value.id,
        json!({ "x": 1 }),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();
    let pending: Vec<String> =
        serde_json::from_value(run.value["pending_approvals"].clone()).unwrap();

    let parked_row = flows_get_run(&config, &thread_id).await.unwrap().value;
    assert!(
        parked_row.graph_hash.is_some(),
        "a freshly parked run must pin the graph it parked against"
    );

    let resumed = flows_resume(&config, &created.value.id, &thread_id, pending, vec![])
        .await
        .expect("resume must succeed when the pinned graph still matches the current one");
    assert_eq!(resumed.value["pending_approvals"], json!([]));
    assert!(
        !resumed.value["output"]["nodes"]["downstream"]["items"].is_null(),
        "downstream should run once the gate is approved via resume"
    );

    let run_row = flows_get_run(&config, &thread_id).await.unwrap().value;
    assert_eq!(run_row.status, "completed");
    assert!(
        run_row.graph_hash.is_none(),
        "a settled row clears its park-time pin rather than leaving it stale: {run_row:?}"
    );
}

/// Migration safety (T-M1 requirement #4): a `flow_runs` row written before
/// this guard existed reads back with `graph_hash IS NULL`. That must be
/// treated as "unknown — allow, with a warning", never as a hard refusal, so
/// upgrading mid-park can never strand an otherwise-valid in-flight approval
/// — even if the flow's graph was *also* edited in the meantime, since there
/// is nothing recorded to compare it against.
#[tokio::test]
async fn flows_resume_allows_a_legacy_row_with_null_graph_hash() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();

    let run = flows_run(
        &config,
        &created.value.id,
        json!({ "x": 1 }),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();
    let pending: Vec<String> =
        serde_json::from_value(run.value["pending_approvals"].clone()).unwrap();

    // Simulate a row written before the T-M1 migration: still `pending_approval`,
    // but with no graph hash pinned — exactly what `add_column_if_missing`
    // leaves behind for every row that existed before this feature shipped.
    let now = Utc::now().to_rfc3339();
    store::finish_flow_run(
        &config,
        &thread_id,
        "pending_approval",
        &now,
        &[],
        &pending,
        None,
        None,
    )
    .unwrap();
    let staged = flows_get_run(&config, &thread_id).await.unwrap().value;
    assert!(
        staged.graph_hash.is_none(),
        "fixture must simulate a legacy row with no pin"
    );

    // The flow is ALSO rewritten afterward — a legacy row has nothing to
    // compare against, so this must not matter.
    let mut rewritten = approval_gated_graph();
    rewritten["nodes"][2]["name"] = json!("Downstream (renamed)");
    store::update_flow_graph(
        &config,
        &created.value.id,
        created.value.name.clone(),
        structurally_valid_graph(rewritten),
        created.value.require_approval,
        None,  // enabled_override
        false, // force_disarm_if_automatic — this fixture isn't exercising the
        // manual->automatic disarm path, only the graph swap.
        None,
    )
    .unwrap();

    let resumed = flows_resume(&config, &created.value.id, &thread_id, pending, vec![])
        .await
        .expect("a legacy row with no graph_hash must still resume (unknown treated as allow)");
    assert_eq!(resumed.value["pending_approvals"], json!([]));
}

/// `compute_graph_hash` must hash graph *content*, not incidental JSON object
/// key order. Node `config` is a free-form `serde_json::Value` (see
/// `tinyflows::model::Node::config`), and this crate has the `preserve_order`
/// feature active transitively — `Value`'s object map keeps insertion order
/// rather than sorting automatically — so two structurally-identical graphs
/// built with the same config keys in a different order would hash
/// differently without the canonicalization `compute_graph_hash` applies.
#[test]
fn graph_hash_is_stable_across_serialization_key_order() {
    let graph_a = structurally_valid_graph(json!({
        "name": "order-test",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            {
                "id": "n",
                "kind": "output_parser",
                "name": "N",
                "config": { "a": 1, "b": 2, "nested": { "x": 1, "y": 2 } }
            }
        ],
        "edges": [ { "from_node": "t", "to_node": "n" } ]
    }));
    let graph_b = structurally_valid_graph(json!({
        "name": "order-test",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            {
                "id": "n",
                "kind": "output_parser",
                "name": "N",
                "config": { "nested": { "y": 2, "x": 1 }, "b": 2, "a": 1 }
            }
        ],
        "edges": [ { "from_node": "t", "to_node": "n" } ]
    }));

    let hash_a = compute_graph_hash(&graph_a, false).expect("graph_a should hash");
    let hash_b = compute_graph_hash(&graph_b, false).expect("graph_b should hash");
    assert_eq!(
        hash_a, hash_b,
        "the same graph content in a different key order must hash identically"
    );

    // Sanity: an actually-different graph must NOT collide.
    let mut graph_c_value = json!({
        "name": "order-test",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            {
                "id": "n",
                "kind": "output_parser",
                "name": "N",
                "config": { "a": 1, "b": 2, "nested": { "x": 1, "y": 2 } }
            }
        ],
        "edges": [ { "from_node": "t", "to_node": "n" } ]
    });
    graph_c_value["nodes"][1]["config"]["a"] = json!(999);
    let graph_c = structurally_valid_graph(graph_c_value);
    let hash_c = compute_graph_hash(&graph_c, false).expect("graph_c should hash");
    assert_ne!(
        hash_a, hash_c,
        "a genuinely different graph must not collide"
    );
}

#[tokio::test]
async fn flows_resume_marks_an_incompatible_legacy_checkpoint_failed() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();
    let run = flows_run(
        &config,
        &created.value.id,
        json!({ "x": 1 }),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();
    let pending: Vec<String> =
        serde_json::from_value(run.value["pending_approvals"].clone()).unwrap();

    // Simulate a graph persisted before the host compatibility gate existed.
    // The store layer intentionally trusts its typed caller; authoring paths
    // own validation.
    let legacy_graph = structurally_valid_graph(nested_conditional_fan_in_graph());
    store::update_flow_graph(
        &config,
        &created.value.id,
        created.value.name.clone(),
        legacy_graph.clone(),
        created.value.require_approval,
        None,
        false,
        None,
    )
    .unwrap();
    // T-M1: re-pin the parked row's graph_hash to this same (legacy,
    // incompatible) graph. Without this the fixture reads as "the graph
    // changed after park" (a DIFFERENT bug class this same PR now catches
    // earlier and refuses with a distinct message) rather than "the
    // checkpoint has always been incompatible" — the scenario this test
    // means to pin. A real legacy row predating T-M1 would carry
    // `graph_hash: NULL` and fall through the same way (see the
    // `flows_resume_allows_a_legacy_row_with_null_graph_hash` test above).
    let run_row_before = flows_get_run(&config, &thread_id).await.unwrap().value;
    let legacy_hash = compute_graph_hash(&legacy_graph, created.value.require_approval)
        .expect("fixture graph should hash");
    store::finish_flow_run(
        &config,
        &thread_id,
        "pending_approval",
        &run_row_before.finished_at.unwrap_or_default(),
        &run_row_before.steps,
        &run_row_before.pending_approvals,
        None,
        Some(&legacy_hash),
    )
    .unwrap();

    let error = flows_resume(&config, &created.value.id, &thread_id, pending, vec![])
        .await
        .expect_err("an incompatible checkpoint cannot be resumed safely");
    assert!(
        error.contains(UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN),
        "{error}"
    );

    let run_row = flows_get_run(&config, &thread_id).await.unwrap().value;
    assert_eq!(run_row.status, "failed");
    assert!(run_row.pending_approvals.is_empty());
    assert!(
        run_row
            .error
            .as_deref()
            .is_some_and(|value| value.contains(UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN)),
        "the terminal run row should retain the rejection reason: {run_row:?}"
    );
    let flow = flows_get(&config, &created.value.id).await.unwrap().value;
    assert_eq!(flow.last_status.as_deref(), Some("failed"));
}

#[tokio::test]
async fn flows_resume_marks_a_checkpoint_with_an_incompatible_saved_child_failed() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();
    let run = flows_run(
        &config,
        &created.value.id,
        json!({ "x": 1 }),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();
    let pending: Vec<String> =
        serde_json::from_value(run.value["pending_approvals"].clone()).unwrap();
    let child = store::create_flow(
        &config,
        "legacy unsafe child".to_string(),
        structurally_valid_graph(nested_conditional_fan_in_graph()),
        false,
        false,
    )
    .unwrap();
    let legacy_graph = structurally_valid_graph(referenced_child_graph(&child.id));
    store::update_flow_graph(
        &config,
        &created.value.id,
        created.value.name.clone(),
        legacy_graph.clone(),
        created.value.require_approval,
        None,
        false,
        None,
    )
    .unwrap();
    // T-M1: re-pin the parked row's hash to this same graph — see the sibling
    // legacy-checkpoint test above for why this fixture needs it now that a
    // graph swap is independently caught by the stale-approval guard.
    let run_row_before = flows_get_run(&config, &thread_id).await.unwrap().value;
    let legacy_hash = compute_graph_hash(&legacy_graph, created.value.require_approval)
        .expect("fixture graph should hash");
    store::finish_flow_run(
        &config,
        &thread_id,
        "pending_approval",
        &run_row_before.finished_at.unwrap_or_default(),
        &run_row_before.steps,
        &run_row_before.pending_approvals,
        None,
        Some(&legacy_hash),
    )
    .unwrap();

    let error = flows_resume(&config, &created.value.id, &thread_id, pending, vec![])
        .await
        .expect_err("an incompatible saved child cannot be resumed safely");
    assert!(
        error.contains(UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN),
        "{error}"
    );
    assert!(error.contains(&child.id), "{error}");

    let run_row = flows_get_run(&config, &thread_id).await.unwrap().value;
    assert_eq!(run_row.status, "failed");
    assert!(run_row.pending_approvals.is_empty());
    assert!(run_row
        .error
        .as_deref()
        .is_some_and(|value| value.contains(UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN)));
    let flow = flows_get(&config, &created.value.id).await.unwrap().value;
    assert_eq!(flow.last_status.as_deref(), Some("failed"));
}

#[tokio::test]
async fn flows_resume_missing_flow_errors() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = flows_resume(&config, "missing", "thread-1", vec![], vec![])
        .await
        .expect_err("must error");
    assert!(err.contains("not found"));
}

// ── flows_resume host-side approval guard (issue B2 finding #3) ──────────
//
// tinyflows 0.2's `resume_with_checkpointer` treats the resume call itself
// as approval of whatever gate paused the run — its `approvals` argument is
// advisory, not enforced by the crate. Live testing confirmed
// `flows_resume(..., approvals: [])` on a paused run still completed it.
// These tests exercise the host-side guard added in `flows::ops::flows_resume`
// that requires `approvals` to actually name a currently-pending gate,
// straight from the persisted `flow_runs` row, before ever calling into the
// engine.

#[tokio::test]
async fn flows_resume_with_empty_approvals_is_rejected_and_does_not_complete_the_run() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();

    let run = flows_run(
        &config,
        &created.value.id,
        json!({ "x": 1 }),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();

    let err = flows_resume(&config, &created.value.id, &thread_id, vec![], vec![])
        .await
        .expect_err("an empty approvals list must not silently approve the pending gate");
    assert!(
        err.contains("no pending approval matches"),
        "expected a clear approval-mismatch error, got: {err}"
    );

    // The run must still be sitting at pending_approval, not completed.
    let run_row = flows_get_run(&config, &thread_id).await.unwrap();
    assert_eq!(run_row.value.status, "pending_approval");
    assert_eq!(run_row.value.pending_approvals, vec!["gate".to_string()]);

    let reloaded = flows_get(&config, &created.value.id).await.unwrap();
    assert_eq!(
        reloaded.value.last_status.as_deref(),
        Some("pending_approval"),
        "a rejected resume attempt must not overwrite the flow's last_status as completed"
    );
}

#[tokio::test]
async fn flows_resume_with_mismatched_approvals_is_rejected() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();

    let run = flows_run(
        &config,
        &created.value.id,
        json!({ "x": 1 }),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();

    // Names a node id that is not actually pending for this run.
    let err = flows_resume(
        &config,
        &created.value.id,
        &thread_id,
        vec!["not-a-real-gate".to_string()],
        vec![],
    )
    .await
    .expect_err("approvals naming no actually-pending gate must be rejected");
    assert!(err.contains("no pending approval matches"));
}

#[tokio::test]
async fn flows_resume_with_the_correct_gate_completes_and_runs_downstream() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();

    let run = flows_run(
        &config,
        &created.value.id,
        json!({ "x": 1 }),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();

    let resumed = flows_resume(
        &config,
        &created.value.id,
        &thread_id,
        vec!["gate".to_string()],
        vec![],
    )
    .await
    .unwrap();
    assert_eq!(resumed.value["pending_approvals"], json!([]));
    assert!(
        !resumed.value["output"]["nodes"]["downstream"]["items"].is_null(),
        "downstream should run once the correct gate is named in approvals"
    );

    let reloaded = flows_get(&config, &created.value.id).await.unwrap();
    assert_eq!(reloaded.value.last_status.as_deref(), Some("completed"));
}

#[tokio::test]
async fn flows_resume_denying_a_gate_routes_to_its_error_port() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(
        &config,
        "gated-deny".to_string(),
        approval_gated_graph_with_error_port(),
        false,
    )
    .await
    .unwrap();

    let run = flows_run(
        &config,
        &created.value.id,
        json!({ "x": 1 }),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();

    // Deny the gate: no approvals, `gate` in rejections.
    let resumed = flows_resume(
        &config,
        &created.value.id,
        &thread_id,
        vec![],
        vec!["gate".to_string()],
    )
    .await
    .unwrap();

    assert_eq!(resumed.value["pending_approvals"], json!([]));
    assert_eq!(
        resumed.value["output"]["nodes"]["recover"]["items"][0]["json"]["error"]["node"],
        json!("gate"),
        "a denied gate must route its error item to the `error`-port recovery node"
    );
    assert!(
        resumed.value["output"]["nodes"]["downstream"].is_null(),
        "the main branch must not run when the gate is denied"
    );

    let reloaded = flows_get(&config, &created.value.id).await.unwrap();
    assert_eq!(reloaded.value.last_status.as_deref(), Some("completed"));

    let run_row = flows_get_run(&config, &thread_id).await.unwrap();
    assert_eq!(run_row.value.status, "completed");
    assert!(run_row.value.pending_approvals.is_empty());
}

#[tokio::test]
async fn flows_resume_denying_a_gate_with_no_error_port_fails_the_run() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // `approval_gated_graph()` has only a `main` edge out of the gate — no
    // `error` port to route a denial to, so the whole run must fail.
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();

    let run = flows_run(
        &config,
        &created.value.id,
        json!({ "x": 1 }),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();

    let err = flows_resume(
        &config,
        &created.value.id,
        &thread_id,
        vec![],
        vec!["gate".to_string()],
    )
    .await
    .expect_err("denying a gate with no error port must fail the run");
    assert!(
        err.contains("denied"),
        "expected a denial error, got: {err}"
    );

    let reloaded = flows_get(&config, &created.value.id).await.unwrap();
    assert_eq!(reloaded.value.last_status.as_deref(), Some("failed"));
    let run_row = flows_get_run(&config, &thread_id).await.unwrap();
    assert_eq!(run_row.value.status, "failed");
}

#[tokio::test]
async fn flows_resume_rejects_a_gate_named_in_both_approvals_and_rejections() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();

    let run = flows_run(
        &config,
        &created.value.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();

    let err = flows_resume(
        &config,
        &created.value.id,
        &thread_id,
        vec!["gate".to_string()],
        vec!["gate".to_string()],
    )
    .await
    .expect_err("a gate cannot be both approved and rejected");
    assert!(err.contains("cannot be both approved and rejected"));

    // The run must be untouched (still pending), never half-resumed.
    let run_row = flows_get_run(&config, &thread_id).await.unwrap();
    assert_eq!(run_row.value.status, "pending_approval");
}

#[tokio::test]
async fn flows_resume_of_a_non_paused_run_errors_clearly() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "demo".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();

    // This run completes outright (no approval gate) — its recorded status
    // is "completed", not "pending_approval".
    let run = flows_run(
        &config,
        &created.value.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();

    let err = flows_resume(&config, &created.value.id, &thread_id, vec![], vec![])
        .await
        .expect_err("resuming an already-completed run must be a clear error, not a silent no-op");
    assert!(
        err.contains("not pending approval") || err.contains("no paused run"),
        "expected a clear non-paused-run error, got: {err}"
    );
}
