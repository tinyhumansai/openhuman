use super::*;

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
    assert!(
        !created.value.enabled,
        "issue B29: a schedule-trigger flow must create DISABLED, not armed"
    );
    assert!(
        crate::openhuman::cron::find_flow_schedule_job(&config, &created.value.id)
            .unwrap()
            .is_none(),
        "a disabled-on-create schedule flow must not have its cron job bound yet"
    );

    // The user arms it explicitly — this is where the cron job binds.
    let enabled = flows_set_enabled(&config, &created.value.id, true)
        .await
        .unwrap();
    assert!(enabled.value.enabled);

    let job = crate::openhuman::cron::find_flow_schedule_job(&config, &created.value.id).unwrap();
    assert!(
        job.is_some(),
        "an enabled schedule flow must have its cron job bound immediately on enable"
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
    flows_set_enabled(&config, &created.value.id, true)
        .await
        .unwrap();
    assert!(
        crate::openhuman::cron::find_flow_schedule_job(&config, &created.value.id)
            .unwrap()
            .is_some(),
        "precondition: cron job bound on enable"
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
async fn reconcile_schedule_triggers_on_boot_survives_a_corrupt_row() {
    // R-M4: `reconcile_schedule_triggers_on_boot` is driven by
    // `list_enabled_flows`, which used to hard-fail its entire query on the
    // first corrupt/unmigratable `graph_json` row. One bad enabled flow must
    // not prevent every OTHER enabled schedule-trigger flow from having its
    // cron job re-registered on boot.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let good = flows_create(
        &config,
        "good-scheduled".to_string(),
        schedule_trigger_graph("0 9 * * *"),
        false,
    )
    .await
    .unwrap();
    flows_set_enabled(&config, &good.value.id, true)
        .await
        .unwrap();

    let bad = flows_create(
        &config,
        "bad-scheduled".to_string(),
        schedule_trigger_graph("0 10 * * *"),
        false,
    )
    .await
    .unwrap();
    flows_set_enabled(&config, &bad.value.id, true)
        .await
        .unwrap();
    store::force_corrupt_graph_json_for_test(&config, &bad.value.id, "{ not valid json").unwrap();

    // Remove the cron job `flows_set_enabled` already bound for the good flow
    // above, so the post-reconcile assertion proves
    // `reconcile_schedule_triggers_on_boot` itself re-registered it (rather
    // than the earlier `flows_set_enabled` call, which would pass this
    // assertion even if the boot reconcile silently did nothing).
    let good_job = crate::openhuman::cron::find_flow_schedule_job(&config, &good.value.id)
        .unwrap()
        .expect("precondition: good flow's cron job bound on enable");
    crate::openhuman::cron::remove_job(&config, &good_job.id).unwrap();
    assert!(
        crate::openhuman::cron::find_flow_schedule_job(&config, &good.value.id)
            .unwrap()
            .is_none(),
        "precondition: good flow's cron job removed before reconcile"
    );

    reconcile_schedule_triggers_on_boot(&config)
        .await
        .expect("boot reconciliation must not fail because of one corrupt sibling row");

    assert!(
        crate::openhuman::cron::find_flow_schedule_job(&config, &good.value.id)
            .unwrap()
            .is_some(),
        "the good flow's cron job must be re-registered by boot reconcile despite the \
         corrupt sibling row"
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
    flows_set_enabled(&config, &created.value.id, true)
        .await
        .unwrap();
    let old_job = crate::openhuman::cron::find_flow_schedule_job(&config, &created.value.id)
        .unwrap()
        .expect("cron job bound on enable");
    assert_eq!(old_job.expression, "0 9 * * *");

    flows_update(
        &config,
        &created.value.id,
        None,
        Some(schedule_trigger_graph("30 8 * * *")),
        None,
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
    flows_set_enabled(&config, &created.value.id, true)
        .await
        .unwrap();
    let old_job = crate::openhuman::cron::find_flow_schedule_job(&config, &created.value.id)
        .unwrap()
        .expect("cron job bound on enable");

    // Name-only update: no graph_json supplied, so the trigger cannot have
    // changed — the existing binding must be left untouched.
    flows_update(
        &config,
        &created.value.id,
        Some("renamed".to_string()),
        None,
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

// ── flows_update B29 Rule 1 analogue (save/enable safety on update) ───────
//
// `flows_create` already refuses to persist an automatic-trigger graph as
// `enabled` (Rule 1, above). Live finding: `flows_update` had no equivalent
// — a flow created `enabled: true` with a manual trigger could later have an
// automatic-trigger graph (schedule / app_event / webhook) saved onto it via
// `flows_update` and go LIVE immediately with no user review. These tests
// cover the manual→automatic transition (must disarm), automatic→automatic
// re-edit (must NOT disarm — the user already opted in), and manual→manual
// (never touched).

#[tokio::test]
async fn flows_update_disables_on_manual_to_automatic_trigger_transition_when_enabled() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // A manual-trigger flow persists enabled straight from create (Rule 1
    // only gates automatic triggers).
    let created = flows_create(
        &config,
        "manual-then-scheduled".to_string(),
        manual_trigger_graph(),
        false,
    )
    .await
    .unwrap();
    assert!(created.value.enabled, "manual-trigger flows create enabled");

    // Saving an automatic-trigger graph onto that enabled flow must disarm
    // it — not go live unattended.
    let updated = flows_update(
        &config,
        &created.value.id,
        None,
        Some(schedule_trigger_graph("0 8 * * *")),
        None,
        None,
    )
    .await
    .unwrap();

    assert!(
        !updated.value.enabled,
        "an enabled flow whose trigger just changed from manual to automatic must be \
         auto-disabled, not armed live"
    );
    assert!(
        updated.logs.iter().any(|l| l.contains("auto-disabled")),
        "the disarm must be surfaced in the outcome logs, got: {:?}",
        updated.logs
    );

    // Persisted, not just returned in-memory.
    let reloaded = flows_get(&config, &created.value.id).await.unwrap();
    assert!(!reloaded.value.enabled);

    // And no cron job was left bound — the flow never actually went live.
    assert!(
        crate::openhuman::cron::find_flow_schedule_job(&config, &created.value.id)
            .unwrap()
            .is_none(),
        "an auto-disabled flow must not have its schedule cron job bound"
    );
}

/// Regression: the manual→automatic disarm must apply unconditionally, not
/// only when `flows_update`'s own `existing` read observes `enabled: true`.
/// A live race (Codex, this PR) could leave that read stale — a concurrent
/// `flows_set_enabled(id, true)` landing between the read and the guarded
/// write would previously compute `should_disarm = false` from the stale
/// snapshot and let the automatic graph persist enabled. This test pins the
/// non-racy half of that contract directly at the `flows_update` level: even
/// starting from an *observed* `enabled: false`, a manual→automatic
/// transition still writes the override (a no-op here since the flow was
/// already disabled) rather than skipping it — see
/// `store::update_flow_graph_override_wins_over_concurrently_enabled_row`
/// (store_tests.rs) for the deterministic proof that this override also wins
/// a genuine concurrent-enable race.
#[tokio::test]
async fn flows_update_disarms_manual_to_automatic_transition_even_when_already_disabled() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(
        &config,
        "manual-then-scheduled".to_string(),
        manual_trigger_graph(),
        false,
    )
    .await
    .unwrap();
    flows_set_enabled(&config, &created.value.id, false)
        .await
        .unwrap();

    let updated = flows_update(
        &config,
        &created.value.id,
        None,
        Some(schedule_trigger_graph("0 8 * * *")),
        None,
        None,
    )
    .await
    .unwrap();

    assert!(
        !updated.value.enabled,
        "a manual→automatic transition must never leave the flow enabled, regardless of \
         whether it looked enabled going in"
    );
    let reloaded = flows_get(&config, &created.value.id).await.unwrap();
    assert!(!reloaded.value.enabled);
}

#[tokio::test]
async fn flows_update_preserves_enabled_when_already_automatic() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    // Rule 1 creates an automatic-trigger flow disabled; the user arms it
    // explicitly — this IS the "already reviewed and opted in" state.
    let created = flows_create(
        &config,
        "scheduled".to_string(),
        schedule_trigger_graph("0 9 * * *"),
        false,
    )
    .await
    .unwrap();
    assert!(!created.value.enabled);
    flows_set_enabled(&config, &created.value.id, true)
        .await
        .unwrap();

    // A legitimate re-edit (still an automatic trigger, just a new cron
    // expression) must NOT be treated as a fresh unattended arm.
    let updated = flows_update(
        &config,
        &created.value.id,
        None,
        Some(schedule_trigger_graph("30 8 * * *")),
        None,
        None,
    )
    .await
    .unwrap();

    assert!(
        updated.value.enabled,
        "re-editing an already-enabled automatic-trigger flow must not disarm it — the \
         user already opted in once"
    );
    assert!(!updated.logs.iter().any(|l| l.contains("auto-disabled")));
}

#[tokio::test]
async fn flows_update_preserves_enabled_for_manual_target() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let created = flows_create(&config, "manual".to_string(), manual_trigger_graph(), false)
        .await
        .unwrap();
    assert!(created.value.enabled);

    // manual → manual: no automatic trigger ever enters the picture, so
    // `enabled` must be left completely untouched.
    let mut new_graph = manual_trigger_graph();
    new_graph["name"] = json!("manual-renamed");
    let updated = flows_update(
        &config,
        &created.value.id,
        None,
        Some(new_graph),
        None,
        None,
    )
    .await
    .unwrap();

    assert!(updated.value.enabled);
    assert!(!updated.logs.iter().any(|l| l.contains("auto-disabled")));
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
        serde_json::Map::new(),
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

/// T-M1 end-to-end: a run parks `pending_approval` on the gate node, the user
/// sees an approval card describing the graph as it existed at park time, and
/// `save_workflow` (modeled here via `store::update_flow_graph`, exactly like
/// `flows_resume_marks_an_incompatible_legacy_checkpoint_failed` above models
/// a pre-gate legacy checkpoint) rewrites a downstream node while the approval
/// sits pending. `flows_resume` must refuse — never compile the CURRENT graph
/// against the OLD checkpoint and fire the new config under the stale
/// approval — and must settle the run terminally rather than leave it parked.
#[tokio::test]
async fn flows_resume_refuses_when_the_graph_changed_after_park() {
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
    assert_eq!(pending, vec!["gate".to_string()]);

    // A freshly parked run must have pinned the graph it parked against.
    let parked_row = flows_get_run(&config, &thread_id).await.unwrap().value;
    assert!(
        parked_row.graph_hash.is_some(),
        "a freshly parked run must pin the graph it parked against: {parked_row:?}"
    );

    // Simulate `save_workflow` rewriting the "downstream" node while the
    // approval card the user is looking at still describes the OLD graph.
    let mut rewritten = approval_gated_graph();
    assert_eq!(rewritten["nodes"][2]["id"], "downstream");
    rewritten["nodes"][2]["name"] = json!("Downstream (rewired by save_workflow)");
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

    let error = flows_resume(
        &config,
        &created.value.id,
        &thread_id,
        pending.clone(),
        vec![],
    )
    .await
    .expect_err("resume must refuse once the graph changed after park");
    assert!(
        error.contains("changed after this run was paused"),
        "{error}"
    );

    // Must NOT have executed: the engine must never have run, so "downstream"
    // must not appear among the run's persisted steps.
    let run_row = flows_get_run(&config, &thread_id).await.unwrap().value;
    assert_eq!(run_row.status, "cancelled");
    assert!(
        !run_row.steps.iter().any(|s| s.node_id == "downstream"),
        "the run must not execute the new config under the stale approval: {run_row:?}"
    );
    assert!(
        run_row
            .error
            .as_deref()
            .is_some_and(|e| e.contains("changed after this run was paused")),
        "the terminal run row should retain the refusal reason: {run_row:?}"
    );
    let flow = flows_get(&config, &created.value.id).await.unwrap().value;
    assert_eq!(flow.last_status.as_deref(), Some("cancelled"));

    // A second resume attempt must not succeed either — the checkpoint was
    // dropped, and the row is now terminal, not `pending_approval`.
    let second = flows_resume(&config, &created.value.id, &thread_id, pending, vec![]).await;
    assert!(
        second.is_err(),
        "a settled/refused run must not be resumable again"
    );
}
