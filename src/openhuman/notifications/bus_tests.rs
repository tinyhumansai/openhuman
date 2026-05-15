//! Additional unit tests for `notifications::bus` — colocated test module.
//!
//! These tests complement the inline `#[cfg(test)] mod tests` block in `bus.rs`
//! and focus on coverage gaps: webhook boundary at exactly status=400, error
//! with sub-400 status, notification id format, deep_link values, and the
//! `publish_core_notification` / `subscribe_core_notifications` broadcast path.

use super::*;
use crate::core::event_bus::DomainEvent;

// ── event_to_notification: webhook boundary conditions ─────────────────────

#[test]
fn webhook_at_exactly_400_emits_system_notification() {
    let ev = DomainEvent::WebhookProcessed {
        tunnel_id: "t".into(),
        skill_id: "edge-skill".into(),
        method: "POST".into(),
        path: "/hook".into(),
        correlation_id: "c".into(),
        status_code: 400,
        elapsed_ms: 10,
        error: None,
    };
    let n = event_to_notification(&ev).expect("status=400 should emit notification");
    assert_eq!(n.category, CoreNotificationCategory::System);
    assert!(
        n.body.contains("400"),
        "body should mention status code 400"
    );
}

#[test]
fn webhook_at_399_with_no_error_is_silent() {
    let ev = DomainEvent::WebhookProcessed {
        tunnel_id: "t".into(),
        skill_id: "ok-skill".into(),
        method: "GET".into(),
        path: "/hook".into(),
        correlation_id: "c".into(),
        status_code: 399,
        elapsed_ms: 5,
        error: None,
    };
    assert!(
        event_to_notification(&ev).is_none(),
        "status<400 with no error should be silent"
    );
}

#[test]
fn webhook_error_with_sub_400_status_still_emits() {
    // The error flag alone is enough to emit even when status < 400.
    let ev = DomainEvent::WebhookProcessed {
        tunnel_id: "t".into(),
        skill_id: "flaky-skill".into(),
        method: "POST".into(),
        path: "/cb".into(),
        correlation_id: "c".into(),
        status_code: 200,
        elapsed_ms: 99,
        error: Some("connection reset".into()),
    };
    let n = event_to_notification(&ev).expect("error flag should emit even at status=200");
    assert_eq!(n.category, CoreNotificationCategory::System);
    assert!(
        n.body.contains("flaky-skill"),
        "skill_id should appear in body"
    );
    assert!(
        n.body.contains("connection reset"),
        "error text should appear in body"
    );
}

#[test]
fn webhook_at_500_with_error_shows_error_text_not_status() {
    // When both status>=400 and error are present, the error branch wins.
    let ev = DomainEvent::WebhookProcessed {
        tunnel_id: "t".into(),
        skill_id: "failing".into(),
        method: "POST".into(),
        path: "/x".into(),
        correlation_id: "c".into(),
        status_code: 502,
        elapsed_ms: 50,
        error: Some("upstream timeout".into()),
    };
    let n = event_to_notification(&ev).expect("should emit");
    assert!(n.body.contains("upstream timeout"), "error message wins");
    // Status is NOT mentioned when error is provided (match branch uses error text)
    assert!(
        !n.body.contains("502"),
        "status code should not appear when error text is present"
    );
}

// ── event_to_notification: id and deep_link invariants ─────────────────────

#[test]
fn cron_notification_id_contains_job_id() {
    let ev = DomainEvent::CronJobCompleted {
        job_id: "daily-report".into(),
        success: true,
        output: "ok".into(),
    };
    let n = event_to_notification(&ev).unwrap();
    assert!(
        n.id.contains("daily-report"),
        "notification id should embed the job_id"
    );
    assert!(
        n.id.starts_with("cron:"),
        "cron notification id should start with cron:"
    );
}

#[test]
fn cron_deep_link_points_to_cron_jobs_settings() {
    let ev = DomainEvent::CronJobCompleted {
        job_id: "j1".into(),
        success: false,
        output: String::new(),
    };
    let n = event_to_notification(&ev).unwrap();
    assert_eq!(
        n.deep_link.as_deref(),
        Some("/settings/cron-jobs"),
        "cron notifications should deep-link to /settings/cron-jobs"
    );
}

#[test]
fn subagent_completed_id_contains_parent_and_task() {
    let ev = DomainEvent::SubagentCompleted {
        parent_session: "sess-abc".into(),
        task_id: "task-xyz".into(),
        agent_id: "researcher".into(),
        elapsed_ms: 120,
        output_chars: 800,
        iterations: 4,
    };
    let n = event_to_notification(&ev).unwrap();
    assert!(
        n.id.contains("sess-abc"),
        "id should contain parent_session"
    );
    assert!(n.id.contains("task-xyz"), "id should contain task_id");
}

#[test]
fn subagent_completed_deep_link_points_to_chat() {
    let ev = DomainEvent::SubagentCompleted {
        parent_session: "p".into(),
        task_id: "t".into(),
        agent_id: "planner".into(),
        elapsed_ms: 50,
        output_chars: 200,
        iterations: 2,
    };
    let n = event_to_notification(&ev).unwrap();
    assert_eq!(n.deep_link.as_deref(), Some("/chat"));
}

#[test]
fn subagent_failed_deep_link_points_to_chat() {
    let ev = DomainEvent::SubagentFailed {
        parent_session: "p".into(),
        task_id: "t".into(),
        agent_id: "worker".into(),
        error: "out of memory".into(),
    };
    let n = event_to_notification(&ev).unwrap();
    assert_eq!(n.deep_link.as_deref(), Some("/chat"));
}

#[test]
fn subagent_failed_truncates_long_error_to_100_chars() {
    let long_error = "x".repeat(200);
    let ev = DomainEvent::SubagentFailed {
        parent_session: "p".into(),
        task_id: "t".into(),
        agent_id: "worker".into(),
        error: long_error.clone(),
    };
    let n = event_to_notification(&ev).unwrap();
    // The implementation takes at most 100 chars from the error.
    let error_part = n.body.replace("worker encountered an error: ", "");
    assert!(
        error_part.chars().count() <= 100,
        "error text should be truncated to 100 chars, got {} chars",
        error_part.chars().count()
    );
}

// ── publish/subscribe broadcast path ───────────────────────────────────────

#[test]
fn publish_and_subscribe_deliver_event() {
    let mut rx = subscribe_core_notifications();

    let evt = CoreNotificationEvent {
        id: "test-123".into(),
        category: CoreNotificationCategory::System,
        title: "Test".into(),
        body: "Test body".into(),
        deep_link: None,
        timestamp_ms: 0,
    };

    let sent = publish_core_notification(evt.clone());
    // At least one subscriber (the one we just created) should receive it.
    assert!(sent >= 1, "should have at least one subscriber");

    // The bus is a process-wide static broadcast channel — when other tests
    // run in parallel and publish their own events, our receiver may see
    // them too. Drain up to N events looking for the one we just sent so
    // this test is order-independent.
    let received = (0..64)
        .find_map(|_| match rx.try_recv() {
            Ok(msg) if msg.id == evt.id => Some(msg),
            Ok(_) => None,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => None,
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => None,
            Err(tokio::sync::broadcast::error::TryRecvError::Closed) => None,
        })
        .expect("subscriber should receive the test event");
    assert_eq!(received.id, "test-123");
    assert_eq!(received.title, "Test");
    assert_eq!(received.category, CoreNotificationCategory::System);
}

#[test]
fn publish_with_no_subscribers_does_not_panic() {
    // Drop any local receiver immediately; the static bus may have zero
    // subscribers at this point (or not — either way must not panic).
    let count = publish_core_notification(CoreNotificationEvent {
        id: "orphan".into(),
        category: CoreNotificationCategory::Agents,
        title: "Orphan".into(),
        body: "nobody is listening".into(),
        deep_link: None,
        timestamp_ms: 42,
    });
    // count is 0 when no subscribers, but the call itself must not panic.
    let _ = count;
}

// ── webhook deep_link ───────────────────────────────────────────────────────

#[test]
fn webhook_error_deep_link_points_to_webhooks_settings() {
    let ev = DomainEvent::WebhookProcessed {
        tunnel_id: "t".into(),
        skill_id: "s".into(),
        method: "POST".into(),
        path: "/hook".into(),
        correlation_id: "c".into(),
        status_code: 500,
        elapsed_ms: 10,
        error: None,
    };
    let n = event_to_notification(&ev).unwrap();
    assert_eq!(n.deep_link.as_deref(), Some("/settings/webhooks-triggers"));
}

// ── NotificationBridgeSubscriber trait contract ─────────────────────────────

#[test]
fn notification_bridge_subscriber_name_is_stable() {
    let sub = NotificationBridgeSubscriber::default();
    // The name is used as the subscription key — must not change
    // silently because callers (loggers, dedup guards) depend on it.
    assert_eq!(sub.name(), "notifications::bridge");
}
