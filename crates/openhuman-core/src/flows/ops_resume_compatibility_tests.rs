use super::*;

#[tokio::test]
async fn flows_resume_marks_an_incompatible_legacy_checkpoint_failed() {
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
    let pending: Vec<String> =
        serde_json::from_value(run.value["pending_approvals"].clone()).unwrap();

    // Model a legacy parked checkpoint whose current saved graph is unsafe.
    // The module owns compatibility refusal and should settle this run as
    // failed when ResumeRun is attempted.
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
    // Keep the compatibility refusal under test distinct from the stale-pin
    // refusal: a pre-pin parked row has no graph hash to compare.
    super::resume_checkpoint_tests::update_parked_run_document_for_test(
        &config,
        &thread_id,
        |document| {
            document
                .as_object_mut()
                .expect("run document is an object")
                .remove("graph_hash");
        },
    );
    let error = flows_resume(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &created.value.id,
        &thread_id,
        pending,
        vec![],
    )
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
    let pending: Vec<String> =
        serde_json::from_value(run.value["pending_approvals"].clone()).unwrap();
    let child = store::create_flow(
        &config,
        "legacy unsafe child".to_string(),
        structurally_valid_graph(nested_conditional_fan_in_graph()),
        false,
        false,
    )
    .unwrap()
    .workflow;
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
    // Keep the compatibility refusal under test distinct from the stale-pin
    // refusal: a pre-pin parked row has no graph hash to compare.
    super::resume_checkpoint_tests::update_parked_run_document_for_test(
        &config,
        &thread_id,
        |document| {
            document
                .as_object_mut()
                .expect("run document is an object")
                .remove("graph_hash");
        },
    );
    let error = flows_resume(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &created.value.id,
        &thread_id,
        pending,
        vec![],
    )
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
    let err = flows_resume(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "missing",
        "thread-1",
        vec![],
        vec![],
    )
    .await
    .expect_err("must error");
    assert!(err.contains("not found"));
}
