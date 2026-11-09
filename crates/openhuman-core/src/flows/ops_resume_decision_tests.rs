use super::*;

// ── TinyFlows-owned resume decision validation ──────────────────────────
//
// Resume decisions are checked against the persisted run's pending gate ids
// before the owner module admits the continuation. Invalid decisions must
// leave the parked row and flow summary untouched.

#[tokio::test]
async fn flows_resume_with_empty_approvals_is_rejected_and_does_not_complete_the_run() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();

    let run = flows_run(
        &crate::core::runtime::DispatchCtx::ambient(),
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
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &created.value.id,
        &thread_id,
        vec![],
        vec![],
    )
    .await
    .expect_err("an empty approvals list must not silently approve the pending gate");
    assert!(
        err.contains("resume decisions do not name a currently pending gate"),
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
        &crate::core::runtime::DispatchCtx::ambient(),
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
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &created.value.id,
        &thread_id,
        vec!["not-a-real-gate".to_string()],
        vec![],
    )
    .await
    .expect_err("approvals naming no actually-pending gate must be rejected");
    assert!(err.contains("resume decisions do not name a currently pending gate"));
}

#[tokio::test]
async fn flows_resume_with_the_correct_gate_completes_and_runs_downstream() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();

    let run = flows_run(
        &crate::core::runtime::DispatchCtx::ambient(),
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
        &crate::core::runtime::DispatchCtx::ambient(),
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
        &crate::core::runtime::DispatchCtx::ambient(),
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
        &crate::core::runtime::DispatchCtx::ambient(),
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
async fn flows_resume_fails_and_persists_the_run_when_denial_has_no_error_port() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // `approval_gated_graph()` has only a `main` edge out of the gate — no
    // `error` port to route a denial to, so the whole run must fail.
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();

    let run = flows_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &created.value.id,
        json!({ "x": 1 }),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();

    let error = flows_resume(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &created.value.id,
        &thread_id,
        vec![],
        vec!["gate".to_string()],
    )
    .await
    .expect_err("denial without an error port must fail the resumed engine run");
    assert!(
        error.contains("approval gate 'gate' was denied and has no `error` port"),
        "{error}"
    );

    let reloaded = flows_get(&config, &created.value.id).await.unwrap();
    assert_eq!(reloaded.value.last_status.as_deref(), Some("failed"));
    let run_row = flows_get_run(&config, &thread_id).await.unwrap();
    assert_eq!(run_row.value.status, "failed");
    assert!(run_row.value.pending_approvals.is_empty());
    assert_eq!(run_row.value.error.as_deref(), Some(error.as_str()));
}

#[tokio::test]
async fn flows_resume_rejects_a_gate_named_in_both_approvals_and_rejections() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();

    let run = flows_run(
        &crate::core::runtime::DispatchCtx::ambient(),
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
        &crate::core::runtime::DispatchCtx::ambient(),
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
        &crate::core::runtime::DispatchCtx::ambient(),
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
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &created.value.id,
        &thread_id,
        vec![],
        vec![],
    )
    .await
    .expect_err("resuming an already-completed run must be a clear error, not a silent no-op");
    let expected = "is not waiting for approval";
    assert!(
        err.contains(expected),
        "expected a clear non-paused-run error, got: {err}"
    );
    let completed = flows_get_run(&config, &thread_id)
        .await
        .expect("read the completed run after refused resume")
        .value;
    assert_eq!(
        completed.status, "completed",
        "refusing a resume must leave the completed run unchanged"
    );
}
