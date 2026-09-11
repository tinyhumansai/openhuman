use super::*;

/// Regression for issue #4868: `flows_discover`'s `flow_discovery` agent must
/// also resolve to its definition's effective cap (50, `iteration_policy =
/// "extended"`), not the global default of 10. Before the systemic fix, this
/// call site had NO override at all (unlike `flows_build`'s now-deleted
/// `apply_builder_iteration_cap`), so it silently got the global 10 in
/// production.
#[tokio::test]
async fn flows_discover_applies_the_flow_discovery_definitions_effective_iteration_cap() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert_eq!(config.agent.max_tool_iterations, 10);

    // Building an agent constructs a memory client, which needs the host seams
    // wired. `Once`-guarded, so this is free when another test got there first.
    crate::openhuman::agent::harness::AgentDefinitionRegistry::init_global(&config.workspace_dir)
        .expect("agent registry init");
    let def = crate::openhuman::agent::harness::AgentDefinitionRegistry::global()
        .expect("registry initialised")
        .get("flow_discovery")
        .expect("flow_discovery definition registered")
        .clone();
    let expected = def.effective_max_iterations();
    assert_eq!(expected, 50);

    let agent = crate::openhuman::agent::Agent::from_config_for_agent(&config, "flow_discovery")
        .expect("build flow_discovery agent");
    assert_eq!(agent.agent_config().max_tool_iterations, expected);
}

#[test]
fn validate_and_migrate_graph_rejects_condition_edges_with_branch_label_on_to_port() {
    // The exact malformed shape the workflow_builder agent produced live
    // (see issue B23): both edges share `from_port: "main"` with the branch
    // label on `to_port` instead. The engine routes exclusively on
    // `from_port` (B24, `tinyflows::validate`), so this must be a hard
    // reject here — never persisted as a silently-broken no-op condition.
    let bad_graph = condition_graph("main", "true", "main", "false");

    let err = validate_and_migrate_graph(bad_graph)
        .expect_err("condition edges with the branch label on to_port must be rejected");
    assert!(
        err.contains("condition") && err.contains("from_port"),
        "expected an InvalidConditionRouting-style error naming from_port, got: {err}"
    );
}

#[test]
fn validate_and_migrate_graph_accepts_condition_edges_with_branch_label_on_from_port() {
    // The correct shape: `from_port` carries "true"/"false", `to_port` stays
    // "main".
    let good_graph = condition_graph("true", "main", "false", "main");

    validate_and_migrate_graph(good_graph)
        .expect("correctly-routed condition graph (branch label on from_port) must validate");
}

#[tokio::test]
async fn flows_create_rejects_condition_edges_with_branch_label_on_to_port() {
    // The same hard gate applies at the actual persistence path
    // (`flows_create`), not just the standalone validate helper — a graph
    // with this shape must never reach the store.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let bad_graph = condition_graph("main", "true", "main", "false");
    let err = flows_create(&config, "bad-condition".to_string(), bad_graph, false)
        .await
        .expect_err("flows_create must reject a condition graph routed on to_port");
    assert!(
        err.contains("condition") && err.contains("from_port"),
        "expected an InvalidConditionRouting-style error, got: {err}"
    );
}

#[tokio::test]
async fn flows_create_schedule_trigger_creates_disabled() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(
        &config,
        "scheduled".to_string(),
        schedule_trigger_graph("30 7 * * 1-5"),
        false,
    )
    .await
    .unwrap();

    assert!(
        !created.value.enabled,
        "a schedule-trigger flow must create disabled"
    );
    assert!(
        crate::openhuman::cron::find_flow_schedule_job(&config, &created.value.id)
            .unwrap()
            .is_none(),
        "no cron job may be bound for a disabled-on-create schedule flow"
    );
    assert!(
        created
            .logs
            .iter()
            .any(|l| l.starts_with("Flow created DISABLED")),
        "flows_create must loudly log the disabled-on-create decision: {:?}",
        created.logs
    );
}

#[tokio::test]
async fn flows_create_app_event_trigger_creates_disabled() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(
        &config,
        "app-event".to_string(),
        app_event_trigger_graph(),
        false,
    )
    .await
    .unwrap();

    assert!(
        !created.value.enabled,
        "an app_event-trigger flow must create disabled"
    );
}

#[tokio::test]
async fn flows_create_manual_trigger_creates_enabled() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(&config, "manual".to_string(), manual_trigger_graph(), false)
        .await
        .unwrap();

    assert!(
        created.value.enabled,
        "a manual-trigger flow only ever fires via explicit flows_run — it must create enabled"
    );
}

#[tokio::test]
async fn flows_create_no_trigger_kind_creates_enabled() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(&config, "legacy".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();

    assert!(
        created.value.enabled,
        "a trigger with no trigger_kind discriminator never self-fires — not a surprise, must \
         create enabled"
    );
}

#[tokio::test]
async fn flows_create_outbound_node_forces_require_approval() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(&config, "tool-flow".to_string(), tool_call_graph(), false)
        .await
        .unwrap();

    assert!(
        created.value.require_approval,
        "a graph with a tool_call node must force require_approval, even though the caller \
         passed false"
    );
    assert!(
        created
            .logs
            .iter()
            .any(|l| l.contains("require_approval forced to true")),
        "flows_create must loudly log the forced require_approval: {:?}",
        created.logs
    );
}

#[tokio::test]
async fn flows_create_outbound_http_forces_require_approval() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(
        &config,
        "http-flow".to_string(),
        http_request_graph(),
        false,
    )
    .await
    .unwrap();

    assert!(
        created.value.require_approval,
        "a graph with an http_request node must force require_approval"
    );
}

#[tokio::test]
async fn flows_create_outbound_code_forces_require_approval() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(&config, "code-flow".to_string(), code_graph(), false)
        .await
        .unwrap();

    assert!(
        created.value.require_approval,
        "a graph with a code node must force require_approval"
    );
}

#[tokio::test]
async fn flows_create_readonly_graph_respects_caller_require_approval() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(
        &config,
        "readonly-flow".to_string(),
        readonly_graph(),
        false,
    )
    .await
    .unwrap();

    assert!(
        !created.value.require_approval,
        "a read-only graph (no tool_call/http_request/code) must not have require_approval \
         forced — the caller's choice stands"
    );
}

#[tokio::test]
async fn flows_create_schedule_outbound_creates_disabled_and_approval() {
    // The exact bug scenario from the ticket: a scheduled flow that posts to
    // Slack, saved with `require_approval: false` — it must come back BOTH
    // disabled AND with require_approval forced true.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let graph = json!({
        "name": "scheduled-slack-post",
        "nodes": [
            {
                "id": "t",
                "kind": "trigger",
                "name": "Trigger",
                "config": { "trigger_kind": "schedule", "schedule": "30 7 * * 1-5" }
            },
            {
                "id": "post",
                "kind": "tool_call",
                "name": "Post",
                "config": { "slug": "SLACK_SEND_MESSAGE", "args": { "channel": "general" } }
            }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    });

    let created = flows_create(&config, "scheduled-slack".to_string(), graph, false)
        .await
        .unwrap();

    assert!(
        !created.value.enabled,
        "a scheduled flow with an outbound node must still create disabled (Rule 1)"
    );
    assert!(
        created.value.require_approval,
        "a scheduled flow with an outbound node must force require_approval (Rule 2)"
    );
}

#[tokio::test]
async fn flows_update_forces_require_approval_when_adding_side_effect_nodes() {
    // Compound bypass fix, half 2: `flows_create`'s Rule 2 (force
    // require_approval when the graph gains an outbound side-effect node)
    // must also re-apply on `flows_update` — a flow that starts read-only and
    // is later edited to add a Composio/http_request/code node must not be
    // able to keep require_approval=false just because the update path never
    // re-checked.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "demo".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();
    assert!(
        !created.value.require_approval,
        "a trigger-only graph must not force require_approval on create"
    );

    let updated = flows_update(
        &config,
        &created.value.id,
        None,
        Some(tool_call_graph()),
        Some(false),
        None,
    )
    .await
    .unwrap();

    assert!(
        updated.value.require_approval,
        "flows_update must force require_approval when the replacement graph adds an outbound \
         side-effect node (tool_call), even though the caller passed false"
    );
    assert!(
        updated
            .logs
            .iter()
            .any(|l| l.contains("require_approval forced to true")),
        "flows_update must loudly log the forced require_approval: {:?}",
        updated.logs
    );
}

#[tokio::test]
async fn flows_update_does_not_force_require_approval_on_readonly_graph() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "demo".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();
    assert!(!created.value.require_approval);

    // Name-only update — no graph change, no side-effect nodes.
    let updated = flows_update(
        &config,
        &created.value.id,
        Some("renamed".to_string()),
        None,
        None,
        None,
    )
    .await
    .unwrap();

    assert!(
        !updated.value.require_approval,
        "a name-only update to a read-only graph must not force require_approval"
    );
}

// ── graph_has_outbound_side_effect / trigger_is_automatic helper tests ────

#[test]
fn graph_has_outbound_side_effect_detects_tool_call() {
    let g = graph(tool_call_graph());
    assert!(graph_has_outbound_side_effect(&g));
}

#[test]
fn graph_has_outbound_side_effect_detects_http_request() {
    let g = graph(http_request_graph());
    assert!(graph_has_outbound_side_effect(&g));
}

#[test]
fn graph_has_outbound_side_effect_detects_code() {
    let g = graph(code_graph());
    assert!(graph_has_outbound_side_effect(&g));
}

#[test]
fn graph_has_outbound_side_effect_false_for_agent_only() {
    let g = graph(readonly_graph());
    assert!(!graph_has_outbound_side_effect(&g));
}

#[test]
fn trigger_is_automatic_schedule() {
    let g = graph(schedule_trigger_graph("0 9 * * *"));
    assert!(trigger_is_automatic(&g));
}

#[test]
fn trigger_is_automatic_manual() {
    let g = graph(manual_trigger_graph());
    assert!(!trigger_is_automatic(&g));
}

#[test]
fn trigger_is_automatic_no_trigger_kind() {
    let g = graph(trigger_only_graph());
    assert!(!trigger_is_automatic(&g));
}

#[tokio::test]
async fn strict_gate_passes_a_valid_graph_and_rejects_a_structurally_invalid_one() {
    let config = Config::default();
    // A trigger-only graph is structurally valid and has no outbound gates.
    assert!(strict_gate(&config, &trigger_only_graph()).await.is_ok());

    // No trigger → structural failure surfaced by strict mode.
    let bad = json!({
        "nodes": [ { "id": "a", "kind": "output_parser", "name": "A" } ],
        "edges": []
    });
    let err = strict_gate(&config, &bad).await.unwrap_err();
    assert!(err.contains("structurally invalid"), "{err}");
    assert!(err.contains("trigger"), "{err}");

    // A structurally valid graph must still pass the shared engine gate.
    let err = strict_gate(&config, &nested_conditional_fan_in_graph())
        .await
        .unwrap_err();
    assert!(err.contains(UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN), "{err}");
}

#[tokio::test]
async fn strict_gate_rejects_an_incompatible_saved_child_reference() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let child = store::create_flow(
        &config,
        "legacy unsafe child".to_string(),
        structurally_valid_graph(nested_conditional_fan_in_graph()),
        false,
        false,
    )
    .unwrap();

    let error = strict_gate(&config, &referenced_child_graph(&child.id))
        .await
        .expect_err("strict authoring must reject an incompatible saved child");
    assert!(
        error.contains(UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN),
        "{error}"
    );
    assert!(error.contains(&child.id), "{error}");
    assert!(error.contains("saved-child"), "{error}");
}

#[tokio::test]
async fn builder_proposal_rejects_an_incompatible_saved_child_reference() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let child = store::create_flow(
        &config,
        "legacy unsafe child".to_string(),
        structurally_valid_graph(nested_conditional_fan_in_graph()),
        false,
        false,
    )
    .unwrap();
    let parent = structurally_valid_graph(referenced_child_graph(&child.id));

    let error = build_builder_proposal(
        &config,
        "propose_workflow",
        "parent",
        &parent,
        false,
        false,
        None,
        None,
        None,
    )
    .await
    .expect_err("a proposal must reject an incompatible saved child");
    assert!(
        error.contains(UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN),
        "{error}"
    );
    assert!(error.contains(&child.id), "{error}");
    assert!(error.contains("saved-child"), "{error}");
}

#[test]
fn referenced_child_compatibility_stops_at_saved_workflow_cycles() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow_a = store::create_flow(
        &config,
        "cycle a".to_string(),
        structurally_valid_graph(trigger_only_graph()),
        false,
        false,
    )
    .unwrap();
    let flow_b = store::create_flow(
        &config,
        "cycle b".to_string(),
        structurally_valid_graph(trigger_only_graph()),
        false,
        false,
    )
    .unwrap();
    store::update_flow_graph(
        &config,
        &flow_a.id,
        flow_a.name.clone(),
        structurally_valid_graph(referenced_child_graph(&flow_b.id)),
        false,
        None,
        false,
        None,
    )
    .unwrap();
    store::update_flow_graph(
        &config,
        &flow_b.id,
        flow_b.name.clone(),
        structurally_valid_graph(referenced_child_graph(&flow_a.id)),
        false,
        None,
        false,
        None,
    )
    .unwrap();

    let candidate = structurally_valid_graph(referenced_child_graph(&flow_a.id));
    assert!(referenced_workflow_compatibility_errors(&config, &candidate).is_empty());
}

// ── core-managed drafts (F5) ─────────────────────────────────────────────────

#[tokio::test]
async fn draft_promote_creates_a_new_flow_and_removes_the_draft() {
    use crate::openhuman::flows::DraftOrigin;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let draft = flows_draft_create(
        &config,
        None,
        "From draft".to_string(),
        trigger_only_graph(),
        DraftOrigin::Chat,
    )
    .unwrap()
    .value;

    let flow = flows_draft_promote(&config, &draft.id, None)
        .await
        .unwrap()
        .value;
    assert_eq!(flow.name, "From draft");
    // The draft file is gone once promoted.
    assert!(flows_draft_get(&config, &draft.id).is_err());
    // The flow really exists.
    assert!(flows_get(&config, &flow.id).await.is_ok());
}

#[tokio::test]
async fn draft_promote_with_flow_id_updates_the_existing_flow() {
    use crate::openhuman::flows::DraftOrigin;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let flow = flows_create(&config, "Original".to_string(), trigger_only_graph(), false)
        .await
        .unwrap()
        .value;

    let draft = flows_draft_create(
        &config,
        Some(flow.id.clone()),
        "Renamed via draft".to_string(),
        trigger_only_graph(),
        DraftOrigin::Canvas,
    )
    .unwrap()
    .value;

    let updated = flows_draft_promote(&config, &draft.id, None)
        .await
        .unwrap()
        .value;
    assert_eq!(updated.id, flow.id, "same flow, not a new one");
    assert_eq!(updated.name, "Renamed via draft");
    assert!(
        flows_draft_get(&config, &draft.id).is_err(),
        "draft removed"
    );
}

#[tokio::test]
async fn draft_promote_of_invalid_graph_is_rejected_and_keeps_the_draft() {
    use crate::openhuman::flows::DraftOrigin;
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // A graph with no trigger fails the create gate.
    let bad = json!({
        "nodes": [ { "id": "a", "kind": "output_parser", "name": "A" } ],
        "edges": []
    });
    let draft = flows_draft_create(&config, None, "Bad".to_string(), bad, DraftOrigin::Chat)
        .unwrap()
        .value;

    assert!(flows_draft_promote(&config, &draft.id, None).await.is_err());
    // The draft survives a failed promote so the user can fix it.
    assert!(flows_draft_get(&config, &draft.id).is_ok());
}
