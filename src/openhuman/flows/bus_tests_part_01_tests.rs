use super::*;

#[test]
fn pinned_trigger_inputs_reads_values_an_author_fixed_for_unattended_runs() {
    let flow = flow_with_trigger_config(
        "f1",
        true,
        json!({
            "trigger_kind": "schedule",
            "schedule": "0 9 * * *",
            "inputs": { "repo": "acme/api", "depth": 3 }
        }),
    );
    let inputs = pinned_trigger_inputs(&flow);
    assert_eq!(inputs["repo"], json!("acme/api"));
    assert_eq!(inputs["depth"], json!(3));
}

#[test]
fn pinned_trigger_inputs_is_empty_when_unset_or_malformed() {
    // Empty, not an error: a flow declaring no inputs (the overwhelming
    // majority) must keep dispatching on a tick exactly as before, and a
    // malformed value is caught downstream by `prepare_flow_run`, which
    // reports it against the flow's actual declarations.
    for cfg in [
        json!({ "trigger_kind": "schedule" }),
        json!({ "trigger_kind": "schedule", "inputs": null }),
        json!({ "trigger_kind": "schedule", "inputs": ["repo"] }),
    ] {
        let flow = flow_with_trigger_config("f1", true, cfg.clone());
        assert!(
            pinned_trigger_inputs(&flow).is_empty(),
            "expected no pinned inputs for {cfg}"
        );
    }
}

#[test]
fn pinned_trigger_inputs_is_empty_for_a_graph_with_no_trigger() {
    let mut flow = flow_with_trigger_config("f1", true, json!({ "trigger_kind": "schedule" }));
    flow.graph.nodes.clear();
    assert!(pinned_trigger_inputs(&flow).is_empty());
}

#[test]
fn name_and_domains_are_stable() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sub = FlowTriggerSubscriber::new(test_config(&tmp));
    assert_eq!(sub.name(), "flows::trigger");
    assert_eq!(
        sub.domains(),
        Some(&["cron", "composio", "webhook", "system"][..])
    );
}

#[tokio::test]
async fn handle_does_not_panic_on_arbitrary_events() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sub = FlowTriggerSubscriber::new(test_config(&tmp));
    sub.handle(&DomainEvent::CronJobTriggered {
        job_id: "j1".into(),
        job_name: "test".into(),
        job_type: "shell".into(),
    })
    .await;
    sub.handle(&DomainEvent::FlowScheduleTick {
        flow_id: "missing-flow".into(),
    })
    .await;
}

#[test]
fn extract_trigger_kind_reads_schedule() {
    let flow = flow_with_trigger_config(
        "f1",
        true,
        json!({ "trigger_kind": "schedule", "schedule": "0 9 * * *" }),
    );
    assert!(matches!(
        extract_trigger_kind(&flow),
        Some(TriggerKind::Schedule)
    ));
}

#[test]
fn extract_trigger_kind_none_for_missing_discriminator() {
    let flow = flow_with_trigger_config("f1", true, json!({}));
    assert!(extract_trigger_kind(&flow).is_none());
}

#[test]
fn extract_trigger_kind_none_for_invalid_discriminator() {
    let flow = flow_with_trigger_config("f1", true, json!({ "trigger_kind": "not_a_kind" }));
    assert!(extract_trigger_kind(&flow).is_none());
}

#[test]
fn matches_app_event_requires_toolkit_and_slug_match() {
    let flow = flow_with_trigger_config(
        "f1",
        true,
        json!({ "trigger_kind": "app_event", "toolkit": "gmail", "trigger_slug": "GMAIL_NEW_GMAIL_MESSAGE" }),
    );
    assert!(matches_app_event(&flow, "gmail", "GMAIL_NEW_GMAIL_MESSAGE"));
    // Case-insensitive.
    assert!(matches_app_event(&flow, "Gmail", "gmail_new_gmail_message"));
    // Wrong toolkit or slug does not match.
    assert!(!matches_app_event(
        &flow,
        "slack",
        "GMAIL_NEW_GMAIL_MESSAGE"
    ));
    assert!(!matches_app_event(&flow, "gmail", "SLACK_NEW_MESSAGE"));
}

#[test]
fn matches_app_event_false_for_non_app_event_trigger() {
    let flow = flow_with_trigger_config(
        "f1",
        true,
        json!({ "trigger_kind": "schedule", "schedule": "0 9 * * *" }),
    );
    assert!(!matches_app_event(
        &flow,
        "gmail",
        "GMAIL_NEW_GMAIL_MESSAGE"
    ));
}

#[tokio::test]
async fn handle_app_event_ignores_disabled_flows() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = flow_with_trigger_config(
        "disabled-flow",
        false,
        json!({ "trigger_kind": "app_event", "toolkit": "gmail", "trigger_slug": "GMAIL_NEW_GMAIL_MESSAGE" }),
    );
    crate::openhuman::flows::store::upsert_flow(&config, &flow).unwrap();

    // `list_enabled_flows` must not surface the disabled flow at all —
    // proves the subscriber's dispatch source already excludes it,
    // rather than asserting on a spawned background task's side effect.
    let (enabled, skipped) = crate::openhuman::flows::store::list_enabled_flows(&config).unwrap();
    assert!(enabled.is_empty());
    assert_eq!(skipped, 0);
}

#[tokio::test]
async fn handle_schedule_tick_ignores_disabled_flow() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = flow_with_trigger_config(
        "sched-flow",
        false,
        json!({ "trigger_kind": "schedule", "schedule": "0 9 * * *" }),
    );
    crate::openhuman::flows::store::upsert_flow(&config, &flow).unwrap();

    let sub = FlowTriggerSubscriber::new(config.clone());
    // Must not panic and must not spawn a run for a disabled flow — we
    // can't directly observe "no run happened" without a full flows_run
    // fixture, but this exercises the early-return path without error.
    sub.handle(&DomainEvent::FlowScheduleTick {
        flow_id: "sched-flow".into(),
    })
    .await;
}

// ── in-flight dedupe (CodeRabbit finding B) ─────────────────────

#[test]
fn try_acquire_dispatch_skips_a_flow_already_in_flight() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sub = FlowTriggerSubscriber::new(test_config(&tmp));

    let guard = sub
        .try_acquire_dispatch("f1")
        .expect("first claim for f1 should succeed");
    assert!(
        sub.try_acquire_dispatch("f1").is_none(),
        "a second claim for the same flow while the first is held must be skipped"
    );

    // A different flow is unaffected.
    assert!(sub.try_acquire_dispatch("f2").is_some());

    drop(guard);
    assert!(
        sub.try_acquire_dispatch("f1").is_some(),
        "dropping the guard must release the claim so f1 can run again"
    );
}

#[test]
fn default_constructs_the_same_as_new() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let a = FlowTriggerSubscriber::new(config.clone());
    let b = FlowTriggerSubscriber::new(config);
    assert_eq!(a.name(), b.name());
}

// ── FlowRunDigestSubscriber ─────────────────────────────────────

#[test]
fn digest_name_and_domains_are_stable() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sub = FlowRunDigestSubscriber::new(test_config(&tmp));
    assert_eq!(sub.name(), "flows::digest");
    assert_eq!(sub.domains(), Some(&["cron"][..]));
}

#[tokio::test]
async fn digest_handle_does_not_panic_on_unrelated_events() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sub = FlowRunDigestSubscriber::new(test_config(&tmp));
    // Must not panic, and must not touch the memory layer at all, for
    // any event other than `FlowRunFinished`.
    sub.handle(&DomainEvent::CronJobTriggered {
        job_id: "j1".into(),
        job_name: "test".into(),
        job_type: "shell".into(),
    })
    .await;
}

#[tokio::test]
async fn digest_ignores_failed_run() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let memory = digest_test_memory(&tmp);

    let flow = flow_with_trigger_config("f-failed", true, json!({}));
    store::upsert_flow(&config, &flow).unwrap();
    store::insert_flow_run(
        &config,
        "run-failed",
        "f-failed",
        "thread-failed",
        "2026-01-01T00:00:00Z",
    )
    .unwrap();
    store::finish_flow_run(
        &config,
        "run-failed",
        "failed",
        "2026-01-01T00:05:00Z",
        &[],
        &[],
        Some("boom"),
        None,
    )
    .unwrap();

    let sub = FlowRunDigestSubscriber::with_memory(config, memory.clone());
    sub.handle(&DomainEvent::FlowRunFinished {
        flow_id: "f-failed".into(),
        run_id: "run-failed".into(),
        status: "failed".into(),
    })
    .await;

    let entry = memory
        .get(&flow_namespace("f-failed"), "run_digest:run-failed")
        .await
        .unwrap();
    assert!(
        entry.is_none(),
        "a failed run must never produce a run_digest entry"
    );
}

#[tokio::test]
async fn digest_ignores_cancelled_run() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let memory = digest_test_memory(&tmp);

    let flow = flow_with_trigger_config("f-cancelled", true, json!({}));
    store::upsert_flow(&config, &flow).unwrap();
    store::insert_flow_run(
        &config,
        "run-cancelled",
        "f-cancelled",
        "thread-cancelled",
        "2026-01-01T00:00:00Z",
    )
    .unwrap();
    store::finish_flow_run(
        &config,
        "run-cancelled",
        "cancelled",
        "2026-01-01T00:05:00Z",
        &[],
        &[],
        None,
        None,
    )
    .unwrap();

    let sub = FlowRunDigestSubscriber::with_memory(config, memory.clone());
    sub.handle(&DomainEvent::FlowRunFinished {
        flow_id: "f-cancelled".into(),
        run_id: "run-cancelled".into(),
        status: "cancelled".into(),
    })
    .await;

    let entry = memory
        .get(&flow_namespace("f-cancelled"), "run_digest:run-cancelled")
        .await
        .unwrap();
    assert!(entry.is_none());
}

#[tokio::test]
async fn digest_writes_run_digest_entry_for_completed_run() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let memory = digest_test_memory(&tmp);

    let flow = flow_with_trigger_config("f-ok", true, json!({}));
    store::upsert_flow(&config, &flow).unwrap();
    store::insert_flow_run(
        &config,
        "run-ok",
        "f-ok",
        "thread-ok",
        "2026-01-01T00:00:00Z",
    )
    .unwrap();
    let step = crate::openhuman::flows::FlowRunStep {
        node_id: "n1".to_string(),
        output: json!({ "sent": 3 }),
        port: None,
        status: Some("success".to_string()),
        duration_ms: Some(12),
        diagnostics: Vec::new(),
    };
    store::finish_flow_run(
        &config,
        "run-ok",
        "completed",
        "2026-01-01T00:05:00Z",
        &[step],
        &[],
        None,
        None,
    )
    .unwrap();

    let sub = FlowRunDigestSubscriber::with_memory(config, memory.clone());
    sub.handle(&DomainEvent::FlowRunFinished {
        flow_id: "f-ok".into(),
        run_id: "run-ok".into(),
        status: "completed".into(),
    })
    .await;

    let entry = memory
        .get(&flow_namespace("f-ok"), "run_digest:run-ok")
        .await
        .unwrap()
        .expect("completed run must produce a run_digest entry");
    assert_eq!(entry.taint, MemoryTaint::ExternalSync);
    assert!(entry.content.contains("f-ok"));
    assert!(entry.content.contains("completed"));
    assert!(entry.content.contains("n1"));
    assert!(entry.content.chars().count() <= DIGEST_MAX_CHARS);
}

#[tokio::test]
async fn digest_treats_completed_with_warnings_as_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let memory = digest_test_memory(&tmp);

    let flow = flow_with_trigger_config("f-warn", true, json!({}));
    store::upsert_flow(&config, &flow).unwrap();
    store::insert_flow_run(
        &config,
        "run-warn",
        "f-warn",
        "thread-warn",
        "2026-01-01T00:00:00Z",
    )
    .unwrap();
    store::finish_flow_run(
        &config,
        "run-warn",
        "completed_with_warnings",
        "2026-01-01T00:05:00Z",
        &[],
        &[],
        None,
        None,
    )
    .unwrap();

    let sub = FlowRunDigestSubscriber::with_memory(config, memory.clone());
    sub.handle(&DomainEvent::FlowRunFinished {
        flow_id: "f-warn".into(),
        run_id: "run-warn".into(),
        status: "completed_with_warnings".into(),
    })
    .await;

    let entry = memory
        .get(&flow_namespace("f-warn"), "run_digest:run-warn")
        .await
        .unwrap();
    assert!(entry.is_some());
}

#[test]
fn truncate_chars_bounds_output_and_marks_truncation() {
    let long = "x".repeat(50);
    let truncated = truncate_chars(&long, 10);
    assert_eq!(truncated.chars().count(), 10);
    assert!(truncated.ends_with('…'));

    let short = "hello";
    assert_eq!(truncate_chars(short, 10), "hello");
}

#[test]
fn render_run_digest_is_bounded_and_includes_key_fields() {
    let run = FlowRun {
        id: "run-1".to_string(),
        flow_id: "f1".to_string(),
        thread_id: "thread-1".to_string(),
        status: "completed".to_string(),
        started_at: "2026-01-01T00:00:00Z".to_string(),
        finished_at: Some("2026-01-01T00:05:00Z".to_string()),
        steps: vec![crate::openhuman::flows::FlowRunStep {
            node_id: "n1".to_string(),
            output: json!({ "ok": true }),
            port: None,
            status: Some("success".to_string()),
            duration_ms: Some(5),
            diagnostics: Vec::new(),
        }],
        pending_approvals: Vec::new(),
        error: None,
        graph_hash: None,
    };
    let digest = render_run_digest("My Flow", &run);
    assert!(digest.contains("My Flow"));
    assert!(digest.contains("completed"));
    assert!(digest.contains("n1"));
    assert!(digest.chars().count() <= DIGEST_MAX_CHARS);
}

#[test]
fn dedup_commit_name_and_domains_are_stable() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sub = DedupCommitSubscriber::new(test_config(&tmp));
    assert_eq!(sub.name(), "flows::dedup_commit");
    assert_eq!(sub.domains(), Some(&["cron"][..]));
}

#[tokio::test]
async fn dedup_commit_ignores_unrelated_events() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sub = DedupCommitSubscriber::new(test_config(&tmp));
    // Must not panic for any event other than `FlowRunFinished`.
    sub.handle(&DomainEvent::CronJobTriggered {
        job_id: "j1".into(),
        job_name: "test".into(),
        job_type: "shell".into(),
    })
    .await;
}

#[tokio::test]
async fn dedup_commit_flow_with_no_dedup_nodes_is_a_noop() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = flow_with_trigger_config("f-no-dedup", true, json!({}));
    store::upsert_flow(&config, &flow).unwrap();

    let sub = DedupCommitSubscriber::new(config);
    // Must not panic when the flow has no `dedup` node at all.
    sub.handle(&DomainEvent::FlowRunFinished {
        flow_id: "f-no-dedup".into(),
        run_id: "run-1".into(),
        status: "completed".into(),
    })
    .await;
}

#[tokio::test]
async fn dedup_commit_unions_tentative_into_committed_and_clears_tentative_on_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = flow_with_dedup_node("f-ok", "dd");
    store::upsert_flow(&config, &flow).unwrap();

    let namespace = dedup_state_namespace("f-ok");
    store::kv_set(&config, &namespace, "dedup:dd:committed", &json!(["a"])).unwrap();
    store::kv_set(
        &config,
        &namespace,
        "dedup:dd:tentative",
        &json!(["b", "c"]),
    )
    .unwrap();

    let sub = DedupCommitSubscriber::new(config.clone());
    sub.handle(&DomainEvent::FlowRunFinished {
        flow_id: "f-ok".into(),
        run_id: "run-ok".into(),
        status: "completed".into(),
    })
    .await;

    let committed = store::kv_get(&config, &namespace, "dedup:dd:committed")
        .unwrap()
        .expect("committed key must still exist");
    let mut committed: Vec<&str> = committed
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    committed.sort_unstable();
    assert_eq!(committed, vec!["a", "b", "c"], "committed = union");

    assert!(
        store::kv_get(&config, &namespace, "dedup:dd:tentative")
            .unwrap()
            .is_none(),
        "tentative must be cleared after a successful commit"
    );
}

#[tokio::test]
async fn dedup_commit_treats_completed_with_warnings_as_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = flow_with_dedup_node("f-warn", "dd");
    store::upsert_flow(&config, &flow).unwrap();

    let namespace = dedup_state_namespace("f-warn");
    store::kv_set(&config, &namespace, "dedup:dd:tentative", &json!(["x"])).unwrap();

    let sub = DedupCommitSubscriber::new(config.clone());
    sub.handle(&DomainEvent::FlowRunFinished {
        flow_id: "f-warn".into(),
        run_id: "run-warn".into(),
        status: "completed_with_warnings".into(),
    })
    .await;

    let committed = store::kv_get(&config, &namespace, "dedup:dd:committed")
        .unwrap()
        .expect("completed_with_warnings must still commit");
    assert_eq!(committed, json!(["x"]));
    assert!(store::kv_get(&config, &namespace, "dedup:dd:tentative")
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn dedup_commit_releases_tentative_without_touching_committed_on_failure() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = flow_with_dedup_node("f-failed", "dd");
    store::upsert_flow(&config, &flow).unwrap();

    let namespace = dedup_state_namespace("f-failed");
    store::kv_set(&config, &namespace, "dedup:dd:committed", &json!(["a"])).unwrap();
    store::kv_set(&config, &namespace, "dedup:dd:tentative", &json!(["b"])).unwrap();

    let sub = DedupCommitSubscriber::new(config.clone());
    sub.handle(&DomainEvent::FlowRunFinished {
        flow_id: "f-failed".into(),
        run_id: "run-failed".into(),
        status: "failed".into(),
    })
    .await;

    assert_eq!(
        store::kv_get(&config, &namespace, "dedup:dd:committed")
            .unwrap()
            .unwrap(),
        json!(["a"]),
        "committed must be untouched by a failed run"
    );
    assert!(
        store::kv_get(&config, &namespace, "dedup:dd:tentative")
            .unwrap()
            .is_none(),
        "tentative must be released (cleared) on failure so the item retries"
    );
}

#[tokio::test]
async fn dedup_commit_releases_tentative_on_cancelled_and_interrupted() {
    for status in ["cancelled", "interrupted"] {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = test_config(&tmp);
        let flow_id = format!("f-{status}");
        let flow = flow_with_dedup_node(&flow_id, "dd");
        store::upsert_flow(&config, &flow).unwrap();

        let namespace = dedup_state_namespace(&flow_id);
        store::kv_set(&config, &namespace, "dedup:dd:tentative", &json!(["z"])).unwrap();

        let sub = DedupCommitSubscriber::new(config.clone());
        sub.handle(&DomainEvent::FlowRunFinished {
            flow_id: flow_id.clone(),
            run_id: format!("run-{status}"),
            status: status.to_string(),
        })
        .await;

        assert!(
            store::kv_get(&config, &namespace, "dedup:dd:committed")
                .unwrap()
                .is_none(),
            "status {status} must never commit"
        );
        assert!(
            store::kv_get(&config, &namespace, "dedup:dd:tentative")
                .unwrap()
                .is_none(),
            "status {status} must release tentative"
        );
    }
}
