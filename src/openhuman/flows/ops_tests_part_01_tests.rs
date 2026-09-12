use super::*;

#[test]
fn engine_compatibility_distinguishes_nested_from_safe_fan_ins() {
    let risky = structurally_valid_graph(nested_conditional_fan_in_graph());
    let errors = engine_compatibility_errors(&risky);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN);
    assert_eq!(errors[0].node_id.as_deref(), Some("m"));

    let one_level = structurally_valid_graph(json!({
        "name": "one-level-mixed-fan-in",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Trigger" },
            { "id": "cond", "kind": "condition", "name": "Condition", "config": { "field": "flag" } },
            { "id": "a", "kind": "output_parser", "name": "A" },
            { "id": "other", "kind": "output_parser", "name": "Other" },
            { "id": "c", "kind": "output_parser", "name": "C" },
            { "id": "m", "kind": "merge", "name": "Merge" }
        ],
        "edges": [
            { "from_node": "start", "from_port": "main", "to_node": "cond" },
            { "from_node": "start", "from_port": "main", "to_node": "c" },
            { "from_node": "cond", "from_port": "true", "to_node": "a" },
            { "from_node": "cond", "from_port": "false", "to_node": "other" },
            { "from_node": "a", "from_port": "main", "to_node": "m" },
            { "from_node": "c", "from_port": "main", "to_node": "m" }
        ]
    }));
    assert!(engine_compatibility_errors(&one_level).is_empty());

    let nested_without_fan_in = structurally_valid_graph(json!({
        "name": "nested-without-fan-in",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Trigger" },
            { "id": "outer", "kind": "condition", "name": "Outer", "config": { "field": "outer" } },
            { "id": "inner", "kind": "condition", "name": "Inner", "config": { "field": "inner" } },
            { "id": "outer_else", "kind": "output_parser", "name": "Outer else" },
            { "id": "a", "kind": "output_parser", "name": "A" },
            { "id": "inner_else", "kind": "output_parser", "name": "Inner else" }
        ],
        "edges": [
            { "from_node": "start", "from_port": "main", "to_node": "outer" },
            { "from_node": "outer", "from_port": "true", "to_node": "inner" },
            { "from_node": "outer", "from_port": "false", "to_node": "outer_else" },
            { "from_node": "inner", "from_port": "true", "to_node": "a" },
            { "from_node": "inner", "from_port": "false", "to_node": "inner_else" }
        ]
    }));
    assert!(engine_compatibility_errors(&nested_without_fan_in).is_empty());

    let unconditional = structurally_valid_graph(json!({
        "name": "unconditional-fan-in",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Trigger" },
            { "id": "a", "kind": "output_parser", "name": "A" },
            { "id": "c", "kind": "output_parser", "name": "C" },
            { "id": "m", "kind": "merge", "name": "Merge" }
        ],
        "edges": [
            { "from_node": "start", "from_port": "main", "to_node": "a" },
            { "from_node": "start", "from_port": "main", "to_node": "c" },
            { "from_node": "a", "from_port": "main", "to_node": "m" },
            { "from_node": "c", "from_port": "main", "to_node": "m" }
        ]
    }));
    assert!(engine_compatibility_errors(&unconditional).is_empty());
}

#[test]
fn engine_compatibility_rejects_main_label_on_conditional_fan_in_path() {
    let graph = structurally_valid_graph(main_port_conditional_fan_in_graph());
    let errors = engine_compatibility_errors(&graph);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, UNSUPPORTED_MAIN_PORT_CONDITIONAL_FAN_IN);
    assert_eq!(errors[0].node_id.as_deref(), Some("m"));

    let reconverged = structurally_valid_graph(json!({
        "name": "main-port-reconverges-before-fan-in",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Trigger" },
            { "id": "route", "kind": "switch", "name": "Route", "config": { "field": "kind" } },
            { "id": "a", "kind": "output_parser", "name": "A" },
            { "id": "c", "kind": "output_parser", "name": "C" },
            { "id": "m", "kind": "merge", "name": "Merge" }
        ],
        "edges": [
            { "from_node": "start", "from_port": "main", "to_node": "route" },
            { "from_node": "start", "from_port": "main", "to_node": "c" },
            { "from_node": "route", "from_port": "main", "to_node": "a" },
            { "from_node": "route", "from_port": "default", "to_node": "a" },
            { "from_node": "a", "from_port": "main", "to_node": "m" },
            { "from_node": "c", "from_port": "main", "to_node": "m" }
        ]
    }));
    assert!(engine_compatibility_errors(&reconverged).is_empty());
}

/// A loop head has two incoming edges, and this gate mirrors the engine's
/// fan-in classification — so without excluding back-edges it would report
/// every legal bounded loop as an unrelieved fan-in and refuse to save it.
#[test]
fn engine_compatibility_does_not_treat_a_loop_back_edge_as_a_fan_in() {
    let looping = structurally_valid_graph(json!({
        "name": "bounded-loop",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Trigger" },
            { "id": "l", "kind": "loop", "name": "Loop",
              "config": { "max_iterations": 3, "on_exceeded": "continue" } },
            { "id": "work", "kind": "output_parser", "name": "Work" },
            { "id": "out", "kind": "output_parser", "name": "Out" }
        ],
        "edges": [
            { "from_node": "start", "from_port": "main", "to_node": "l" },
            { "from_node": "l", "from_port": "body", "to_node": "work" },
            { "from_node": "work", "from_port": "main", "to_node": "l" },
            { "from_node": "l", "from_port": "done", "to_node": "out" }
        ]
    }));
    assert!(
        engine_compatibility_errors(&looping).is_empty(),
        "a bounded loop must save cleanly: {:?}",
        engine_compatibility_errors(&looping)
    );
}

#[test]
fn engine_compatibility_requires_exhaustive_router_choices_for_reconvergence() {
    let exhaustive_condition = nested_router_reconvergence_graph("condition", &["true", "false"]);
    assert!(engine_compatibility_errors(&exhaustive_condition).is_empty());

    let missing_condition_branch = nested_router_reconvergence_graph("condition", &["true"]);
    let errors = engine_compatibility_errors(&missing_condition_branch);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN);

    let exhaustive_switch = nested_router_reconvergence_graph("switch", &["known-case", "default"]);
    assert!(engine_compatibility_errors(&exhaustive_switch).is_empty());

    // Same-port fan-out is unconditional: TinyFlows schedules both `main`
    // successors. A side path after an exhaustive router must not make the
    // reconverging path look like another conditional choice.
    let exhaustive_switch_with_main_fanout = structurally_valid_graph(json!({
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Trigger" },
            { "id": "outer", "kind": "condition", "name": "Outer", "config": { "field": "outer" } },
            { "id": "inner", "kind": "switch", "name": "Inner", "config": { "field": "inner" } },
            { "id": "outer_else", "kind": "output_parser", "name": "Outer else" },
            { "id": "fanout", "kind": "output_parser", "name": "Fan out" },
            { "id": "a", "kind": "output_parser", "name": "A" },
            { "id": "side", "kind": "output_parser", "name": "Side" },
            { "id": "c", "kind": "output_parser", "name": "C" },
            { "id": "m", "kind": "merge", "name": "Merge" }
        ],
        "edges": [
            { "from_node": "start", "from_port": "main", "to_node": "outer" },
            { "from_node": "start", "from_port": "main", "to_node": "c" },
            { "from_node": "outer", "from_port": "true", "to_node": "inner" },
            { "from_node": "outer", "from_port": "false", "to_node": "outer_else" },
            { "from_node": "inner", "from_port": "known-case", "to_node": "fanout" },
            { "from_node": "inner", "from_port": "default", "to_node": "fanout" },
            { "from_node": "fanout", "from_port": "main", "to_node": "a" },
            { "from_node": "fanout", "from_port": "main", "to_node": "side" },
            { "from_node": "a", "from_port": "main", "to_node": "m" },
            { "from_node": "c", "from_port": "main", "to_node": "m" }
        ]
    }));
    assert!(engine_compatibility_errors(&exhaustive_switch_with_main_fanout).is_empty());

    // A switch with only `default` is exhaustive: every input takes that edge,
    // so it is an unconditional step even though it has a single wired port.
    let default_only_switch = nested_router_reconvergence_graph("switch", &["default"]);
    assert!(engine_compatibility_errors(&default_only_switch).is_empty());

    let missing_switch_default =
        nested_router_reconvergence_graph("switch", &["known-case", "other-case"]);
    let errors = engine_compatibility_errors(&missing_switch_default);
    assert!(!errors.is_empty());
    assert!(errors
        .iter()
        .all(|error| error.code == UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN));
    // Both the switch's own reconvergence and the downstream merge are unsafe;
    // multiple switch ports may also report the same predecessor. Pin the
    // affected fan-ins without coupling the test to diagnostic multiplicity.
    assert!(errors
        .iter()
        .any(|error| error.node_id.as_deref() == Some("a")));
    assert!(errors
        .iter()
        .any(|error| error.node_id.as_deref() == Some("m")));
}

#[test]
fn engine_compatibility_rejects_reconvergence_before_nested_router() {
    let graph = structurally_valid_graph(json!({
        "name": "reconverged-before-nested-router",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Trigger" },
            { "id": "outer", "kind": "condition", "name": "Outer", "config": { "field": "outer" } },
            { "id": "inner", "kind": "condition", "name": "Inner", "config": { "field": "inner" } },
            { "id": "a", "kind": "output_parser", "name": "A" },
            { "id": "inner_else", "kind": "output_parser", "name": "Inner else" },
            { "id": "c", "kind": "output_parser", "name": "C" },
            { "id": "m", "kind": "merge", "name": "Merge" }
        ],
        "edges": [
            { "from_node": "start", "from_port": "main", "to_node": "outer" },
            { "from_node": "start", "from_port": "main", "to_node": "c" },
            { "from_node": "outer", "from_port": "true", "to_node": "inner" },
            { "from_node": "outer", "from_port": "false", "to_node": "inner" },
            { "from_node": "inner", "from_port": "true", "to_node": "a" },
            { "from_node": "inner", "from_port": "false", "to_node": "inner_else" },
            { "from_node": "a", "from_port": "main", "to_node": "m" },
            { "from_node": "c", "from_port": "main", "to_node": "m" }
        ]
    }));
    let errors = engine_compatibility_errors(&graph);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN);
}

#[test]
fn engine_compatibility_treats_single_wired_router_outputs_as_conditional() {
    let graph = structurally_valid_graph(json!({
        "name": "single-wired-nested-router-fan-in",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Trigger" },
            { "id": "outer", "kind": "switch", "name": "Outer", "config": { "field": "outer" } },
            { "id": "inner", "kind": "condition", "name": "Inner", "config": { "field": "inner" } },
            { "id": "a", "kind": "output_parser", "name": "A" },
            { "id": "c", "kind": "output_parser", "name": "C" },
            { "id": "m", "kind": "merge", "name": "Merge" }
        ],
        "edges": [
            { "from_node": "start", "from_port": "main", "to_node": "outer" },
            { "from_node": "start", "from_port": "main", "to_node": "c" },
            { "from_node": "outer", "from_port": "case", "to_node": "inner" },
            { "from_node": "inner", "from_port": "true", "to_node": "a" },
            { "from_node": "a", "from_port": "main", "to_node": "m" },
            { "from_node": "c", "from_port": "main", "to_node": "m" }
        ]
    }));

    let errors = engine_compatibility_errors(&graph);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN);
    assert_eq!(errors[0].node_id.as_deref(), Some("m"));
}

#[test]
fn engine_compatibility_detects_a_router_directly_preceding_fan_in() {
    let nested = structurally_valid_graph(json!({
        "name": "direct-nested-router-fan-in",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Trigger" },
            { "id": "outer", "kind": "switch", "name": "Outer", "config": { "field": "outer" } },
            { "id": "inner", "kind": "condition", "name": "Inner", "config": { "field": "inner" } },
            { "id": "c", "kind": "output_parser", "name": "C" },
            { "id": "m", "kind": "merge", "name": "Merge" }
        ],
        "edges": [
            { "from_node": "start", "from_port": "main", "to_node": "outer" },
            { "from_node": "start", "from_port": "main", "to_node": "c" },
            { "from_node": "outer", "from_port": "case", "to_node": "inner" },
            { "from_node": "inner", "from_port": "true", "to_node": "m" },
            { "from_node": "c", "from_port": "main", "to_node": "m" }
        ]
    }));
    let errors = engine_compatibility_errors(&nested);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN);

    let main_port = structurally_valid_graph(json!({
        "name": "direct-main-port-router-fan-in",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Trigger" },
            { "id": "route", "kind": "switch", "name": "Route", "config": { "field": "kind" } },
            { "id": "c", "kind": "output_parser", "name": "C" },
            { "id": "m", "kind": "merge", "name": "Merge" }
        ],
        "edges": [
            { "from_node": "start", "from_port": "main", "to_node": "route" },
            { "from_node": "start", "from_port": "main", "to_node": "c" },
            { "from_node": "route", "from_port": "main", "to_node": "m" },
            { "from_node": "c", "from_port": "main", "to_node": "m" }
        ]
    }));
    let errors = engine_compatibility_errors(&main_port);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, UNSUPPORTED_MAIN_PORT_CONDITIONAL_FAN_IN);
}

#[test]
fn engine_compatibility_recurses_through_nested_inline_sub_workflows() {
    let unsafe_child = nested_conditional_fan_in_graph();
    let middle = json!({
        "nodes": [
            { "id": "middle-trigger", "kind": "trigger", "name": "Trigger" },
            {
                "id": "inner-child",
                "kind": "sub_workflow",
                "name": "Inner child",
                "config": { "workflow": unsafe_child }
            }
        ],
        "edges": [
            { "from_node": "middle-trigger", "from_port": "main", "to_node": "inner-child" }
        ]
    });
    let parent = structurally_valid_graph(json!({
        "nodes": [
            { "id": "parent-trigger", "kind": "trigger", "name": "Trigger" },
            {
                "id": "middle-child",
                "kind": "sub_workflow",
                "name": "Middle child",
                "config": { "workflow": middle }
            }
        ],
        "edges": [
            { "from_node": "parent-trigger", "from_port": "main", "to_node": "middle-child" }
        ]
    }));

    let errors = engine_compatibility_errors(&parent);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN);
    assert!(errors[0].message.contains("middle-child"));
    assert!(errors[0].message.contains("inner-child"));
}

#[test]
fn resolver_lookup_rejects_an_incompatible_saved_child() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let child = store::create_flow(
        &config,
        "legacy child".to_string(),
        structurally_valid_graph(nested_conditional_fan_in_graph()),
        false,
        false,
    )
    .unwrap();

    let error = load_engine_compatible_flow_graph(&config, &child.id)
        .expect_err("resolver lookup must reject an unsafe legacy child");
    assert!(
        error.contains(UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN),
        "{error}"
    );
    assert!(error.contains(&child.id), "{error}");
}

#[test]
fn resolver_lookup_rejects_an_incompatible_saved_grandchild() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let grandchild = store::create_flow(
        &config,
        "legacy unsafe grandchild".to_string(),
        structurally_valid_graph(nested_conditional_fan_in_graph()),
        false,
        false,
    )
    .unwrap();
    let child = store::create_flow(
        &config,
        "saved child".to_string(),
        structurally_valid_graph(referenced_child_graph(&grandchild.id)),
        false,
        false,
    )
    .unwrap();

    let error = load_engine_compatible_flow_graph(&config, &child.id)
        .expect_err("resolver lookup must reject an unsafe saved grandchild");
    assert!(
        error.contains(UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN),
        "{error}"
    );
    assert!(error.contains(&child.id), "{error}");
    assert!(error.contains(&grandchild.id), "{error}");
    assert!(error.contains("saved-child"), "{error}");
}

#[test]
fn flows_validate_returns_stable_nested_conditional_fan_in_error() {
    let outcome = flows_validate(nested_conditional_fan_in_graph());
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
    let flow = store::create_flow(&config, "legacy".to_string(), graph, false, true).unwrap();

    let err = flows_run(
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
    .unwrap();
    let parent = store::create_flow(
        &config,
        "parent".to_string(),
        structurally_valid_graph(referenced_child_graph(&child.id)),
        false,
        true,
    )
    .unwrap();

    let error = flows_run(
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
    let flow = store::create_flow(&config, "legacy".to_string(), graph, false, false).unwrap();

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
    .unwrap();

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
    .unwrap();
    let original_graph = structurally_valid_graph(trigger_only_graph());
    let parent = store::create_flow(
        &config,
        "safe parent".to_string(),
        original_graph.clone(),
        false,
        true,
    )
    .unwrap();

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
