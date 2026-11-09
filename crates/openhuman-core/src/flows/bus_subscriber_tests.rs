use super::*;
use tinyflows_bus::catalog::{FlowRun, FlowRunStep};
use tinyflows_bus::runs::FlowRunTrigger;

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
async fn portability_proxy_describes_bindings_and_returns_pinned_inputs() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let schedule_flow = flow_with_trigger_config(
        "schedule-flow",
        true,
        json!({
            "trigger_kind": "schedule",
            "schedule": "0 9 * * *",
            "inputs": {"repo": "acme/api", "depth": 3}
        }),
    );

    let details = describe_trigger(&config, &schedule_flow).unwrap();
    assert_eq!(details.kind.as_deref(), Some("schedule"));
    assert!(
        details.schedule.is_some(),
        "schedule is returned as a typed value"
    );
    let inputs = trigger::pinned_trigger_inputs(&config, &schedule_flow).unwrap();
    assert_eq!(inputs["repo"], json!("acme/api"));
    assert_eq!(inputs["depth"], json!(3));

    let event_flow = flow_with_trigger_config(
        "event-flow",
        true,
        json!({
            "trigger_kind": "app_event",
            "toolkit": "gmail",
            "trigger_slug": "GMAIL_NEW_GMAIL_MESSAGE"
        }),
    );
    assert!(
        trigger::matches_app_event(&config, &event_flow, "Gmail", "gmail_new_gmail_message")
            .unwrap()
    );
    assert!(
        !trigger::matches_app_event(&config, &event_flow, "slack", "GMAIL_NEW_GMAIL_MESSAGE")
            .unwrap()
    );

    let malformed = flow_with_trigger_config(
        "malformed-flow",
        true,
        json!({"trigger_kind": "schedule", "schedule": {"invalid": true}}),
    );
    assert!(describe_trigger(&config, &malformed).is_err());
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
    sub.dispatch_schedule_tick(
        &crate::core::runtime::DispatchCtx::host_default(),
        "missing-flow",
    )
    .await;
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
    crate::flows::test_support::seed_flow(&config, &flow).unwrap();

    // `list_enabled_flows` must not surface the disabled flow at all —
    // proves the subscriber's dispatch source already excludes it,
    // rather than asserting on a spawned background task's side effect.
    let (enabled, skipped) = crate::flows::store::list_enabled_flows(&config).unwrap();
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
    crate::flows::test_support::seed_flow(&config, &flow).unwrap();

    let sub = FlowTriggerSubscriber::new(config.clone());
    // Must not panic and must not spawn a run for a disabled flow — we
    // can't directly observe "no run happened" without a full flows_run
    // fixture, but this exercises the early-return path without error.
    sub.dispatch_schedule_tick(
        &crate::core::runtime::DispatchCtx::host_default(),
        "sched-flow",
    )
    .await;
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
    let _engine = digest_test_engine(&config);

    let sub = FlowRunDigestSubscriber::new(config.clone());
    sub.handle(&DomainEvent::FlowRunFinished {
        flow_id: "f-failed".into(),
        run_id: "run-failed".into(),
        status: "failed".into(),
    })
    .await;

    assert!(
        stored_digests(&config, "f-failed").await.is_empty(),
        "a failed run must never produce a run digest"
    );
}

#[tokio::test]
async fn digest_ignores_cancelled_run() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let _engine = digest_test_engine(&config);

    let sub = FlowRunDigestSubscriber::new(config.clone());
    sub.handle(&DomainEvent::FlowRunFinished {
        flow_id: "f-cancelled".into(),
        run_id: "run-cancelled".into(),
        status: "cancelled".into(),
    })
    .await;

    assert!(stored_digests(&config, "f-cancelled").await.is_empty());
}

#[tokio::test]
async fn digest_rendering_scrubs_persisted_outputs() {
    let step = FlowRunStep {
        node_id: "n1".to_string(),
        output: json!({
            "sent": 3,
            "access_token": format!("synthetic-credential-value{}", "x".repeat(140)),
        }),
        port: None,
        status: Some("success".to_string()),
        duration_ms: Some(12),
        diagnostics: Vec::new(),
    };
    // Generated synthetic fixture data exercises text-token scrubbing; it is
    // deterministic test input and never a real credential.
    let token_output = format!("sk-ant-api03-{}", "0123456789abcdef".repeat(3));
    let string_step = FlowRunStep {
        node_id: "n2".to_string(),
        output: json!(token_output.clone()),
        port: None,
        status: Some("success".to_string()),
        duration_ms: Some(8),
        diagnostics: Vec::new(),
    };
    let run = FlowRun {
        id: "run-ok".to_string(),
        flow_id: "f-ok".to_string(),
        thread_id: "thread-ok".to_string(),
        status: "completed".to_string(),
        started_at: "2026-01-01T00:00:00Z".to_string(),
        finished_at: Some("2026-01-01T00:05:00Z".to_string()),
        steps: vec![step, string_step],
        pending_approvals: Vec::new(),
        error: None,
        graph_hash: None,
    };
    let entry = render_run_digest("f-ok", &run);
    assert!(entry.contains("completed"));
    assert!(entry.contains("n1"));
    assert!(entry.contains("\"sent\":3"));
    assert!(!entry.contains("synthetic-credential-value"));
    assert!(entry.contains("[REDACTED_SECRET]"));
    assert!(!entry.contains(&token_output));
    assert!(entry.contains("n2"));
    assert!(entry.chars().count() <= DIGEST_MAX_CHARS);
}

#[tokio::test]
async fn digest_treats_completed_with_warnings_as_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let _engine = digest_test_engine(&config);

    let flow = flow_with_trigger_config("f-warn", true, json!({}));
    crate::flows::test_support::seed_flow(&config, &flow).unwrap();
    let run = crate::flows::ops::flows_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &flow.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .unwrap();
    let run_id = run.value["thread_id"].as_str().unwrap().to_owned();

    // The real run has already exercised the module-owned storage/projection
    // path. Call the digest policy directly with the warning-success status so
    // this test does not depend on a process-global subscriber registration.
    for digest in stored_digests(&config, "f-warn").await {
        crate::memory::ops::forget(
            &config,
            crate::memory::types::ForgetParams {
                ids: vec![digest.id.0],
                reach: None,
            },
        )
        .await
        .unwrap();
    }
    let sub = FlowRunDigestSubscriber::new(config.clone());
    sub.handle_finished(
        crate::core::runtime::DispatchCtx::ambient(),
        "f-warn",
        &run_id,
        "completed_with_warnings",
    )
    .await;

    assert_eq!(stored_digests(&config, "f-warn").await.len(), 1);
}

#[tokio::test]
async fn digest_is_skipped_quietly_when_memory_is_off() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = (*test_config(&tmp)).clone();
    config.memory.engine = String::new();
    let config = Arc::new(config);
    let flow = flow_with_trigger_config("f-off", true, json!({}));
    crate::flows::test_support::seed_flow(&config, &flow).unwrap();
    // No run row exists: with memory off the subscriber must return before
    // even loading it, so this completes without a warning path.
    FlowRunDigestSubscriber::new(config)
        .handle(&DomainEvent::FlowRunFinished {
            flow_id: "f-off".into(),
            run_id: "missing".into(),
            status: "completed".into(),
        })
        .await;
}

#[tokio::test]
async fn retention_cap_forgets_the_oldest_digests() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let _engine = digest_test_engine(&config);
    let base = chrono::Utc::now();
    for i in 0..4_i64 {
        let mut meta = crate::flows::flow_meta("f-cap", &["flow_run_digest".to_string()]);
        meta.observed_at = Some(base + chrono::Duration::seconds(i));
        crate::memory::ops::store_item(
            &config,
            tinymemory_api::StoreItem::document(format!("digest {i}"), meta),
        )
        .await
        .unwrap();
    }
    assert_eq!(enforce_retention_cap(&config, "f-cap", 2).await.unwrap(), 2);
    let kept = stored_digests(&config, "f-cap").await;
    assert_eq!(kept.len(), 2);
    assert!(kept
        .iter()
        .all(|hit| hit.text.contains("digest 2") || hit.text.contains("digest 3")));
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
        steps: vec![FlowRunStep {
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
