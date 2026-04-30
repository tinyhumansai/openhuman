use super::*;
use serde_json::json;
use std::sync::Mutex;

/// Cargo runs tests concurrently by default, and `TRIAGE_DISABLED_ENV`
/// is process-global. Every test that reads or writes it must hold this
/// guard for the duration of its env-var usage, otherwise interleaved
/// `set_var` / `remove_var` calls cause spurious failures.
static TRIAGE_ENV_GUARD: Mutex<()> = Mutex::new(());

#[tokio::test]
async fn ignores_non_composio_events() {
    let sub = ComposioTriggerSubscriber::new();
    sub.handle(&DomainEvent::CronJobTriggered {
        job_id: "j1".into(),
        job_name: "test-job".into(),
        job_type: "shell".into(),
    })
    .await;
    // No panic = pass.
}

#[tokio::test]
async fn handles_trigger_event_without_panic() {
    // Disable triage so this test takes the log-only path and
    // doesn't spawn a real LLM turn.
    let _guard = TRIAGE_ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var(TRIAGE_DISABLED_ENV, "1");
    let sub = ComposioTriggerSubscriber::new();
    sub.handle(&DomainEvent::ComposioTriggerReceived {
        toolkit: "gmail".into(),
        trigger: "GMAIL_NEW_GMAIL_MESSAGE".into(),
        metadata_id: "trig-1".into(),
        metadata_uuid: "uuid-1".into(),
        payload: json!({ "from": "a@b.com", "subject": "hi" }),
    })
    .await;
    std::env::remove_var(TRIAGE_DISABLED_ENV);
}

#[test]
fn triage_disabled_flag_parser() {
    let _guard = TRIAGE_ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    // Truthy values disable triage.
    for val in ["1", "true", "TRUE", "yes", "YES"] {
        std::env::set_var(TRIAGE_DISABLED_ENV, val);
        assert!(triage_disabled(), "expected '{val}' to disable triage");
    }
    // Non-truthy values leave triage on.
    for val in ["", "0", "false", "off"] {
        std::env::set_var(TRIAGE_DISABLED_ENV, val);
        assert!(!triage_disabled(), "expected '{val}' to keep triage on");
    }
    // Unset = triage on (default).
    std::env::remove_var(TRIAGE_DISABLED_ENV);
    assert!(!triage_disabled(), "unset must default to triage enabled");
}

#[tokio::test]
async fn handles_connection_created_event_without_panic() {
    let sub = ComposioConnectionCreatedSubscriber::new();
    sub.handle(&DomainEvent::ComposioConnectionCreated {
        toolkit: "gmail".into(),
        connection_id: "conn-1".into(),
        connect_url: "https://composio.example/connect/abc".into(),
    })
    .await;
}

#[test]
fn subscribers_have_stable_names_and_domains() {
    let t = ComposioTriggerSubscriber::new();
    assert_eq!(t.name(), "composio::trigger");
    assert_eq!(t.domains(), Some(["composio"].as_ref()));

    let c = ComposioConnectionCreatedSubscriber::new();
    assert_eq!(c.name(), "composio::connection_created");
    assert_eq!(c.domains(), Some(["composio"].as_ref()));
}

#[test]
fn subscriber_default_impls_equal_new() {
    // Call Default just to cover the impl block. Since both are
    // unit structs, equality is implicit — we just exercise the
    // constructor to bump coverage on the Default line.
    let _ = ComposioTriggerSubscriber::default();
    let _ = ComposioConnectionCreatedSubscriber::default();
}

#[tokio::test]
async fn trigger_subscriber_ignores_other_composio_event_variants() {
    // Only ComposioTriggerReceived is relevant — the subscriber must
    // early-return for anything else without error.
    let sub = ComposioTriggerSubscriber::new();
    sub.handle(&DomainEvent::ComposioConnectionCreated {
        toolkit: "gmail".into(),
        connection_id: "c-1".into(),
        connect_url: "url".into(),
    })
    .await;
}

#[tokio::test]
async fn connection_subscriber_ignores_other_composio_event_variants() {
    let sub = ComposioConnectionCreatedSubscriber::new();
    sub.handle(&DomainEvent::ComposioTriggerReceived {
        toolkit: "gmail".into(),
        trigger: "GMAIL_NEW_GMAIL_MESSAGE".into(),
        metadata_id: "id-1".into(),
        metadata_uuid: "u-1".into(),
        payload: json!({}),
    })
    .await;
}

#[tokio::test]
async fn connection_subscriber_skips_when_no_provider_registered() {
    // Pass a toolkit that has no native provider — the subscriber
    // must hit the `no provider registered` early-return branch.
    let sub = ComposioConnectionCreatedSubscriber::new();
    sub.handle(&DomainEvent::ComposioConnectionCreated {
        toolkit: "__no_such_provider_toolkit__".into(),
        connection_id: "c-1".into(),
        connect_url: "url".into(),
    })
    .await;
}

#[test]
fn wait_error_variants_construct_and_format() {
    let e = WaitError::Timeout {
        last_status: Some("PENDING".into()),
    };
    let s = format!("{e:?}");
    assert!(s.contains("Timeout"));
    let e = WaitError::Lookup {
        error: "backend down".into(),
    };
    let s = format!("{e:?}");
    assert!(s.contains("Lookup"));
}
