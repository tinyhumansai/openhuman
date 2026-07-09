use super::*;
use crate::openhuman::config::Config;
use serde_json::json;
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

fn trigger_only_graph() -> Value {
    json!({
        "name": "trigger-only",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" }
        ],
        "edges": []
    })
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

    let updated = flows_update(&config, &created.value.id, None, None, Some(true))
        .await
        .unwrap();
    assert!(updated.value.require_approval);

    // Omitting `require_approval` on a later update preserves the current value.
    let unchanged = flows_update(&config, &created.value.id, None, None, None)
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

    let err = flows_update(&config, &created.value.id, None, Some(invalid_graph), None)
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
    let err = flows_run(&config, "missing", json!({}), FlowRunTrigger::Rpc)
        .await
        .expect_err("must error");
    assert!(err.contains("not found"));
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

    let err = flows_run(&config, &created.value.id, json!({}), FlowRunTrigger::Rpc)
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

    let run = flows_run(&config, &created.value.id, json!({}), FlowRunTrigger::Rpc)
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

// ── automatic-dispatch binding (issue B2 finding #1) ─────────────────────
//
// Live testing found that `flows_create` persisted a freshly-created,
// `enabled = true` schedule flow WITHOUT registering its cron job — only
// `flows_set_enabled` bound it. So a brand-new enabled schedule flow would
// silently never fire until an app restart (boot reconcile) or a manual
// disable→enable toggle. These tests exercise the fix directly against the
// real `cron` store (not a mock), the same way `bind_schedule_trigger`
// itself does.

fn schedule_trigger_graph(cron_expr: &str) -> Value {
    json!({
        "name": "scheduled",
        "nodes": [
            {
                "id": "t",
                "kind": "trigger",
                "name": "Trigger",
                "config": { "trigger_kind": "schedule", "schedule": cron_expr }
            }
        ],
        "edges": []
    })
}

#[tokio::test]
async fn flows_create_binds_schedule_cron_job_for_an_enabled_flow() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(
        &config,
        "scheduled".to_string(),
        schedule_trigger_graph("0 9 * * *"),
        false,
    )
    .await
    .unwrap();
    assert!(created.value.enabled, "flows_create defaults to enabled");

    let job = crate::openhuman::cron::find_flow_schedule_job(&config, &created.value.id).unwrap();
    assert!(
        job.is_some(),
        "an enabled schedule flow must have its cron job bound immediately on create, not only \
         after a set_enabled toggle"
    );
    assert_eq!(job.unwrap().expression, "0 9 * * *");
}

#[tokio::test]
async fn flows_delete_unbinds_schedule_cron_job() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(
        &config,
        "scheduled".to_string(),
        schedule_trigger_graph("0 9 * * *"),
        false,
    )
    .await
    .unwrap();
    assert!(
        crate::openhuman::cron::find_flow_schedule_job(&config, &created.value.id)
            .unwrap()
            .is_some(),
        "precondition: cron job bound on create"
    );

    flows_delete(&config, &created.value.id).await.unwrap();

    assert!(
        crate::openhuman::cron::find_flow_schedule_job(&config, &created.value.id)
            .unwrap()
            .is_none(),
        "deleting a flow must remove its schedule-trigger cron job — it lives in a separate \
         cron.db that flow_definitions' ON DELETE CASCADE cannot reach"
    );
}

#[tokio::test]
async fn flows_update_rebinds_schedule_cron_job_when_trigger_schedule_changes() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(
        &config,
        "scheduled".to_string(),
        schedule_trigger_graph("0 9 * * *"),
        false,
    )
    .await
    .unwrap();
    let old_job = crate::openhuman::cron::find_flow_schedule_job(&config, &created.value.id)
        .unwrap()
        .expect("cron job bound on create");
    assert_eq!(old_job.expression, "0 9 * * *");

    flows_update(
        &config,
        &created.value.id,
        None,
        Some(schedule_trigger_graph("30 8 * * *")),
        None,
    )
    .await
    .unwrap();

    let new_job = crate::openhuman::cron::find_flow_schedule_job(&config, &created.value.id)
        .unwrap()
        .expect("cron job still bound after trigger schedule change");
    assert_eq!(
        new_job.expression, "30 8 * * *",
        "the bound cron job's schedule must reflect the new trigger config"
    );

    // No duplicate/orphaned job left behind for this flow.
    let flow_jobs: Vec<_> = crate::openhuman::cron::list_jobs(&config)
        .unwrap()
        .into_iter()
        .filter(|j| j.command == created.value.id)
        .collect();
    assert_eq!(flow_jobs.len(), 1);
}

#[tokio::test]
async fn flows_update_does_not_rebind_when_graph_is_not_supplied() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(
        &config,
        "scheduled".to_string(),
        schedule_trigger_graph("0 9 * * *"),
        false,
    )
    .await
    .unwrap();
    let old_job = crate::openhuman::cron::find_flow_schedule_job(&config, &created.value.id)
        .unwrap()
        .expect("cron job bound on create");

    // Name-only update: no graph_json supplied, so the trigger cannot have
    // changed — the existing binding must be left untouched.
    flows_update(
        &config,
        &created.value.id,
        Some("renamed".to_string()),
        None,
        None,
    )
    .await
    .unwrap();

    let job = crate::openhuman::cron::find_flow_schedule_job(&config, &created.value.id)
        .unwrap()
        .expect("cron job still bound");
    assert_eq!(job.id, old_job.id);
    assert_eq!(job.expression, old_job.expression);
}

// ── flows_resume (issue B2) ───────────────────────────────────────────────

fn approval_gated_graph() -> Value {
    json!({
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
    })
}

#[tokio::test]
async fn flows_resume_continues_a_paused_run_to_completion() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();

    let run = flows_run(
        &config,
        &created.value.id,
        json!({ "x": 1 }),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();
    let pending: Vec<String> =
        serde_json::from_value(run.value["pending_approvals"].clone()).unwrap();
    assert_eq!(pending, vec!["gate".to_string()]);

    let resumed = flows_resume(&config, &created.value.id, &thread_id, pending, vec![])
        .await
        .unwrap();
    assert_eq!(resumed.value["pending_approvals"], json!([]));
    assert!(
        !resumed.value["output"]["nodes"]["downstream"]["items"].is_null(),
        "downstream should run once the gate is approved via resume"
    );

    let reloaded = flows_get(&config, &created.value.id).await.unwrap();
    assert_eq!(reloaded.value.last_status.as_deref(), Some("completed"));

    // The run-history row must reflect the final completed status, not the
    // intermediate pending_approval one it started at.
    let run_row = flows_get_run(&config, &thread_id).await.unwrap();
    assert_eq!(run_row.value.status, "completed");
    assert!(run_row.value.pending_approvals.is_empty());
    assert!(
        run_row
            .value
            .steps
            .iter()
            .any(|s| s.node_id == "downstream"),
        "resume should reconstruct the downstream step that ran after approval"
    );
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

// ── flows_resume deny semantics (issue G4) ────────────────────────────────

/// A gate with BOTH a `main` edge (to `downstream`) and an `error` edge (to
/// `recover`): denying the gate routes to `recover`, not `downstream`.
fn approval_gated_graph_with_error_port() -> Value {
    json!({
        "name": "approval-gated-error-port",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "gate", "kind": "output_parser", "name": "Gate", "config": { "requires_approval": true } },
            { "id": "downstream", "kind": "output_parser", "name": "Downstream" },
            { "id": "recover", "kind": "output_parser", "name": "Recover" }
        ],
        "edges": [
            { "from_node": "t", "to_node": "gate" },
            { "from_node": "gate", "from_port": "main", "to_node": "downstream" },
            { "from_node": "gate", "from_port": "error", "to_node": "recover" }
        ]
    })
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

    let run = flows_run(&config, &created.value.id, json!({}), FlowRunTrigger::Rpc)
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
    let run = flows_run(&config, &created.value.id, json!({}), FlowRunTrigger::Rpc)
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

#[tokio::test]
async fn flows_resume_with_no_recorded_run_for_thread_id_errors_clearly() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "demo".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();

    let err = flows_resume(
        &config,
        &created.value.id,
        "thread-that-was-never-started",
        vec![],
        vec![],
    )
    .await
    .expect_err("must error when no run is recorded for this thread_id");
    assert!(err.contains("no paused run to resume"));
}

// ── run history (flows_list_runs / flows_get_run) ────────────────────────

#[tokio::test]
async fn flows_run_persists_a_flow_run_row_queryable_via_list_and_get() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "demo".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();

    let run = flows_run(
        &config,
        &created.value.id,
        json!({ "hello": "world" }),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();

    let runs = flows_list_runs(&config, &created.value.id, 20)
        .await
        .unwrap();
    assert_eq!(runs.value.len(), 1);
    assert_eq!(runs.value[0].id, thread_id);
    assert_eq!(runs.value[0].status, "completed");

    let single = flows_get_run(&config, &thread_id).await.unwrap();
    assert_eq!(single.value.flow_id, created.value.id);
    assert_eq!(single.value.status, "completed");
    assert!(
        single.value.steps.iter().any(|s| s.node_id == "t"),
        "the trigger node's step should be reconstructed from output[\"nodes\"]"
    );
}

#[tokio::test]
async fn flows_get_run_missing_run_errors() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = flows_get_run(&config, "missing-run")
        .await
        .expect_err("must error");
    assert!(err.contains("not found"));
}

// ── pending-approval notification ────────────────────────────────────────

#[tokio::test]
async fn flows_run_emits_pending_approval_notification() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let mut rx = crate::openhuman::notifications::bus::subscribe_core_notifications();

    let created = flows_create(
        &config,
        "gated-notify".to_string(),
        approval_gated_graph(),
        false,
    )
    .await
    .unwrap();

    let run = flows_run(&config, &created.value.id, json!({}), FlowRunTrigger::Rpc)
        .await
        .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();

    // Filter for our notification specifically — the broadcast bus is
    // process-global, so a concurrently-running test's notification could
    // otherwise be received first.
    let expected_prefix = format!("flow-pending-approval:{}:", created.value.id);
    let mut found = None;
    for _ in 0..20 {
        match tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await {
            Ok(Ok(n)) if n.id.starts_with(&expected_prefix) => {
                found = Some(n);
                break;
            }
            Ok(Ok(_unrelated)) => continue,
            _ => break,
        }
    }
    let notification = found.expect("expected a pending-approval notification for this flow");

    assert_eq!(
        notification.category,
        crate::openhuman::notifications::types::CoreNotificationCategory::Agents
    );
    let actions = notification
        .actions
        .expect("pending-approval notification must carry an action");
    let approve = actions
        .iter()
        .find(|a| a.action_id == "approve")
        .expect("expected an 'approve' action");
    let payload = approve
        .payload
        .clone()
        .expect("approve action must carry a payload");
    assert_eq!(payload["flow_id"], json!(created.value.id));
    assert_eq!(payload["thread_id"], json!(thread_id));
    assert_eq!(payload["node_ids"], json!(["gate"]));
}

#[tokio::test]
async fn flows_run_does_not_notify_when_run_completes_without_pending_approvals() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let mut rx = crate::openhuman::notifications::bus::subscribe_core_notifications();

    let created = flows_create(&config, "no-gate".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();
    let created_id = created.value.id.clone();

    flows_run(&config, &created.value.id, json!({}), FlowRunTrigger::Rpc)
        .await
        .unwrap();

    let expected_prefix = format!("flow-pending-approval:{created_id}:");
    let saw_notification = tokio::time::timeout(std::time::Duration::from_millis(300), async {
        loop {
            match rx.recv().await {
                Ok(n) if n.id.starts_with(&expected_prefix) => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(
        !saw_notification,
        "a fully-completed run must not publish a pending-approval notification"
    );
}

// ── Live run observation (issue G2) ───────────────────────────────────────

use crate::openhuman::tinyflows::observability::FlowRunObserver;
use std::sync::Arc as StdArc;
// `RunObserver` must be in scope to call `on_step_finish` on the observer.
use tinyflows::observability::{ExecutionStep, RunObserver as _, StepStatus};

/// trigger -> output_parser passthrough: the parser is a non-trigger node, so
/// the engine fires `on_step_finish` for it, exercising live persistence.
fn passthrough_graph() -> Value {
    json!({
        "name": "passthrough",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "p", "kind": "output_parser", "name": "Parse" }
        ],
        "edges": [ { "from_node": "t", "to_node": "p" } ]
    })
}

#[tokio::test]
async fn observer_persists_each_step_incrementally() {
    // The observer no-ops until the run's start row exists (mirrors
    // `start_flow_run_row`), so seed a flow + a running run row first.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "obs".to_string(), passthrough_graph(), false)
        .await
        .unwrap();
    let run_id = format!("flow:{}:run-under-test", created.value.id);
    store::insert_flow_run(
        &config,
        &run_id,
        &created.value.id,
        &run_id,
        "2026-01-01T00:00:00Z",
    )
    .unwrap();

    let observer = FlowRunObserver::new(
        StdArc::new(config.clone()),
        created.value.id.clone(),
        &run_id,
    );
    observer.on_step_finish(&ExecutionStep {
        node_id: "a".to_string(),
        status: StepStatus::Success,
        output: json!([{ "json": { "ok": true } }]),
        duration_ms: 7,
        diagnostics: Vec::new(),
    });
    observer.on_step_finish(&ExecutionStep {
        node_id: "b".to_string(),
        status: StepStatus::Error,
        output: Value::Null,
        duration_ms: 3,
        diagnostics: Vec::new(),
    });

    // The store now holds both live steps with real status + timing — proof of
    // incremental persistence (post-hoc reconstruction leaves status None).
    let row = store::get_flow_run(&config, &run_id).unwrap().unwrap();
    assert_eq!(row.steps.len(), 2, "both live steps should be persisted");
    let a = row.steps.iter().find(|s| s.node_id == "a").unwrap();
    assert_eq!(a.status.as_deref(), Some("success"));
    assert_eq!(a.duration_ms, Some(7));
    let b = row.steps.iter().find(|s| s.node_id == "b").unwrap();
    assert_eq!(b.status.as_deref(), Some("error"));
    assert_eq!(b.duration_ms, Some(3));

    // Re-firing the same node id replaces its entry rather than duplicating it.
    observer.on_step_finish(&ExecutionStep {
        node_id: "a".to_string(),
        status: StepStatus::Success,
        output: json!([{ "json": { "ok": true } }]),
        duration_ms: 42,
        diagnostics: Vec::new(),
    });
    let row = store::get_flow_run(&config, &run_id).unwrap().unwrap();
    assert_eq!(row.steps.len(), 2, "re-firing a node must not duplicate it");
    let a = row.steps.iter().find(|s| s.node_id == "a").unwrap();
    assert_eq!(
        a.duration_ms,
        Some(42),
        "the step should be replaced in place"
    );
}

#[tokio::test]
async fn flows_run_persists_live_steps_with_status_and_timing() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(
        &config,
        "passthrough".to_string(),
        passthrough_graph(),
        false,
    )
    .await
    .unwrap();

    let run = flows_run(
        &config,
        &created.value.id,
        json!({ "x": 1 }),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();

    let row = flows_get_run(&config, &thread_id).await.unwrap();
    assert_eq!(row.value.status, "completed");

    // The non-trigger node 'p' was observed live: it carries a real status +
    // timing that only the live observer (not post-hoc reconstruction) sets.
    let p = row
        .value
        .steps
        .iter()
        .find(|s| s.node_id == "p")
        .expect("the output_parser step should be persisted");
    assert_eq!(p.status.as_deref(), Some("success"));
    assert!(
        p.duration_ms.is_some(),
        "a live-observed step should carry executor timing"
    );

    // The trigger node emits no `on_step_finish`; `settle_steps` fills it in
    // from the post-hoc reconstruction, so it carries no live status.
    let t = row
        .value
        .steps
        .iter()
        .find(|s| s.node_id == "t")
        .expect("the trigger step should be reconstructed at settle");
    assert!(
        t.status.is_none(),
        "the trigger step is reconstructed post-hoc, not observed live"
    );
}

// ── flows_cancel_run (issue G4) ───────────────────────────────────────────

#[tokio::test]
async fn flows_cancel_run_cancels_a_parked_pending_approval_run() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();

    // Run pauses at the gate → a durable `pending_approval` row with no live
    // task (the run future already returned): the not-in-flight cancel path.
    let run = flows_run(&config, &created.value.id, json!({}), FlowRunTrigger::Rpc)
        .await
        .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();
    assert_eq!(
        flows_get_run(&config, &thread_id)
            .await
            .unwrap()
            .value
            .status,
        "pending_approval"
    );

    let cancelled = flows_cancel_run(&config, &thread_id).await.unwrap();
    assert_eq!(cancelled.value["cancelled"], json!(true));
    assert_eq!(
        cancelled.value["was_in_flight"],
        json!(false),
        "a parked run has no live task, so the cancel settles the row directly"
    );

    // The run row and the flow summary both reach the terminal `cancelled`.
    let run_row = flows_get_run(&config, &thread_id).await.unwrap();
    assert_eq!(run_row.value.status, "cancelled");
    assert!(run_row.value.pending_approvals.is_empty());
    assert_eq!(run_row.value.error.as_deref(), Some("run cancelled"));

    let reloaded = flows_get(&config, &created.value.id).await.unwrap();
    assert_eq!(reloaded.value.last_status.as_deref(), Some("cancelled"));

    // A cancelled run can no longer be resumed — the status guard rejects it.
    let err = flows_resume(
        &config,
        &created.value.id,
        &thread_id,
        vec!["gate".to_string()],
        vec![],
    )
    .await
    .expect_err("a cancelled run must not be resumable");
    assert!(err.contains("not pending approval") || err.contains("no paused run"));
}

#[tokio::test]
async fn flows_cancel_run_of_an_already_completed_run_errors() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "demo".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();

    let run = flows_run(&config, &created.value.id, json!({}), FlowRunTrigger::Rpc)
        .await
        .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();

    let err = flows_cancel_run(&config, &thread_id)
        .await
        .expect_err("cancelling an already-completed run must be a clear error");
    assert!(err.contains("already terminal"), "got: {err}");
}

#[tokio::test]
async fn flows_cancel_run_of_a_completed_with_warnings_run_errors() {
    // A settled `completed_with_warnings` run (run honesty, PR2) must be just
    // as terminal as a plain `completed` run — otherwise `flows_cancel_run`
    // falls through to its not-in-flight path and overwrites the row (and the
    // flow summary) as `"cancelled"`, silently discarding the warning status
    // the run already recorded.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "demo".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();

    let run = flows_run(&config, &created.value.id, json!({}), FlowRunTrigger::Rpc)
        .await
        .unwrap();
    let thread_id = run.value["thread_id"].as_str().unwrap().to_string();

    // Force the settled row to the warning status directly — an end-to-end
    // null-binding graph isn't needed to exercise this guard.
    store::finish_flow_run(
        &config,
        &thread_id,
        "completed_with_warnings",
        &chrono::Utc::now().to_rfc3339(),
        &[],
        &[],
        None,
    )
    .unwrap();

    let err = flows_cancel_run(&config, &thread_id)
        .await
        .expect_err("cancelling a completed_with_warnings run must be a clear error");
    assert!(err.contains("already terminal"), "got: {err}");

    // And the row must still read back as the warning status, not overwritten.
    let run_row = flows_get_run(&config, &thread_id).await.unwrap();
    assert_eq!(run_row.value.status, "completed_with_warnings");
}

#[tokio::test]
async fn flows_cancel_run_missing_run_errors() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = flows_cancel_run(&config, "no-such-run")
        .await
        .expect_err("must error for an unknown run");
    assert!(err.contains("not found"));
}

// ── parked-run TTL sweep (issue G4) ───────────────────────────────────────

#[tokio::test]
async fn parked_run_ttl_sweep_expires_stale_runs_but_spares_fresh_ones() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(&config, "gated".to_string(), approval_gated_graph(), false)
        .await
        .unwrap();

    // Seed a parked run whose "parked since" (finished_at) is far in the past,
    // so it is well beyond the TTL.
    let stale_id = format!("flow:{}:stale-run", created.value.id);
    let ancient = "2000-01-01T00:00:00+00:00";
    store::insert_flow_run(&config, &stale_id, &created.value.id, &stale_id, ancient).unwrap();
    store::finish_flow_run(
        &config,
        &stale_id,
        "pending_approval",
        ancient,
        &[],
        &["gate".to_string()],
        None,
    )
    .unwrap();

    // A genuinely fresh parked run (just paused now) must survive the sweep.
    let fresh = flows_run(&config, &created.value.id, json!({}), FlowRunTrigger::Rpc)
        .await
        .unwrap();
    let fresh_id = fresh.value["thread_id"].as_str().unwrap().to_string();

    let swept = sweep_expired_parked_runs(&config).await;
    assert_eq!(swept, 1, "only the stale parked run should be swept");

    let stale_row = store::get_flow_run(&config, &stale_id).unwrap().unwrap();
    assert_eq!(stale_row.status, "cancelled");
    assert!(
        stale_row.error.unwrap_or_default().contains("expired"),
        "an expired run's error must note the TTL expiry"
    );

    let fresh_row = store::get_flow_run(&config, &fresh_id).unwrap().unwrap();
    assert_eq!(
        fresh_row.status, "pending_approval",
        "a run parked within the TTL must not be swept"
    );

    // The swept run is no longer resumable.
    let err = flows_resume(
        &config,
        &created.value.id,
        &stale_id,
        vec!["gate".to_string()],
        vec![],
    )
    .await
    .expect_err("an expired parked run must not be resumable");
    assert!(err.contains("not pending approval") || err.contains("no paused run"));
}

// ---------------------------------------------------------------------------
// Unfired-trigger-kind warnings (PHASE 1a validation + PHASE 3c flows_validate)
// ---------------------------------------------------------------------------

fn webhook_trigger_graph() -> Value {
    json!({
        "name": "hooked",
        "nodes": [
            {
                "id": "t",
                "kind": "trigger",
                "name": "Trigger",
                "config": { "trigger_kind": "webhook" }
            }
        ],
        "edges": []
    })
}

#[test]
fn flows_validate_warns_on_unfired_webhook_trigger() {
    let outcome = flows_validate(webhook_trigger_graph());
    assert!(outcome.value.valid, "a webhook graph is structurally valid");
    assert!(outcome.value.errors.is_empty());
    assert_eq!(
        outcome.value.warnings.len(),
        1,
        "an unfired webhook trigger must produce exactly one warning: {:?}",
        outcome.value.warnings
    );
    assert!(
        outcome.value.warnings[0].contains("webhook")
            && outcome.value.warnings[0].contains("does not fire"),
        "warning must name the kind and explain it does not fire: {:?}",
        outcome.value.warnings
    );
}

#[test]
fn flows_validate_does_not_warn_on_schedule_trigger() {
    let outcome = flows_validate(schedule_trigger_graph("0 9 * * *"));
    assert!(outcome.value.valid);
    assert!(
        outcome.value.warnings.is_empty(),
        "a schedule trigger fires — it must not warn: {:?}",
        outcome.value.warnings
    );
}

#[test]
fn flows_validate_reports_error_for_graph_without_trigger() {
    let graph = json!({
        "name": "bad",
        "nodes": [ { "id": "a", "kind": "output_parser", "name": "A" } ],
        "edges": []
    });
    let outcome = flows_validate(graph);
    assert!(!outcome.value.valid);
    assert_eq!(outcome.value.errors.len(), 1);
    assert!(outcome.value.errors[0].contains("trigger"));
    assert!(
        outcome.value.warnings.is_empty(),
        "an invalid graph reports no warnings"
    );
}

#[tokio::test]
async fn flows_set_enabled_surfaces_unfired_trigger_warning_at_enable() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(
        &config,
        "hooked".to_string(),
        webhook_trigger_graph(),
        false,
    )
    .await
    .unwrap();

    // Re-enable (create already enables) to exercise the enable path's warning.
    let enabled = flows_set_enabled(&config, &created.value.id, true)
        .await
        .unwrap();
    assert!(enabled.value.enabled);
    assert!(
        enabled
            .logs
            .iter()
            .any(|l| l.starts_with("warning:") && l.contains("webhook")),
        "enabling a webhook-trigger flow must surface a loud warning log, got: {:?}",
        enabled.logs
    );
}

#[tokio::test]
async fn flows_set_enabled_schedule_flow_has_no_warning() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(
        &config,
        "scheduled".to_string(),
        schedule_trigger_graph("0 9 * * *"),
        false,
    )
    .await
    .unwrap();

    let enabled = flows_set_enabled(&config, &created.value.id, true)
        .await
        .unwrap();
    assert!(
        !enabled.logs.iter().any(|l| l.starts_with("warning:")),
        "a schedule-trigger flow must not surface an unfired-trigger warning: {:?}",
        enabled.logs
    );
}

// ── flows_list_connections (picker source) ──────────────────────────────

use crate::openhuman::composio::ComposioConnection;
use crate::openhuman::credentials::{HttpCredential, HttpCredentialSummary, HttpCredentialsStore};

fn composio_conn(id: &str, toolkit: &str, status: &str, email: Option<&str>) -> ComposioConnection {
    ComposioConnection {
        id: id.to_string(),
        toolkit: toolkit.to_string(),
        status: status.to_string(),
        created_at: None,
        account_email: email.map(str::to_string),
        workspace: None,
        username: None,
    }
}

fn http_summary(name: &str, scheme: &str) -> HttpCredentialSummary {
    HttpCredentialSummary {
        name: name.to_string(),
        scheme: scheme.to_string(),
        header_name: None,
        username: None,
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

#[test]
fn build_flow_connections_emits_parseable_refs_for_both_kinds() {
    let composio = vec![composio_conn(
        "ca_abc",
        "Gmail",
        "ACTIVE",
        Some("user@example.com"),
    )];
    let http = vec![http_summary("stripe", "bearer")];

    let out = build_flow_connections(composio, http);
    assert_eq!(out.len(), 2);

    let gmail = &out[0];
    assert_eq!(gmail.kind, "composio");
    // Toolkit is normalized (lowercased) and the ref round-trips through the
    // exact parser the caps seam uses on execution.
    assert_eq!(gmail.connection_ref, "composio:gmail:ca_abc");
    assert_eq!(
        crate::openhuman::tinyflows::caps::composio_connection_id(&gmail.connection_ref),
        Some("ca_abc")
    );
    assert_eq!(gmail.toolkit.as_deref(), Some("gmail"));
    assert_eq!(gmail.display, "Gmail · user@example.com");
    assert!(gmail.scheme.is_none());

    let stripe = &out[1];
    assert_eq!(stripe.kind, "http");
    assert_eq!(stripe.connection_ref, "http_cred:stripe");
    assert_eq!(
        crate::openhuman::tinyflows::caps::http_cred_name(&stripe.connection_ref),
        Some("stripe")
    );
    assert_eq!(stripe.scheme.as_deref(), Some("bearer"));
    assert_eq!(stripe.display, "stripe (bearer)");
    assert!(stripe.toolkit.is_none());
}

#[test]
fn build_flow_connections_skips_non_active_composio_accounts() {
    let composio = vec![
        composio_conn("ca_ok", "notion", "ACTIVE", None),
        composio_conn("ca_pending", "slack", "PENDING", None),
    ];
    let out = build_flow_connections(composio, Vec::new());
    assert_eq!(out.len(), 1, "only the ACTIVE connection is surfaced");
    assert_eq!(out[0].connection_ref, "composio:notion:ca_ok");
    // No cached identity → title-cased toolkit alone.
    assert_eq!(out[0].display, "Notion");
}

#[test]
fn build_flow_connections_never_carries_secret_fields() {
    let out = build_flow_connections(
        vec![composio_conn("ca_abc", "gmail", "ACTIVE", Some("u@x.io"))],
        vec![http_summary("stripe", "header")],
    );
    let json = serde_json::to_string(&out).unwrap();
    // The serialized picker payload must expose only ref/kind/display/toolkit/
    // scheme — no secret-bearing key names at all.
    for banned in [
        "secret", "token", "password", "\"key\"", "apiKey", "api_key",
    ] {
        assert!(
            !json
                .to_ascii_lowercase()
                .contains(&banned.to_ascii_lowercase()),
            "serialized FlowConnection leaked a secret-bearing field ({banned}): {json}"
        );
    }
}

#[test]
fn title_case_toolkit_handles_underscores_and_dashes() {
    assert_eq!(title_case_toolkit("gmail"), "Gmail");
    assert_eq!(title_case_toolkit("google_calendar"), "Google Calendar");
    assert_eq!(title_case_toolkit("google-sheets"), "Google Sheets");
    assert_eq!(title_case_toolkit(""), "");
}

#[tokio::test]
async fn flows_list_connections_aggregates_http_creds_and_tolerates_composio() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    // Force Direct mode with no key so the composio source short-circuits to an
    // empty list offline (no network) — proving the aggregation still returns
    // the HTTP-credential half.
    config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    // Secrets in the clear at rest for the test (mirrors the E2E config).
    config.secrets.encrypt = false;

    // Seed one HTTP credential through the same store the op reads.
    let store = HttpCredentialsStore::from_config(&config);
    store
        .upsert(&HttpCredential::bearer("stripe", "sk_live_seed_secret"))
        .unwrap();

    let outcome = flows_list_connections(&config).await.unwrap();
    let refs: Vec<_> = outcome
        .value
        .iter()
        .map(|c| c.connection_ref.as_str())
        .collect();
    assert!(
        refs.contains(&"http_cred:stripe"),
        "http_cred must be surfaced: {refs:?}"
    );

    // The secret must never appear anywhere in the RPC payload.
    let json = serde_json::to_string(&outcome.value).unwrap();
    assert!(
        !json.contains("sk_live_seed_secret"),
        "secret leaked into flows_list_connections payload: {json}"
    );
}

// ── Flow Scout suggestion lifecycle ──────────────────────────────────────────

fn seed_suggestion(config: &Config, id: &str) {
    let s = crate::openhuman::flows::FlowSuggestion {
        id: id.to_string(),
        title: format!("Idea {id}"),
        one_liner: "does a thing".to_string(),
        rationale: "grounded".to_string(),
        trigger_hint: Some("schedule".to_string()),
        steps_outline: vec!["a".to_string()],
        suggested_connections: vec![],
        suggested_slugs: vec![],
        build_prompt: "Build a workflow…".to_string(),
        confidence: 0.5,
        status: crate::openhuman::flows::SuggestionStatus::New,
        created_at: "2026-07-05T00:00:00Z".to_string(),
        source_run_id: None,
    };
    crate::openhuman::flows::store::upsert_suggestions(config, &[s]).unwrap();
}

#[tokio::test]
async fn list_suggestions_filters_by_status() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    seed_suggestion(&config, "s1");
    seed_suggestion(&config, "s2");

    let active = flows_list_suggestions(
        &config,
        Some(crate::openhuman::flows::SuggestionStatus::New),
    )
    .await
    .unwrap();
    assert_eq!(active.value.len(), 2);

    // Unfiltered returns all too.
    let all = flows_list_suggestions(&config, None).await.unwrap();
    assert_eq!(all.value.len(), 2);
}

#[tokio::test]
async fn dismiss_and_mark_built_move_suggestions_out_of_active() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    seed_suggestion(&config, "s1");
    seed_suggestion(&config, "s2");

    let d = flows_dismiss_suggestion(&config, "s1").await.unwrap();
    assert_eq!(d.value["dismissed"], json!(true));
    let b = flows_mark_suggestion_built(&config, "s2").await.unwrap();
    assert_eq!(b.value["built"], json!(true));

    // Neither is in the active (New) set anymore.
    let active = flows_list_suggestions(
        &config,
        Some(crate::openhuman::flows::SuggestionStatus::New),
    )
    .await
    .unwrap();
    assert!(active.value.is_empty());
}

#[tokio::test]
async fn dismiss_unknown_suggestion_reports_not_found() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let d = flows_dismiss_suggestion(&config, "missing").await.unwrap();
    assert_eq!(d.value["dismissed"], json!(false));
}

// ─────────────────────────────────────────────────────────────────────────────
// FlowStreamTarget (Phase B copilot/scout streaming) — pure param plumbing.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn flow_stream_target_none_without_thread_id() {
    // No thread → headless run, regardless of request_id.
    assert!(FlowStreamTarget::from_params(None, None).is_none());
    assert!(FlowStreamTarget::from_params(None, Some("r-1".to_string())).is_none());
}

#[test]
fn flow_stream_target_blank_thread_id_is_absent() {
    // Whitespace-only thread id is treated as no thread (callers pass raw input).
    assert!(FlowStreamTarget::from_params(Some("   ".to_string()), None).is_none());
    assert!(FlowStreamTarget::from_params(Some(String::new()), None).is_none());
}

#[test]
fn flow_stream_target_trims_and_keeps_request_id() {
    let t = FlowStreamTarget::from_params(Some("  t-1  ".to_string()), Some("  r-1  ".to_string()))
        .expect("stream target");
    assert_eq!(t.thread_id, "t-1");
    assert_eq!(t.request_id, "r-1");
}

#[test]
fn flow_stream_target_generates_request_id_when_absent_or_blank() {
    // Absent request id → a fresh uuid is minted.
    let a = FlowStreamTarget::from_params(Some("t-1".to_string()), None).expect("target");
    assert!(!a.request_id.is_empty());
    assert_ne!(a.request_id, a.thread_id);
    // Blank request id is treated the same way.
    let b = FlowStreamTarget::from_params(Some("t-1".to_string()), Some("  ".to_string()))
        .expect("target");
    assert!(!b.request_id.is_empty());
    // Two mints are distinct uuids.
    assert_ne!(a.request_id, b.request_id);
}

// ── validate_binding_resolvability ──────────────────────────────────────────

/// Runs a candidate graph `Value` through the exact same migrate/validate
/// path the builder tools use, for a [`WorkflowGraph`] test fixture.
fn graph(value: Value) -> WorkflowGraph {
    validate_and_migrate_graph(value).expect("structurally valid test graph")
}

#[test]
fn binding_to_agent_without_schema_is_rejected() {
    // The exact live-failure shape: `summarize` has no `output_parser.schema`
    // at all, so its structured output has no addressable `channel` field.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "agent_ref": "researcher", "prompt": "summarize" } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "=nodes.summarize.item.json.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "post" }
        ]
    }));
    let errors = validate_binding_resolvability(&g);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("post"), "{}", errors[0]);
    assert!(errors[0].contains("channel"), "{}", errors[0]);
    assert!(errors[0].contains("summarize"), "{}", errors[0]);
    assert!(errors[0].contains("output_parser.schema"), "{}", errors[0]);
}

#[test]
fn binding_to_agent_with_schema_missing_field_is_rejected() {
    // A schema IS declared, but it doesn't cover the field the binding reads.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "prompt": "summarize",
                "output_parser": { "schema": { "type": "object",
                    "properties": { "summary": { "type": "string" } } } } } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "=nodes.summarize.item.json.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "post" }
        ]
    }));
    let errors = validate_binding_resolvability(&g);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("channel"), "{}", errors[0]);
}

#[test]
fn binding_to_agent_with_matching_schema_is_accepted() {
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "prompt": "summarize",
                "output_parser": { "schema": { "type": "object",
                    "required": ["channel"],
                    "properties": { "channel": { "type": "string" } } } } } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "=nodes.summarize.item.json.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "post" }
        ]
    }));
    assert!(
        validate_binding_resolvability(&g).is_empty(),
        "{:?}",
        validate_binding_resolvability(&g)
    );
}

// ── validate_tool_contracts (systemic tool-contract fix, Part 2) ───────────
//
// The live-catalog cache is process-global (`LIVE_CATALOG_CACHE`) — every
// test below seeds the exact toolkit it needs via `seed_live_catalog_cache`
// so none of this touches a live Composio backend.

use crate::openhuman::tinyflows::caps::{
    seed_live_catalog_cache, seed_probe_cache, ProbedOutputSample, ToolContract,
};

fn seeded_slack_send_contract() -> ToolContract {
    ToolContract {
        slug: "SLACK_SEND_MESSAGE".to_string(),
        toolkit: "slack".to_string(),
        description: None,
        required_args: vec!["channel".to_string(), "text".to_string()],
        input_schema: None,
        output_fields: vec!["ts".to_string(), "channel".to_string()],
        output_schema: Some(json!({
            "type": "object",
            "properties": { "ts": {"type": "string"}, "channel": {"type": "string"} }
        })),
        primary_array_path: None,
        // `slack` ships a static curated catalog (`catalog_for_toolkit`), so
        // `validate_tool_contracts` now enforces the same curated-only bar
        // `flow_tool_allowed`'s Path A does at runtime (Codex feedback on
        // this PR) — this fixture models a real curated Slack action, not
        // an uncurated one, since these tests exercise the required-arg /
        // hallucinated-slug checks rather than the curation gate itself.
        is_curated: true,
    }
}

#[tokio::test]
async fn validate_tool_contracts_rejects_a_hallucinated_slug() {
    seed_live_catalog_cache("slack", vec![seeded_slack_send_contract()]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_POST_MESSAGE_TO_CHANNEL",
                "args": { "channel": "#general", "text": "hi" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("post"), "{}", errors[0]);
    assert!(
        errors[0].contains("SLACK_POST_MESSAGE_TO_CHANNEL"),
        "{}",
        errors[0]
    );
    assert!(errors[0].contains("search_tool_catalog"), "{}", errors[0]);
}

#[tokio::test]
async fn validate_tool_contracts_rejects_a_missing_required_arg() {
    seed_live_catalog_cache("slack", vec![seeded_slack_send_contract()]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "#general" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("`text`"), "{}", errors[0]);
    assert!(errors[0].contains("get_tool_contract"), "{}", errors[0]);
}

#[tokio::test]
async fn validate_tool_contracts_passes_a_fully_wired_real_slug() {
    seed_live_catalog_cache("slack", vec![seeded_slack_send_contract()]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "#general", "text": "hi" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert!(errors.is_empty(), "{errors:?}");
}

/// (Codex feedback on this PR) `notion` ships a static curated catalog
/// (`catalog_for_toolkit`), so at RUNTIME `flow_tool_allowed`'s Path A
/// hard-rejects any slug `find_curated` doesn't recognize — even a real,
/// live action. Without this check, a real-but-uncurated action for a
/// statically-catalogued toolkit would pass authoring/save here and then
/// fail every single run as "tool not permitted". Uses its own toolkit key
/// (`notion`, not `slack`/`gmail`) since it seeds different `is_curated`
/// content than every other test sharing those keys.
#[tokio::test]
async fn validate_tool_contracts_rejects_a_real_but_uncurated_action_on_a_statically_catalogued_toolkit(
) {
    seed_live_catalog_cache(
        "notion",
        vec![ToolContract {
            slug: "NOTION_UNCURATED_ACTION".to_string(),
            toolkit: "notion".to_string(),
            description: None,
            required_args: vec![],
            input_schema: None,
            output_fields: vec![],
            output_schema: None,
            primary_array_path: None,
            // Real (a live catalog fetch found it), but NOT one of
            // OpenHuman's curated Notion actions.
            is_curated: false,
        }],
    );
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "NOTION_UNCURATED_ACTION", "args": {} } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("NOTION_UNCURATED_ACTION"),
        "{}",
        errors[0]
    );
    assert!(errors[0].contains("curated"), "{}", errors[0]);
}

#[tokio::test]
async fn validate_tool_contracts_skips_expression_derived_and_native_slugs() {
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "dynamic", "kind": "tool_call", "name": "Dynamic",
              "config": { "slug": "=item.tool", "args": {} } },
            { "id": "native", "kind": "tool_call", "name": "Native",
              "config": { "slug": "oh:web_search", "args": {} } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "dynamic" },
            { "from_node": "t", "to_node": "native" }
        ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert!(errors.is_empty(), "{errors:?}");
}

#[tokio::test]
async fn validate_tool_contracts_skips_rather_than_rejects_when_the_catalog_is_unreachable() {
    // No seed for this toolkit and no live backend configured — the fetch
    // fails, and the node must be SKIPPED (never false-rejected).
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SOMEUNSEEDEDTOOLKIT_DO_THING", "args": {} } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert!(
        errors.is_empty(),
        "a live-catalog fetch failure must skip, not reject: {errors:?}"
    );
}

// ── validate_tool_contracts: arg-NAME validation against the input schema
//    (B13 — a misnamed/unsupported field, e.g. `text` instead of
//    `markdown_text` for `SLACK_SEND_MESSAGE`, used to sail through
//    `missing_required_args` because SOME value was present, just under the
//    wrong key) ────────────────────────────────────────────────────────────

/// `SLACK_SEND_MESSAGE` with a real `input_schema` naming `channel` and
/// `markdown_text` — models the live bug this fixes: `markdown_text` is the
/// real field, `text` is not.
fn seeded_slack_send_message_contract_with_schema() -> ToolContract {
    ToolContract {
        slug: "SLACK_SEND_MESSAGE".to_string(),
        toolkit: "slack".to_string(),
        description: None,
        required_args: vec![],
        input_schema: Some(json!({
            "type": "object",
            "properties": {
                "channel": { "type": "string" },
                "markdown_text": { "type": "string" }
            }
        })),
        output_fields: vec![],
        output_schema: None,
        primary_array_path: None,
        is_curated: true,
    }
}

#[tokio::test]
async fn validate_tool_contracts_rejects_an_arg_name_not_in_the_input_schema() {
    seed_live_catalog_cache(
        "slack",
        vec![seeded_slack_send_message_contract_with_schema()],
    );
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "#general", "text": "hi" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("post"), "{}", errors[0]);
    assert!(errors[0].contains("`text`"), "{}", errors[0]);
    assert!(errors[0].contains("markdown_text"), "{}", errors[0]);
    assert!(errors[0].contains("get_tool_contract"), "{}", errors[0]);
}

#[tokio::test]
async fn validate_tool_contracts_passes_the_real_arg_name_from_the_input_schema() {
    seed_live_catalog_cache(
        "slack",
        vec![seeded_slack_send_message_contract_with_schema()],
    );
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "#general", "markdown_text": "hi" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert!(errors.is_empty(), "{errors:?}");
}

/// Uses its own cache key/toolkit (never `"slack"`/`"gmail"`) since the
/// arg-name check must behave identically no matter which slug it's
/// exercised against, and a dedicated, unregistered toolkit sidesteps both
/// the process-global `LIVE_CATALOG_CACHE` sharing risk the other
/// `validate_tool_contracts` tests accept AND the static curated-catalog
/// gate (this toolkit has none, so `is_curated` is irrelevant here).
#[tokio::test]
async fn validate_tool_contracts_skips_arg_name_check_when_input_schema_is_unknown() {
    seed_live_catalog_cache(
        "argschemaunknown",
        vec![ToolContract {
            slug: "ARGSCHEMAUNKNOWN_DO_THING".to_string(),
            toolkit: "argschemaunknown".to_string(),
            description: None,
            required_args: vec![],
            input_schema: None,
            output_fields: vec![],
            output_schema: None,
            primary_array_path: None,
            is_curated: false,
        }],
    );
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "ARGSCHEMAUNKNOWN_DO_THING",
                "args": { "totally_made_up_field": "hi" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert!(
        errors.is_empty(),
        "an unknown input_schema must skip the arg-name check, never reject: {errors:?}"
    );
}

#[tokio::test]
async fn validate_tool_contracts_allows_arbitrary_arg_names_when_schema_permits_additional_properties(
) {
    seed_live_catalog_cache(
        "argschemaadditional",
        vec![ToolContract {
            slug: "ARGSCHEMAADDITIONAL_DO_THING".to_string(),
            toolkit: "argschemaadditional".to_string(),
            description: None,
            required_args: vec![],
            input_schema: Some(json!({
                "type": "object",
                "properties": { "channel": { "type": "string" } },
                "additionalProperties": true
            })),
            output_fields: vec![],
            output_schema: None,
            primary_array_path: None,
            is_curated: false,
        }],
    );
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "ARGSCHEMAADDITIONAL_DO_THING",
                "args": { "channel": "#general", "any_extra_field": "hi" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_tool_contracts(&config, &g).await;
    assert!(
        errors.is_empty(),
        "additionalProperties: true must allow arbitrary arg names: {errors:?}"
    );
}

// ── graph_wiring_warnings: required-arg advisory + output-field/split_out.path
//    advisories (Part 2c/2d) ────────────────────────────────────────────────

/// `graph_wiring_warnings`'s own required-arg check, exercised DIRECTLY
/// (rather than through `revise_workflow`/`save_workflow`, where the newer
/// `validate_tool_contracts` hard-rejects the identical condition first —
/// see `revise_workflow_rejects_a_missing_required_composio_arg` in
/// `builder_tools_tests.rs`). Keeps this advisory code path covered for any
/// caller that consults `graph_wiring_warnings` without also running the
/// hard gate first.
#[tokio::test]
async fn graph_wiring_warnings_flags_a_missing_required_arg() {
    seed_live_catalog_cache("slack", vec![seeded_slack_send_contract()]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "#general" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let warnings = graph_wiring_warnings(&config, &g).await;
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("`text`") && w.contains("post")),
        "{warnings:?}"
    );
}

#[tokio::test]
async fn graph_wiring_warnings_flags_a_downstream_field_not_in_output_fields() {
    seed_live_catalog_cache("slack", vec![seeded_slack_send_contract()]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "#general", "text": "hi" } } },
            { "id": "xform", "kind": "transform", "name": "Log",
              // Correctly `data.`-prefixed (a real tool_call's payload is
              // always nested under `data`), but the field itself isn't in
              // SLACK_SEND_MESSAGE's real output_fields (`ts`/`channel`) —
              // must WARN, not reject.
              "config": { "set": { "note": "=nodes.post.item.json.data.not_a_real_field" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "xform" }
        ]
    }));
    let warnings = graph_wiring_warnings(&config, &g).await;
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("not_a_real_field") && w.contains("post")),
        "{warnings:?}"
    );
}

#[tokio::test]
async fn graph_wiring_warnings_is_silent_when_the_downstream_field_is_real() {
    seed_live_catalog_cache("slack", vec![seeded_slack_send_contract()]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "#general", "text": "hi" } } },
            { "id": "xform", "kind": "transform", "name": "Log",
              // `data.ts` — correctly dereferences the Composio execute
              // envelope's `data` wrapper before the real field name.
              "config": { "set": { "note": "=nodes.post.item.json.data.ts" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "xform" }
        ]
    }));
    let warnings = graph_wiring_warnings(&config, &g).await;
    assert!(
        !warnings.iter().any(|w| w.contains("not in")),
        "a real output field must not warn: {warnings:?}"
    );
}

/// B1 regression test: the exact "hollow run" bug. Before this fix, a
/// binding like `=nodes.post.item.json.ts` (a REAL field name, but missing
/// the `data.` segment every Composio `tool_call`'s runtime output wraps its
/// payload in) was silently accepted here — it looks like a legitimate
/// binding to a known output field, but resolves `null` at runtime because
/// the real value lives one level deeper, under `data`. This must now WARN.
#[tokio::test]
async fn graph_wiring_warnings_flags_a_downstream_binding_missing_the_data_prefix() {
    seed_live_catalog_cache("slack", vec![seeded_slack_send_contract()]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "#general", "text": "hi" } } },
            { "id": "xform", "kind": "transform", "name": "Log",
              // `ts` IS a real SLACK_SEND_MESSAGE output field — but without
              // the `data.` prefix this is GUARANTEED to resolve null.
              "config": { "set": { "note": "=nodes.post.item.json.ts" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "xform" }
        ]
    }));
    let warnings = graph_wiring_warnings(&config, &g).await;
    assert!(
        warnings.iter().any(|w| w.contains("item.json.data.ts")
            && w.contains("post")
            && w.contains("wraps its payload in `data`")),
        "{warnings:?}"
    );
}

/// Codex feedback on this PR: a binding to the WHOLE payload
/// (`=nodes.post.item.json.data`, e.g. wiring an agent's `input_context` off
/// the entire tool_call result) must NOT be flagged as "missing the `data.`
/// segment" — it already IS the `data` field, there's nothing to strip a
/// prefix off of. Before this fix the code suggested rewiring to the
/// nonsense `item.json.data.data`.
#[tokio::test]
async fn graph_wiring_warnings_is_silent_for_a_whole_payload_binding() {
    seed_live_catalog_cache("slack", vec![seeded_slack_send_contract()]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "#general", "text": "hi" } } },
            { "id": "xform", "kind": "transform", "name": "Log",
              "config": { "set": { "note": "=nodes.post.item.json.data" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "xform" }
        ]
    }));
    assert!(
        graph_wiring_warnings(&config, &g).await.is_empty(),
        "{:?}",
        graph_wiring_warnings(&config, &g).await
    );
}

/// Codex feedback on this PR: `ComposioExecuteResponse`'s OTHER top-level
/// envelope fields (`successful`, `error`, `costUsd`, `markdownFormatted`)
/// live alongside `data`, not inside it — a binding straight to one of
/// these is real and legitimate. Before this fix the code flagged
/// `.item.json.successful` / `.item.json.error` as missing the `data.`
/// segment and suggested the nonsense `item.json.data.successful`.
#[tokio::test]
async fn graph_wiring_warnings_is_silent_for_composio_envelope_metadata_fields() {
    seed_live_catalog_cache("slack", vec![seeded_slack_send_contract()]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "#general", "text": "hi" } } },
            { "id": "xform", "kind": "transform", "name": "Log",
              "config": { "set": {
                "ok": "=nodes.post.item.json.successful",
                "err": "=nodes.post.item.json.error"
              } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "xform" }
        ]
    }));
    assert!(
        graph_wiring_warnings(&config, &g).await.is_empty(),
        "{:?}",
        graph_wiring_warnings(&config, &g).await
    );
}

#[tokio::test]
async fn graph_wiring_warnings_suggests_the_real_split_out_path() {
    let mut contract = seeded_slack_send_contract();
    contract.slug = "SLACKFANOUT_SEND_MESSAGE".to_string();
    contract.toolkit = "slackfanout".to_string();
    contract.primary_array_path = Some("data.messages".to_string());
    seed_live_catalog_cache("slackfanout", vec![contract]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACKFANOUT_SEND_MESSAGE",
                "args": { "channel": "#general", "text": "hi" } } },
            { "id": "split", "kind": "split_out", "name": "Split",
              "config": { "path": "items" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "split" }
        ]
    }));
    let warnings = graph_wiring_warnings(&config, &g).await;
    assert!(
        warnings.iter().any(|w| w.contains("json.data.messages")),
        "{warnings:?}"
    );
}

/// B12 enforcement: a `split_out.path` that resolves to a NON-array (an
/// object, here) against a KNOWN output schema is flagged even though the
/// action names no array anywhere (`primary_array_path` is `None`) — there
/// is nothing to *suggest*, but a definite non-array hit is still a strong
/// "wrong array path" signal worth catching at build time.
#[tokio::test]
async fn graph_wiring_warnings_flags_a_split_out_path_that_resolves_to_a_non_array() {
    // seeded_slack_send_contract's output_schema names only scalar fields
    // (ts/channel) — a real, known schema with no array in it anywhere.
    let mut contract = seeded_slack_send_contract();
    contract.slug = "NONARRAYFANOUT_SEND_MESSAGE".to_string();
    contract.toolkit = "nonarrayfanout".to_string();
    seed_live_catalog_cache("nonarrayfanout", vec![contract]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "NONARRAYFANOUT_SEND_MESSAGE",
                "args": { "channel": "#general", "text": "hi" } } },
            { "id": "split", "kind": "split_out", "name": "Split",
              "config": { "path": "json.data" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "split" }
        ]
    }));
    let warnings = graph_wiring_warnings(&config, &g).await;
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("split") && w.contains("does not name an array")),
        "{warnings:?}"
    );
}

/// The non-array enforcement stays SILENT when the action's output schema is
/// genuinely unknown (not just "known but arrayless") — nothing real to check
/// the path against, so no false positive.
#[tokio::test]
async fn graph_wiring_warnings_is_silent_on_split_out_when_schema_is_wholly_unknown() {
    let contract = ToolContract {
        slug: "UNKNOWNSCHEMA_DO_THING".to_string(),
        toolkit: "unknownschema".to_string(),
        description: None,
        required_args: vec![],
        input_schema: None,
        output_fields: vec![],
        output_schema: None,
        primary_array_path: None,
        is_curated: true,
    };
    seed_live_catalog_cache("unknownschema", vec![contract]);
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "UNKNOWNSCHEMA_DO_THING", "args": {} } },
            { "id": "split", "kind": "split_out", "name": "Split",
              "config": { "path": "json.data" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "split" }
        ]
    }));
    assert!(
        graph_wiring_warnings(&config, &g).await.is_empty(),
        "{:?}",
        graph_wiring_warnings(&config, &g).await
    );
}

/// B12 end-to-end: the EXACT live bug shape (flow "funny reminders v2").
/// `GITHUB_LIST_REPOSITORY_ISSUES`-equivalent contract has NO schema at all
/// (`output_schema: None`, `primary_array_path: None` — verified live for
/// every GitHub action), so before a probe the enforcement above has nothing
/// to check the configured `"json.data"` against and stays silent. Once
/// `get_tool_output_sample` has probed the slug (seeded here via
/// `seed_probe_cache`, standing in for a real bounded call), the cached
/// `primary_array_path` overrides the schema-derived (absent) hint and the
/// EXISTING mismatch-suggestion path fires with the real nested path.
#[tokio::test]
async fn graph_wiring_warnings_suggests_the_probed_split_out_path_when_schema_is_unknown() {
    let contract = ToolContract {
        slug: "GHPROBEFANOUT_LIST_REPOSITORY_ISSUES".to_string(),
        toolkit: "ghprobefanout".to_string(),
        description: None,
        required_args: vec!["owner".to_string(), "repo".to_string()],
        input_schema: None,
        output_fields: vec![],
        output_schema: None,
        primary_array_path: None,
        is_curated: true,
    };
    seed_live_catalog_cache("ghprobefanout", vec![contract]);
    seed_probe_cache(
        "GHPROBEFANOUT_LIST_REPOSITORY_ISSUES",
        ProbedOutputSample {
            primary_array_path: Some("data.issues".to_string()),
            output_fields: vec!["issues".to_string(), "total_count".to_string()],
            sample: json!({ "data": { "issues": [], "total_count": 0 } }),
        },
    );
    let config = Config::default();
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "GHPROBEFANOUT_LIST_REPOSITORY_ISSUES",
                "args": { "owner": "acme", "repo": "widgets" } } },
            // The exact wrong guess observed live: whole-payload access
            // instead of the real nested `data.issues`.
            { "id": "split", "kind": "split_out", "name": "Split",
              "config": { "path": "json.data" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "split" }
        ]
    }));
    let warnings = graph_wiring_warnings(&config, &g).await;
    assert!(
        warnings.iter().any(|w| w.contains("json.data.issues")),
        "{warnings:?}"
    );

    // Fixed: once config.path matches the probed real path, the warning
    // clears.
    let fixed = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "GHPROBEFANOUT_LIST_REPOSITORY_ISSUES",
                "args": { "owner": "acme", "repo": "widgets" } } },
            { "id": "split", "kind": "split_out", "name": "Split",
              "config": { "path": "json.data.issues" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "split" }
        ]
    }));
    assert!(
        graph_wiring_warnings(&config, &fixed).await.is_empty(),
        "{:?}",
        graph_wiring_warnings(&config, &fixed).await
    );
}

/// CodeRabbit (PR #4702 review): parity coverage for the probe-override path
/// in `graph_output_field_warnings` — mirrors
/// `graph_wiring_warnings_suggests_the_probed_split_out_path_when_schema_is_unknown`
/// above, but for a downstream FIELD binding rather than `split_out.path`.
/// With no schema at all (`output_schema: None`, `output_fields: []`), the
/// field-not-in-output_fields check would otherwise stay silent (nothing
/// real to check against) — once `get_tool_output_sample` has probed the
/// slug, the probed `output_fields` become the ground truth: a binding to a
/// probed-real field is silent, and a binding to a field NOT in the probed
/// set is flagged, exactly like the schema-known case already covers.
#[tokio::test]
async fn graph_wiring_warnings_uses_the_probed_output_fields_when_schema_is_unknown() {
    let contract = ToolContract {
        slug: "GHPROBEFIELDS_LIST_REPOSITORY_ISSUES".to_string(),
        toolkit: "ghprobefields".to_string(),
        description: None,
        required_args: vec!["owner".to_string(), "repo".to_string()],
        input_schema: None,
        output_fields: vec![],
        output_schema: None,
        primary_array_path: None,
        is_curated: true,
    };
    seed_live_catalog_cache("ghprobefields", vec![contract]);
    seed_probe_cache(
        "GHPROBEFIELDS_LIST_REPOSITORY_ISSUES",
        ProbedOutputSample {
            primary_array_path: Some("data.issues".to_string()),
            output_fields: vec!["issues".to_string(), "total_count".to_string()],
            sample: json!({ "data": { "issues": [], "total_count": 0 } }),
        },
    );
    let config = Config::default();

    // A binding to a field the probe actually observed — silent.
    let real_field = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "GHPROBEFIELDS_LIST_REPOSITORY_ISSUES",
                "args": { "owner": "acme", "repo": "widgets" } } },
            { "id": "xform", "kind": "transform", "name": "Log",
              "config": { "set": { "note": "=nodes.post.item.json.data.total_count" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "xform" }
        ]
    }));
    assert!(
        graph_wiring_warnings(&config, &real_field).await.is_empty(),
        "a probed-real field must not warn: {:?}",
        graph_wiring_warnings(&config, &real_field).await
    );

    // A binding to a field the probe did NOT observe — flagged, using the
    // probed output_fields as ground truth even though the schema itself is
    // unknown.
    let fake_field = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "GHPROBEFIELDS_LIST_REPOSITORY_ISSUES",
                "args": { "owner": "acme", "repo": "widgets" } } },
            { "id": "xform", "kind": "transform", "name": "Log",
              "config": { "set": { "note": "=nodes.post.item.json.data.not_a_probed_field" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "post" },
            { "from_node": "post", "to_node": "xform" }
        ]
    }));
    let warnings = graph_wiring_warnings(&config, &fake_field).await;
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("not_a_probed_field") && w.contains("post")),
        "{warnings:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// degrade_completed_status (PR2 — run honesty)
// ─────────────────────────────────────────────────────────────────────────────

fn clean_step(node_id: &str) -> FlowRunStep {
    FlowRunStep {
        node_id: node_id.to_string(),
        output: Value::Null,
        port: None,
        status: Some("success".to_string()),
        duration_ms: Some(1),
        diagnostics: Vec::new(),
    }
}

#[test]
fn degrade_completed_status_all_clean_stays_completed() {
    let steps = vec![clean_step("a"), clean_step("b")];
    assert_eq!(degrade_completed_status(&steps), "completed");
}

#[test]
fn degrade_completed_status_null_binding_becomes_warnings() {
    let mut warned = clean_step("a");
    warned.diagnostics = vec![json!({ "location": "args.to", "expression": "=item.to" })];
    let steps = vec![clean_step("trigger"), warned];
    assert_eq!(degrade_completed_status(&steps), "completed_with_warnings");
}

#[test]
fn degrade_completed_status_errored_step_becomes_failed() {
    let mut errored = clean_step("a");
    errored.status = Some("error".to_string());
    let steps = vec![clean_step("trigger"), errored];
    assert_eq!(degrade_completed_status(&steps), "failed");
}

#[test]
fn degrade_completed_status_error_outranks_diagnostics() {
    // A step can carry both an error status and null-resolution diagnostics
    // (e.g. it errored trying to use the unresolved value) — failed wins.
    let mut errored_with_diagnostics = clean_step("a");
    errored_with_diagnostics.status = Some("error".to_string());
    errored_with_diagnostics.diagnostics =
        vec![json!({ "location": "args.to", "expression": "=item.to" })];
    let steps = vec![errored_with_diagnostics];
    assert_eq!(degrade_completed_status(&steps), "failed");
}

#[test]
fn failed_step_error_summary_none_when_no_step_errored() {
    let steps = vec![clean_step("a"), clean_step("b")];
    assert_eq!(failed_step_error_summary(&steps), None);
}

#[test]
fn failed_step_error_summary_names_the_errored_node() {
    let mut errored = clean_step("x");
    errored.status = Some("error".to_string());
    let steps = vec![clean_step("trigger"), errored];
    let summary = failed_step_error_summary(&steps).expect("an errored step must summarize");
    assert!(summary.contains('x'), "got: {summary}");
}

#[test]
fn failed_step_error_summary_names_every_errored_node() {
    let mut errored_a = clean_step("a");
    errored_a.status = Some("error".to_string());
    let mut errored_b = clean_step("b");
    errored_b.status = Some("error".to_string());
    let steps = vec![errored_a, errored_b];
    let summary = failed_step_error_summary(&steps).unwrap();
    assert!(
        summary.contains('a') && summary.contains('b'),
        "got: {summary}"
    );
}

#[test]
fn envelope_violation_detected() {
    // `summarize` DOES declare a matching schema, but the binding reaches
    // into `.item.channel` (skipping `.json`) — that dereferences the
    // `{json,text,raw}` envelope wrapper itself, not the field inside it.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "prompt": "summarize",
                "output_parser": { "schema": { "type": "object",
                    "properties": { "channel": { "type": "string" } } } } } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "=nodes.summarize.item.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "post" }
        ]
    }));
    let errors = validate_binding_resolvability(&g);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("json"), "{}", errors[0]);
    assert!(errors[0].contains("summarize"), "{}", errors[0]);
}

#[test]
fn non_enveloping_node_binding_is_accepted() {
    // `code` nodes emit their item directly (no envelope) — `.item.<field>`
    // is the correct, and only, form.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "compute", "kind": "code", "name": "Compute",
              "config": { "language": "javascript", "source": "return {channel:'general'};" } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "=nodes.compute.item.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "compute" },
            { "from_node": "compute", "to_node": "post" }
        ]
    }));
    assert!(
        validate_binding_resolvability(&g).is_empty(),
        "{:?}",
        validate_binding_resolvability(&g)
    );
}

#[test]
fn literal_args_unaffected() {
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "general", "count": 3, "cc": ["a@b.com"] } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    assert!(validate_binding_resolvability(&g).is_empty());
}

#[test]
fn agent_prompt_binding_unaffected() {
    // The field-addressability checks are scoped to `tool_call` `args` only
    // — an agent's own `prompt` referencing a dangling/unschemad node path is
    // NOT inspected for that, even though it IS inspected for the narrower
    // "reads as prose, not jq" case (see the tests below). A simple dotted
    // path — even one pointing at a missing node — is a real, valid
    // expression (it just resolves to `null` at runtime, same as any other
    // dangling reference), so it's accepted here.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "prompt": "=nodes.missing.item.channel" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "summarize" } ]
    }));
    assert!(validate_binding_resolvability(&g).is_empty());
}

// ── agent-prompt invalid-jq gate (PR C) ─────────────────────────────────────

#[test]
fn agent_prompt_prose_written_as_expression_is_rejected() {
    // The exact live-failure shape: a builder smuggled upstream data into the
    // prompt via a jq `=`-expression, but the result is prose, not a valid jq
    // program — it resolves to `null` at runtime, handing the agent an empty
    // prompt (the root-cause bug `input_context` exists to fix).
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "classify", "kind": "agent", "name": "Classify",
              "config": { "prompt": "=You are given an email: .item. Classify the following \
                  email as urgent/normal/low priority. Return JSON with fields \"priority\" and \
                  \"reason\"." } }
        ],
        "edges": [ { "from_node": "t", "to_node": "classify" } ]
    }));
    let errors = validate_binding_resolvability(&g);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("classify"), "{}", errors[0]);
    assert!(errors[0].contains("input_context"), "{}", errors[0]);
}

#[test]
fn agent_prompt_jq_concatenation_is_accepted() {
    // A real jq program built from string-literal concatenation is a
    // legitimate, resolvable expression — not the prose failure mode above.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "greet", "kind": "agent", "name": "Greet",
              "config": { "prompt": "=\"Hi \" + .item.name" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "greet" } ]
    }));
    assert!(
        validate_binding_resolvability(&g).is_empty(),
        "{:?}",
        validate_binding_resolvability(&g)
    );
}

#[test]
fn agent_plain_prompt_is_accepted() {
    // No leading `=` at all — an ordinary instruction string, never inspected
    // by this gate regardless of content.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "classify", "kind": "agent", "name": "Classify",
              "config": { "prompt": "Classify the email as urgent, normal, or low priority.",
                "input_context": "=item" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "classify" } ]
    }));
    assert!(validate_binding_resolvability(&g).is_empty());
}

#[test]
fn agent_prompt_with_escaped_quote_inside_jq_string_is_accepted() {
    // Regression for the quote-toggle desync: an escaped quote (`\"`) inside
    // a jq string literal must not flip the strip pass's `in_str` state.
    // Before the fix, the text between the escaped quote and the string's
    // real closing quote ("hello world") leaked out of the string-stripping
    // pass as if it were bare jq code, tripping the "two consecutive
    // barewords" prose heuristic and rejecting this otherwise-valid
    // concatenation expression.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "greet", "kind": "agent", "name": "Greet",
              "config": { "prompt": "=\"Say \\\"hello world\\\" nicely\" + .item.name" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "greet" } ]
    }));
    assert!(
        validate_binding_resolvability(&g).is_empty(),
        "{:?}",
        validate_binding_resolvability(&g)
    );
}

#[test]
fn agent_prose_prompt_with_populated_messages_is_accepted() {
    // Both runtime paths (`build_completion_messages` /
    // `node_request_to_prompt` in `tinyflows/caps.rs`) fall through to a
    // populated `messages` array once `prompt` resolves to `null` — exactly
    // what this prose-as-`=`-expression prompt does. So a node with real
    // `messages` never actually runs on the null prompt; this gate must not
    // reject the graph for a vestigial/unused `prompt` field alongside it.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "classify", "kind": "agent", "name": "Classify",
              "config": {
                  "prompt": "=You are given an email: .item. Classify the following email.",
                  "messages": [ { "role": "user", "content": "Classify this email." } ]
              } }
        ],
        "edges": [ { "from_node": "t", "to_node": "classify" } ]
    }));
    assert!(
        validate_binding_resolvability(&g).is_empty(),
        "{:?}",
        validate_binding_resolvability(&g)
    );
}

#[test]
fn agent_prose_prompt_with_empty_messages_is_still_rejected() {
    // An empty `messages` array doesn't supply the turn at runtime (both
    // `build_completion_messages` and `node_request_to_prompt` treat an empty
    // array the same as absent) — the prose-prompt gate must still apply.
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "classify", "kind": "agent", "name": "Classify",
              "config": {
                  "prompt": "=You are given an email: .item. Classify the following email.",
                  "messages": []
              } }
        ],
        "edges": [ { "from_node": "t", "to_node": "classify" } ]
    }));
    let errors = validate_binding_resolvability(&g);
    assert_eq!(errors.len(), 1, "{errors:?}");
}

#[test]
fn finalize_terminal_status_pending_approval_wins_over_error() {
    // Precedence: an outstanding pending_approval always wins, even if a step
    // also settled with an error — mirrors degrade_completed_status's own
    // precedence rule, now centralized in finalize_terminal_status.
    let mut errored = clean_step("a");
    errored.status = Some("error".to_string());
    let steps = vec![errored];
    let (status, error) = finalize_terminal_status(&steps, &["gate".to_string()]);
    assert_eq!(status, "pending_approval");
    assert_eq!(error, None);
}

#[test]
fn finalize_terminal_status_populates_error_on_degraded_failure() {
    let mut errored = clean_step("x");
    errored.status = Some("error".to_string());
    let steps = vec![errored];
    let (status, error) = finalize_terminal_status(&steps, &[]);
    assert_eq!(status, "failed");
    assert!(error.unwrap().contains('x'));
}

#[test]
fn finalize_terminal_status_no_error_when_clean() {
    let steps = vec![clean_step("a")];
    let (status, error) = finalize_terminal_status(&steps, &[]);
    assert_eq!(status, "completed");
    assert_eq!(error, None);
}

/// Regression for issue #4593: the `flows_build` builder turn runs under
/// `AgentTurnOrigin::Cli`, which makes the `ApprovalGate` auto-allow every
/// `external_effect` tool. The flows live-runner executes a *live* saved flow,
/// so it must be unreachable on this path — `restrict_builder_toolset` drops it
/// from the builder's callable belt while leaving the authoring tools in place
/// so the turn still functions (never fail-closes).
#[tokio::test]
async fn flows_build_hides_the_live_run_tool_from_the_builder_belt() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Document WHY the live-runner must be hidden: running a saved flow fires
    // real Slack/Gmail/HTTP/code effects, so it is an external-effect tool. This
    // pins that invariant independently of belt name-resolution so the
    // hide-list can't silently stop covering a live-run tool.
    use crate::openhuman::tools::Tool as _;
    let live_runner =
        crate::openhuman::flows::tools::RunFlowTool::new(std::sync::Arc::new(config.clone()));
    assert!(
        live_runner.external_effect(),
        "the flows live-runner must be external-effect for the #4593 concern to apply"
    );

    crate::openhuman::agent::harness::AgentDefinitionRegistry::init_global(&config.workspace_dir)
        .expect("agent registry init");
    let mut agent =
        crate::openhuman::agent::Agent::from_config_for_agent(&config, "workflow_builder")
            .expect("build workflow_builder agent");
    agent.set_agent_definition_name("workflow_builder".to_string());

    // Precondition: the builder advertises the live-run tool (`run_flow`) on its
    // belt before restriction — the exact tool #4593 is about.
    assert!(
        agent.visible_tool_names_for_test().contains("run_flow"),
        "precondition: workflow_builder belt should advertise the live-run tool `run_flow`; \
         visible = {:?}",
        agent.visible_tool_names_for_test()
    );

    restrict_builder_toolset(&mut agent);

    // After restriction neither the current name nor the post-rename name is
    // callable on the flows_build path — the hide-list covers both (#4593).
    let visible = agent.visible_tool_names_for_test();
    for hidden in ["run_workflow", "run_flow"] {
        assert!(
            !visible.contains(hidden),
            "live-run tool `{hidden}` must be hidden on the flows_build path; visible = {visible:?}"
        );
    }
    // Authoring / read tools stay reachable so the builder turn still works
    // headlessly under the CLI origin (no fail-close).
    for keep in [
        "propose_workflow",
        "revise_workflow",
        "save_workflow",
        "dry_run_workflow",
        "list_flows",
    ] {
        assert!(
            visible.contains(keep),
            "authoring tool `{keep}` must remain visible after restriction; visible = {visible:?}"
        );
    }
}
