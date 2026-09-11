use super::*;

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
        serde_json::Map::new(),
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
async fn flows_list_all_runs_aggregates_across_flows_newest_first() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let a = flows_create(&config, "alpha".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();
    let b = flows_create(&config, "beta".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();

    // Run alpha first, then beta — beta's run is the newest.
    flows_run(
        &config,
        &a.value.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let beta_run = flows_run(
        &config,
        &b.value.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let beta_thread = beta_run.value["thread_id"].as_str().unwrap().to_string();

    let all = flows_list_all_runs(&config, 100).await.unwrap();
    assert_eq!(all.value.len(), 2, "runs from both flows should be listed");
    // Newest first — beta's run leads.
    assert_eq!(all.value[0].id, beta_thread);
    assert_eq!(all.value[0].flow_id, b.value.id);
    // Both flows are represented.
    let flow_ids: std::collections::HashSet<_> =
        all.value.iter().map(|r| r.flow_id.clone()).collect();
    assert!(flow_ids.contains(&a.value.id) && flow_ids.contains(&b.value.id));
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
    let mut rx = crate::openhuman::desktop::notifications::bus::subscribe_core_notifications();

    let created = flows_create(
        &config,
        "gated-notify".to_string(),
        approval_gated_graph(),
        false,
    )
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
        crate::openhuman::desktop::notifications::types::CoreNotificationCategory::Agents
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
    let mut rx = crate::openhuman::desktop::notifications::bus::subscribe_core_notifications();

    let created = flows_create(&config, "no-gate".to_string(), trigger_only_graph(), false)
        .await
        .unwrap();
    let created_id = created.value.id.clone();

    flows_run(
        &config,
        &created.value.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
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

/// Issue B35 (runs-rail live refresh): `flows_run` must publish
/// `DomainEvent::FlowRunStarted` right after the run row is persisted, with
/// the flow id and the run's thread id, so the socket bridge can tell an open
/// Workflows sidebar/drawer to refetch and show "Running" immediately instead
/// of waiting for the (up to 610s) blocking RPC to resolve.
#[tokio::test]
async fn flows_run_publishes_flow_run_started_with_flow_and_run_id() {
    use crate::core::bus::BUS;
    use crate::core::events::DomainEvent;
    use async_trait::async_trait;
    use std::sync::Mutex as StdMutex;
    use tinybus::EventHandler;

    #[derive(Default)]
    struct Collector {
        events: Arc<StdMutex<Vec<(String, String)>>>,
    }

    #[async_trait]
    impl EventHandler<DomainEvent> for Collector {
        fn name(&self) -> &str {
            "test::flows::ops::flow_run_started_collector"
        }
        fn domains(&self) -> Option<&[&str]> {
            Some(&["cron"])
        }
        async fn handle(&self, event: &DomainEvent) {
            if let DomainEvent::FlowRunStarted { flow_id, run_id } = event {
                self.events
                    .lock()
                    .unwrap()
                    .push((flow_id.clone(), run_id.clone()));
            }
        }
    }

    crate::core::bus::init().await.expect("bus init");
    let events: Arc<StdMutex<Vec<(String, String)>>> = Arc::new(StdMutex::new(Vec::new()));
    let collector = Arc::new(Collector {
        events: Arc::clone(&events),
    });
    let _handle = BUS.subscribe(collector).expect("bus subscriber installed");

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(
        &config,
        "b35-run-started".to_string(),
        trigger_only_graph(),
        false,
    )
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

    // The bus is process-global and shared with concurrently-running tests,
    // so filter for our own flow id rather than asserting on total count.
    let mut found = None;
    for _ in 0..20 {
        {
            let guard = events.lock().unwrap();
            if let Some(entry) = guard.iter().find(|(fid, _)| *fid == created.value.id) {
                found = Some(entry.clone());
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let (flow_id, run_id) = found.expect("expected a FlowRunStarted event for this flow");
    assert_eq!(flow_id, created.value.id);
    assert_eq!(run_id, thread_id);
}

/// PR #5115 review finding (Codex): a run that merely pauses at an approval
/// gate must NOT publish `DomainEvent::FlowRunFinished` — only the eventual
/// terminal settle (here, after `flows_resume`) should. `finalize_terminal_status`
/// can return `"pending_approval"`, and `finish_flow_run_row` used to publish
/// unconditionally on every status; since `useFlowRunFinished` de-dupes
/// delivered events by `${flow_id}:${run_id}`, an event fired for the pause
/// would poison that cache and cause the real completion event after resume
/// to be silently dropped as an alias replay. Exercises the full pause ->
/// resume lifecycle and asserts exactly one `FlowRunFinished` is observed,
/// carrying the final `"completed"` status, not `"pending_approval"`.
#[tokio::test]
async fn flows_run_finished_event_skips_pending_approval_and_fires_once_on_resume() {
    use crate::core::bus::BUS;
    use crate::core::events::DomainEvent;
    use async_trait::async_trait;
    use std::sync::Mutex as StdMutex;
    use tinybus::EventHandler;

    #[derive(Default)]
    struct Collector {
        events: Arc<StdMutex<Vec<(String, String, String)>>>,
    }

    #[async_trait]
    impl EventHandler<DomainEvent> for Collector {
        fn name(&self) -> &str {
            "test::flows::ops::flow_run_finished_pending_approval_collector"
        }
        fn domains(&self) -> Option<&[&str]> {
            Some(&["cron"])
        }
        async fn handle(&self, event: &DomainEvent) {
            if let DomainEvent::FlowRunFinished {
                flow_id,
                run_id,
                status,
            } = event
            {
                self.events
                    .lock()
                    .unwrap()
                    .push((flow_id.clone(), run_id.clone(), status.clone()));
            }
        }
    }

    crate::core::bus::init().await.expect("bus init");
    let events: Arc<StdMutex<Vec<(String, String, String)>>> = Arc::new(StdMutex::new(Vec::new()));
    let collector = Arc::new(Collector {
        events: Arc::clone(&events),
    });
    let _handle = BUS.subscribe(collector).expect("bus subscriber installed");

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let created = flows_create(
        &config,
        "b35-finished-skips-pause".to_string(),
        approval_gated_graph(),
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
    let pending: Vec<String> =
        serde_json::from_value(run.value["pending_approvals"].clone()).unwrap();
    assert_eq!(pending, vec!["gate".to_string()]);

    // Give the bus a moment to deliver anything it's going to deliver, then
    // assert the pause produced no FlowRunFinished for this run at all.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    {
        let guard = events.lock().unwrap();
        assert!(
            !guard.iter().any(|(_, rid, _)| *rid == thread_id),
            "a run parked at an approval gate must not publish FlowRunFinished: {guard:?}"
        );
    }

    let resumed = flows_resume(&config, &created.value.id, &thread_id, pending, vec![])
        .await
        .unwrap();
    assert_eq!(resumed.value["pending_approvals"], json!([]));

    // The bus is process-global and shared with concurrently-running tests,
    // so filter for our own run id rather than asserting on total count.
    let mut matched: Vec<(String, String, String)> = Vec::new();
    for _ in 0..20 {
        {
            let guard = events.lock().unwrap();
            matched = guard
                .iter()
                .filter(|(_, rid, _)| *rid == thread_id)
                .cloned()
                .collect();
            if !matched.is_empty() {
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(
        matched.len(),
        1,
        "expected exactly one FlowRunFinished for this run (the post-resume settle, \
         none for the pause): {matched:?}"
    );
    let (flow_id, run_id, status) = matched.into_iter().next().unwrap();
    assert_eq!(flow_id, created.value.id);
    assert_eq!(run_id, thread_id);
    assert_eq!(status, "completed");
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
        transcript: Vec::new(),
    });
    observer.on_step_finish(&ExecutionStep {
        node_id: "b".to_string(),
        status: StepStatus::Error,
        output: Value::Null,
        duration_ms: 3,
        diagnostics: Vec::new(),
        transcript: Vec::new(),
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
        transcript: Vec::new(),
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
        serde_json::Map::new(),
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

    let err = flows_cancel_run(&config, &thread_id)
        .await
        .expect_err("cancelling an already-completed run must be a clear error");
    assert!(err.contains("already terminal"), "got: {err}");
}
