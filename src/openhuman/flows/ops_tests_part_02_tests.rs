use super::*;

#[tokio::test]
async fn flows_duplicate_missing_flow_errors() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = flows_duplicate(&config, "missing").await.unwrap_err();
    assert!(err.contains("not found"));
}

#[tokio::test]
async fn flows_set_enabled_toggles() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "demo".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();
    assert!(created.value.enabled);

    let disabled = flows_set_enabled(&config, &created.value.id, false)
        .await
        .unwrap();
    assert!(!disabled.value.enabled);

    let enabled = flows_set_enabled(&config, &created.value.id, true)
        .await
        .unwrap();
    assert!(enabled.value.enabled);
}

#[tokio::test]
async fn flows_update_replaces_name_and_graph() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "demo".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();

    let mut new_graph = trigger_only_graph();
    new_graph["name"] = json!("renamed-graph");

    let updated = flows_update(
        &config,
        &created.value.id,
        Some("renamed".to_string()),
        Some(new_graph),
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(updated.value.name, "renamed");
    assert_eq!(updated.value.graph.name, "renamed-graph");
}

#[tokio::test]
async fn flows_update_can_set_require_approval() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "demo".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();
    assert!(!created.value.require_approval);

    let updated = flows_update(&config, &created.value.id, None, None, Some(true), None)
        .await
        .unwrap();
    assert!(updated.value.require_approval);

    // Omitting `require_approval` on a later update preserves the current value.
    let unchanged = flows_update(&config, &created.value.id, None, None, None, None)
        .await
        .unwrap();
    assert!(unchanged.value.require_approval);
}

#[tokio::test]
async fn flows_update_rejects_invalid_replacement_graph() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "demo".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();

    let invalid_graph = json!({
        "name": "no-trigger",
        "nodes": [ { "id": "a", "kind": "output_parser", "name": "A" } ],
        "edges": []
    });

    let err = flows_update(
        &config,
        &created.value.id,
        None,
        Some(invalid_graph),
        None,
        None,
    )
    .await
    .expect_err("invalid replacement graph must be rejected");
    assert!(err.contains("trigger"));
}

#[tokio::test]
async fn flows_run_completes_trigger_only_graph() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "demo".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();

    let outcome = flows_run(
        &config,
        &created.value.id,
        json!({ "hello": "world" }),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();

    assert_eq!(outcome.value["pending_approvals"], json!([]));
    assert_eq!(
        outcome.value["output"]["run"]["trigger"],
        json!({ "hello": "world" })
    );

    let reloaded = flows_get(&config, &created.value.id).await.unwrap();
    assert_eq!(reloaded.value.last_status.as_deref(), Some("completed"));
    assert!(reloaded.value.last_run_at.is_some());
}

/// Live finding: a trigger-only graph (no downstream action nodes at all)
/// used to report `status="completed" pending_approvals=0` from `flows_run`
/// completely indistinguishably from a run that actually did something —
/// "triggered but nothing happened" read as a plain success. This asserts
/// the run still completes (running an empty flow isn't an error), but now
/// carries a human-readable `note` in the result so the UI can show
/// "nothing to run" instead of a bare "completed".
#[tokio::test]
async fn flows_run_on_trigger_only_graph_surfaces_no_actionable_nodes_note() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "empty".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();

    let outcome = flows_run(
        &config,
        &created.value.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();

    let note = outcome.value["note"]
        .as_str()
        .expect("trigger-only run must carry a human-readable 'note' field");
    assert!(
        note.contains("no actionable nodes") || note.to_lowercase().contains("nothing"),
        "note should explain that nothing ran, got: {note}"
    );
    assert!(
        outcome.logs.iter().any(|l| l.contains("no actionable")),
        "the note should also surface via the RpcOutcome logs, got: {:?}",
        outcome.logs
    );

    // Still a completed run, not an error — an empty flow isn't a failure,
    // just a no-op that must not masquerade as having done real work.
    let reloaded = flows_get(&config, &created.value.id).await.unwrap();
    assert_eq!(reloaded.value.last_status.as_deref(), Some("completed"));
}

/// A graph with a real downstream node, wired up by an edge, must NOT carry
/// the "nothing to run" note — only a graph with no actionable nodes at all.
/// Uses `output_parser` nodes (like the approval-gated fixture above) rather
/// than an `agent`/`tool_call` node so the run completes deterministically
/// without needing a configured LLM provider or network access.
#[tokio::test]
async fn flows_run_on_graph_with_actionable_nodes_has_no_empty_flow_note() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let graph = json!({
        "name": "has-work",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "downstream", "kind": "output_parser", "name": "Downstream" }
        ],
        "edges": [
            { "from_node": "t", "to_node": "downstream" }
        ]
    });
    let created = flows_create(&config, "has-work".to_string(), graph, false)
        .await
        .unwrap();

    let outcome = flows_run(
        &config,
        &created.value.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();

    assert!(
        outcome.value.get("note").is_none(),
        "a graph with real downstream nodes must not get the empty-flow note, got: {:?}",
        outcome.value.get("note")
    );
}

/// `graph_has_actionable_nodes` must walk from the trigger, not merely check
/// "any non-trigger node plus any edge". A component with edges of its own,
/// but no path back to the trigger, is unreachable and must still surface
/// the "nothing to run" note — a naive count-based check would have missed
/// this and wrongly suppressed the note.
#[tokio::test]
async fn flows_run_on_graph_with_disconnected_component_still_surfaces_empty_flow_note() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let graph = json!({
        "name": "disconnected",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "a", "kind": "output_parser", "name": "Orphan A" },
            { "id": "b", "kind": "output_parser", "name": "Orphan B" }
        ],
        "edges": [
            // "a" -> "b" is wired up, but neither is reachable from "t" — the
            // trigger has no outgoing edges at all.
            { "from_node": "a", "to_node": "b" }
        ]
    });
    let created = flows_create(&config, "disconnected".to_string(), graph, false)
        .await
        .unwrap();

    let outcome = flows_run(
        &config,
        &created.value.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();

    let note = outcome.value["note"]
        .as_str()
        .expect("a component disconnected from the trigger must still surface the empty-flow note");
    assert!(
        note.contains("no actionable nodes") || note.to_lowercase().contains("nothing"),
        "note should explain that nothing ran, got: {note}"
    );
}

#[tokio::test]
async fn flows_run_reports_pending_approval_and_blocks_downstream() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let graph = json!({
        "name": "approval-gated",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "gate", "kind": "output_parser", "name": "Gate", "config": { "requires_approval": true } },
            { "id": "downstream", "kind": "output_parser", "name": "Downstream" }
        ],
        "edges": [
            { "from_node": "t", "to_node": "gate" },
            { "from_node": "gate", "to_node": "downstream" }
        ]
    });

    let created = flows_create(&config, "gated".to_string(), graph, false)
        .await
        .unwrap();

    let outcome = flows_run(
        &config,
        &created.value.id,
        json!({ "x": 1 }),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();

    let pending = outcome.value["pending_approvals"].as_array().unwrap();
    assert!(pending.iter().any(|v| v == "gate"));
    assert!(outcome.value["output"]["nodes"]["downstream"].is_null());

    let reloaded = flows_get(&config, &created.value.id).await.unwrap();
    assert_eq!(
        reloaded.value.last_status.as_deref(),
        Some("pending_approval")
    );
}

#[tokio::test]
async fn flows_get_missing_flow_errors() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = flows_get(&config, "missing").await.expect_err("must error");
    assert!(err.contains("not found"));
}

#[tokio::test]
async fn flows_run_missing_flow_errors() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = flows_run(
        &config,
        "missing",
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .expect_err("must error");
    assert!(err.contains("not found"));
}

#[tokio::test]
async fn flows_run_threads_declared_inputs_into_the_run() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(
        &config,
        "parameterized".to_string(),
        parameterized_graph(),
        false,
    )
    .await
    .unwrap();

    let run = flows_run(
        &config,
        &created.value.id,
        json!({}),
        input_values(&[("repo", json!("acme/api"))]),
        FlowRunTrigger::Rpc,
    )
    .await
    .expect("a run supplying its required input must succeed");

    let output = &run.value["output"];
    assert_eq!(
        output["run"]["inputs"]["repo"],
        json!("acme/api"),
        "the supplied value must reach run.inputs"
    );
    assert_eq!(
        output["run"]["inputs"]["depth"],
        json!(3),
        "the declared default must be applied"
    );
    assert_eq!(
        output["nodes"]["shape"]["items"][0]["json"]["repo"],
        json!("acme/api"),
        "the node's `=inputs.repo` binding must resolve"
    );
}

#[tokio::test]
async fn flows_run_detached_threads_and_validates_declared_inputs_too() {
    // `run_detached` is the entry point both UI Run controls call, so a flow
    // with a required input is only runnable from the UI through here — it must
    // enforce the same contract as the blocking path, synchronously, before it
    // reports a run id the caller will go on to poll.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(
        &config,
        "parameterized".to_string(),
        parameterized_graph(),
        false,
    )
    .await
    .unwrap();

    let err = flows_run_detached(
        &config,
        &created.value.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .expect_err("a missing required input must be refused before a run id is handed out");
    assert!(err.contains("repo"), "got: {err}");

    let started = flows_run_detached(
        &config,
        &created.value.id,
        json!({}),
        input_values(&[("repo", json!("acme/api"))]),
        FlowRunTrigger::Rpc,
    )
    .await
    .expect("a run supplying its required input must start");
    assert_eq!(started.value["status"], "running");
}

#[tokio::test]
async fn flows_run_rejects_a_missing_required_input_without_creating_a_run_row() {
    // The whole point of resolving in `prepare_flow_run`: a caller that gets
    // this error can be certain nothing was started and nothing was recorded.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(
        &config,
        "parameterized".to_string(),
        parameterized_graph(),
        false,
    )
    .await
    .unwrap();

    let err = flows_run(
        &config,
        &created.value.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .expect_err("a missing required input must fail the call");
    assert!(
        err.contains("repo"),
        "the error must name the offending input, got: {err}"
    );

    let runs = flows_list_runs(&config, &created.value.id, 10)
        .await
        .unwrap();
    assert!(
        runs.value.is_empty(),
        "a rejected call must leave no run row behind, got {:?}",
        runs.value
    );
    let reloaded = flows_get(&config, &created.value.id).await.unwrap();
    assert!(
        reloaded.value.last_run_at.is_none(),
        "a rejected call must not stamp last_run_at"
    );
}

#[tokio::test]
async fn flows_run_rejects_a_wrongly_typed_or_undeclared_input() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(
        &config,
        "parameterized".to_string(),
        parameterized_graph(),
        false,
    )
    .await
    .unwrap();

    let type_err = flows_run(
        &config,
        &created.value.id,
        json!({}),
        input_values(&[("repo", json!("acme/api")), ("depth", json!("3"))]),
        FlowRunTrigger::Rpc,
    )
    .await
    .expect_err("a string for a number input must be rejected");
    assert!(type_err.contains("depth"), "got: {type_err}");

    let unknown_err = flows_run(
        &config,
        &created.value.id,
        json!({}),
        input_values(&[("repo", json!("acme/api")), ("reop", json!("typo"))]),
        FlowRunTrigger::Rpc,
    )
    .await
    .expect_err("an undeclared key must be rejected rather than dropped");
    assert!(unknown_err.contains("reop"), "got: {unknown_err}");
}

#[tokio::test]
async fn flows_run_leaves_a_flow_declaring_no_inputs_unchanged() {
    // The pre-existing call shape — empty `inputs` against a graph that
    // declares none — must behave exactly as before.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let graph = json!({
        "name": "plain",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "shape", "kind": "transform", "name": "Shape",
              "config": { "set": { "seen": "=run.trigger.hi" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "shape" } ]
    });
    let created = flows_create(&config, "plain".to_string(), graph, false)
        .await
        .unwrap();

    let run = flows_run(
        &config,
        &created.value.id,
        json!({ "hi": 1 }),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .expect("run");
    assert_eq!(
        run.value["output"]["nodes"]["shape"]["items"][0]["json"]["seen"],
        json!(1)
    );
}

#[tokio::test]
async fn flows_run_records_failed_status_when_a_node_errors() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // A `tool_call` with no `slug` errors in the node executor before reaching
    // any external service; with the default `on_error: stop` the whole run
    // fails deterministically — no network/credentials needed.
    let graph = json!({
        "name": "boom",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "x", "kind": "tool_call", "name": "X" }
        ],
        "edges": [ { "from_node": "t", "to_node": "x" } ]
    });

    let created = flows_create(&config, "boom".to_string(), graph, false)
        .await
        .unwrap();

    let err = flows_run(
        &config,
        &created.value.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .expect_err("a run whose node errors under on_error:stop must fail");
    assert!(!err.is_empty());

    // The failed attempt must be recorded, not left on the prior state.
    let reloaded = flows_get(&config, &created.value.id).await.unwrap();
    assert_eq!(
        reloaded.value.last_status.as_deref(),
        Some("failed"),
        "a failed run must record last_status=failed"
    );
    assert!(
        reloaded.value.last_run_at.is_some(),
        "a failed run must stamp last_run_at"
    );
}

#[tokio::test]
async fn flows_run_populates_error_when_a_continue_policy_node_errors() {
    // Unlike the default `on_error: stop` (previous test), `"continue"` turns
    // the node failure into data on the default port instead of failing the
    // run future — the run settles `Ok`, but the errored step still degrades
    // the terminal status to `"failed"` via `degrade_completed_status`. That
    // path must still populate `FlowRun.error` (its doc contract: "Error
    // message when status == \"failed\"") even though the engine's
    // `ExecutionStep` carries no message of its own for this case.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let graph = json!({
        "name": "boom-continue",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "x", "kind": "tool_call", "name": "X", "config": { "on_error": "continue" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "x" } ]
    });

    let created = flows_create(&config, "boom-continue".to_string(), graph, false)
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
    .expect("on_error:continue must settle the run future Ok, not bubble up an Err");
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();

    let run_row = flows_get_run(&config, &thread_id).await.unwrap();
    assert_eq!(run_row.value.status, "failed");
    let error = run_row
        .value
        .error
        .as_deref()
        .expect("a degraded-to-failed run must populate FlowRun.error, not leave it None");
    assert!(error.contains('x'), "got: {error}");

    let reloaded = flows_get(&config, &created.value.id).await.unwrap();
    assert_eq!(reloaded.value.last_status.as_deref(), Some("failed"));
}
