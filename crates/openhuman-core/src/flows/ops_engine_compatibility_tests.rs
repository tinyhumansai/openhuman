use super::*;

#[tokio::test]
async fn resolver_lookup_rejects_an_incompatible_saved_child() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let child = store::create_flow(
        &config,
        "legacy child".to_string(),
        structurally_valid_graph(nested_conditional_fan_in_graph()),
        false,
        false,
    )
    .unwrap()
    .workflow;

    let error = load_engine_compatible_flow_graph(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &child.id,
    )
    .await
    .expect_err("resolver lookup must reject an unsafe legacy child");
    assert!(
        error.contains(UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN),
        "{error}"
    );
    assert!(error.contains(&child.id), "{error}");
}

#[tokio::test]
async fn resolver_lookup_rejects_an_incompatible_saved_grandchild() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let grandchild = store::create_flow(
        &config,
        "legacy unsafe grandchild".to_string(),
        structurally_valid_graph(nested_conditional_fan_in_graph()),
        false,
        false,
    )
    .unwrap()
    .workflow;
    let child = store::create_flow(
        &config,
        "saved child".to_string(),
        structurally_valid_graph(referenced_child_graph(&grandchild.id)),
        false,
        false,
    )
    .unwrap()
    .workflow;

    let error = load_engine_compatible_flow_graph(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &child.id,
    )
    .await
    .expect_err("resolver lookup must reject an unsafe saved grandchild");
    assert!(
        error.contains(UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN),
        "{error}"
    );
    assert!(error.contains(&child.id), "{error}");
    assert!(error.contains(&grandchild.id), "{error}");
    assert!(error.contains("saved-child"), "{error}");
}

#[tokio::test]
async fn flows_validate_returns_stable_nested_conditional_fan_in_error() {
    let outcome = flows_validate(
        &crate::core::runtime::DispatchCtx::ambient(),
        &Config::default(),
        nested_conditional_fan_in_graph(),
    )
    .await;
    assert!(!outcome.value.valid);
    assert_eq!(outcome.value.error_details.len(), 1);
    assert_eq!(
        outcome.value.error_details[0].code,
        UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN
    );
    assert_eq!(outcome.value.error_details[0].node_id.as_deref(), Some("m"));
    assert!(outcome.value.warnings.is_empty());
}

#[tokio::test]
async fn flows_run_rejects_legacy_nested_conditional_fan_in_before_execution() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // Bypass the current author-time gate to simulate a definition persisted
    // by an older OpenHuman build. Reads remain supported; execution does not.
    let graph = structurally_valid_graph(nested_conditional_fan_in_graph());
    let flow = store::create_flow(&config, "legacy".to_string(), graph, false, true)
        .unwrap()
        .workflow;

    let err = flows_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &flow.id,
        json!({ "outer": true, "inner": true }),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .expect_err("legacy unsafe topology must fail closed");
    assert!(err.contains(UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN), "{err}");

    let reloaded = flows_get(&config, &flow.id).await.unwrap();
    assert_eq!(reloaded.value.last_status, None);
    assert_eq!(
        reloaded.value.graph, flow.graph,
        "stored graph must be preserved"
    );
}

#[tokio::test]
async fn flows_run_rejects_an_incompatible_saved_child_before_execution() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let child = store::create_flow(
        &config,
        "legacy unsafe child".to_string(),
        structurally_valid_graph(nested_conditional_fan_in_graph()),
        false,
        false,
    )
    .unwrap()
    .workflow;
    let parent = store::create_flow(
        &config,
        "parent".to_string(),
        structurally_valid_graph(referenced_child_graph(&child.id)),
        false,
        true,
    )
    .unwrap()
    .workflow;

    let error = flows_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &parent.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .expect_err("an unsafe saved child must fail before root execution starts");
    assert!(
        error.contains(UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN),
        "{error}"
    );
    assert!(error.contains(&child.id), "{error}");

    let reloaded = flows_get(&config, &parent.id).await.unwrap().value;
    assert_eq!(reloaded.last_status, None, "no run should have started");
}

#[tokio::test]
async fn flows_update_allows_metadata_only_edits_of_legacy_incompatible_graph() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let graph = structurally_valid_graph(nested_conditional_fan_in_graph());
    let flow = store::create_flow(&config, "legacy".to_string(), graph, false, false)
        .unwrap()
        .workflow;

    let updated = flows_update(
        &config,
        &flow.id,
        Some("renamed legacy".to_string()),
        None,
        Some(true),
        None,
    )
    .await
    .expect("metadata-only update should preserve access to a legacy graph");

    assert_eq!(updated.value.name, "renamed legacy");
    assert!(updated.value.require_approval);
    assert_eq!(updated.value.graph, flow.graph);
}

#[tokio::test]
async fn flows_create_rejects_an_incompatible_saved_child_before_persisting() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let child = store::create_flow(
        &config,
        "legacy unsafe child".to_string(),
        structurally_valid_graph(nested_conditional_fan_in_graph()),
        false,
        false,
    )
    .unwrap()
    .workflow;

    let error = flows_create(
        &config,
        "rejected parent".to_string(),
        referenced_child_graph(&child.id),
        false,
    )
    .await
    .expect_err("create must reject an unsafe saved child");

    assert!(
        error.contains(UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN),
        "{error}"
    );
    assert!(error.contains(&child.id), "{error}");
    let (flows, _skipped) = store::list_flows(&config).unwrap();
    assert_eq!(flows.len(), 1, "the rejected parent must not be persisted");
    assert_eq!(flows[0].id, child.id);
}

#[tokio::test]
async fn flows_update_rejects_an_incompatible_saved_child_before_persisting() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let child = store::create_flow(
        &config,
        "legacy unsafe child".to_string(),
        structurally_valid_graph(nested_conditional_fan_in_graph()),
        false,
        false,
    )
    .unwrap()
    .workflow;
    let original_graph = structurally_valid_graph(trigger_only_graph());
    let parent = store::create_flow(
        &config,
        "safe parent".to_string(),
        original_graph.clone(),
        false,
        true,
    )
    .unwrap()
    .workflow;

    let error = flows_update(
        &config,
        &parent.id,
        None,
        Some(referenced_child_graph(&child.id)),
        None,
        None,
    )
    .await
    .expect_err("update must reject an unsafe saved child");

    assert!(
        error.contains(UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN),
        "{error}"
    );
    assert!(error.contains(&child.id), "{error}");
    let reloaded = flows_get(&config, &parent.id).await.unwrap().value;
    assert_eq!(
        reloaded.graph, original_graph,
        "the rejected graph update must not be persisted"
    );
}

#[tokio::test]
async fn flows_create_rejects_graph_without_trigger() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let graph_without_trigger = json!({
        "name": "bad",
        "nodes": [ { "id": "a", "kind": "output_parser", "name": "A" } ],
        "edges": []
    });

    let err = flows_create(&config, "bad".to_string(), graph_without_trigger, false)
        .await
        .expect_err("graph without a trigger must be rejected");
    assert!(
        err.contains("trigger"),
        "expected a MissingTrigger-style error, got: {err}"
    );
}

#[tokio::test]
async fn flows_create_get_list_delete_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(&config, "demo".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();
    let flow_id = created.value.id.clone();

    let fetched = flows_get(&config, &flow_id).await.unwrap();
    assert_eq!(fetched.value.id, flow_id);
    assert_eq!(fetched.value.name, "demo");

    let listed = flows_list(&config).await.unwrap();
    assert_eq!(listed.value.len(), 1);

    flows_delete(&config, &flow_id).await.unwrap();
    assert!(flows_get(&config, &flow_id).await.is_err());
    assert!(flows_list(&config).await.unwrap().value.is_empty());
}

#[tokio::test]
async fn flows_duplicate_produces_disabled_unbound_copy_with_new_id() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Enabled source with require_approval set.
    let created = flows_create(&config, "My Flow".to_string(), trigger_only_graph(), true)
        .await
        .unwrap();
    assert!(created.value.enabled);
    let source_id = created.value.id.clone();

    let dup = flows_duplicate(&config, &source_id).await.unwrap();

    // New id, suffixed name, DISABLED (so no trigger is bound => never fires).
    assert_ne!(dup.value.id, source_id);
    assert_eq!(dup.value.name, "My Flow (copy)");
    assert!(
        !dup.value.enabled,
        "a duplicate must be disabled and thus not schedule/trigger-bound"
    );
    // Identical graph + require_approval carried over; run history reset.
    assert_eq!(dup.value.graph, created.value.graph);
    assert!(dup.value.require_approval);
    assert!(dup.value.last_run_at.is_none());
    assert!(dup.value.last_status.is_none());

    // Both flows now exist independently.
    let listed = flows_list(&config).await.unwrap();
    assert_eq!(listed.value.len(), 2);
}
