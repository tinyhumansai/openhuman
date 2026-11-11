use super::*;

// Regression tests for #8489: a builder save that writes a NEW graph onto a
// flow that is ALREADY enabled with an automatic trigger must not push the
// new graph live with no review. The save goes through, but the flow is
// auto-disabled in the same write — and its schedule cron job is torn down,
// exactly like an explicit `flows_set_enabled(false)` — so the revision
// can't go live before the user re-enables it via `flows_set_enabled`
// after reviewing.

#[tokio::test]
async fn save_workflow_auto_disarms_save_onto_enabled_automatic_flow() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow_id = seed_flow(&config, "Scheduled flow").await;

    // Build the enabled+automatic pre-save state: saving a schedule graph
    // onto the seeded manual flow disarms it (the #4889 manual→automatic
    // rule), so re-enable it — now the flow is armed on its schedule.
    let tool = save_workflow_tool(config.clone());
    let first = tool
        .execute(json!({ "flow_id": flow_id, "graph": schedule_trigger_graph() }))
        .await
        .unwrap();
    assert!(!first.is_error, "{}", first.output());
    ops::flows_set_enabled(&config, &flow_id, true)
        .await
        .unwrap();
    let armed = ops::flows_get(&config, &flow_id).await.unwrap().value;
    assert!(
        armed.enabled,
        "precondition: the flow is enabled with an automatic trigger"
    );
    assert!(
        crate::cron::find_flow_schedule_job(&config, &flow_id)
            .unwrap()
            .is_some(),
        "precondition: the schedule cron job is registered while the flow is enabled"
    );

    // Now save a REVISED automatic graph (new cron). This is the #8489
    // case: without the fix it stays enabled and goes live immediately.
    let revised = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger",
              "config": { "trigger_kind": "schedule", "schedule": "0 9 * * 1,4" } }
        ],
        "edges": []
    });
    let result = tool
        .execute(json!({ "flow_id": flow_id, "graph": revised }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());

    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(
        parsed["enabled"],
        json!(false),
        "saving a revised graph onto an enabled automatic-trigger flow must \
         auto-disable it: {parsed}"
    );
    let warnings = parsed["warnings"]
        .as_array()
        .expect("warnings must be an array");
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap_or("").contains("auto-disabled")),
        "save_workflow must warn that the flow was auto-disabled pending review, got: {parsed}"
    );

    // Persisted, not just returned in-memory — including the revised graph
    // and the torn-down schedule binding.
    let reloaded = ops::flows_get(&config, &flow_id).await.unwrap().value;
    assert!(!reloaded.enabled);
    assert_eq!(
        reloaded.graph["nodes"][0]["config"]["schedule"],
        "0 9 * * 1,4"
    );
    assert!(
        crate::cron::find_flow_schedule_job(&config, &flow_id)
            .unwrap()
            .is_none(),
        "auto-disarm must remove the schedule cron job, like an explicit disable"
    );
}

#[tokio::test]
async fn save_workflow_onto_disabled_automatic_flow_stays_quiet() {
    // The #8489 disarm is a no-op when there is nothing live to protect: a
    // builder save onto an already-DISABLED automatic-trigger flow must
    // persist the graph and stay silent about enablement.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow_id = seed_flow(&config, "Scheduled flow").await;

    let tool = save_workflow_tool(config.clone());
    let first = tool
        .execute(json!({ "flow_id": flow_id, "graph": schedule_trigger_graph() }))
        .await
        .unwrap();
    assert!(!first.is_error, "{}", first.output());
    let disarmed = ops::flows_get(&config, &flow_id).await.unwrap().value;
    assert!(
        !disarmed.enabled,
        "precondition: the manual→automatic save disarmed the flow"
    );

    let revised = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger",
              "config": { "trigger_kind": "schedule", "schedule": "0 9 * * 1,4" } }
        ],
        "edges": []
    });
    let result = tool
        .execute(json!({ "flow_id": flow_id, "graph": revised }))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.output());

    let parsed: Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(parsed["enabled"], json!(false), "{parsed}");
    let warnings = parsed["warnings"]
        .as_array()
        .expect("warnings must be an array");
    assert!(
        !warnings
            .iter()
            .any(|w| w.as_str().unwrap_or("").contains("auto-disabled")),
        "no disarm warning when the flow was already disabled, got: {parsed}"
    );
    let reloaded = ops::flows_get(&config, &flow_id).await.unwrap().value;
    assert_eq!(
        reloaded.graph["nodes"][0]["config"]["schedule"],
        "0 9 * * 1,4"
    );
}
