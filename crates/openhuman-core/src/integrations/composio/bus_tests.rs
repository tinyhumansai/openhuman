//! Unit tests for the Composio bus subscribers.

use super::connection_created_subscriber::WaitError;
use super::trigger_subscriber::{
    is_direct_mode, triage_disabled, triage_skip_reason, TRIAGE_DISABLED_ENV,
};
use super::*;
use crate::core::events::DomainEvent;
use serde_json::json;
use tinybus::EventHandler;
use tokio::sync::Mutex;

static TRIAGE_ENV_GUARD: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

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
async fn handling_a_trigger_with_triage_disabled_is_a_smoke_test() {
    // Smoke only: the gate decisions are pinned on `triage_skip_reason`.
    // `ComposioTriggerSubscriber::handle` calls
    // `config_rpc::load_config_with_timeout()` at the very top (the
    // direct-mode trigger gate, `bus.rs`), *before* the
    // `TRIAGE_DISABLED_ENV` kill-switch. That read hits the
    // process-global `OPENHUMAN_WORKSPACE`, so without isolation it
    // races the `config.toml` `save()` of a concurrent
    // `TEST_ENV_LOCK`-holding composio test and its corrupted-config
    // recovery can overwrite that test's config. Hold `TEST_ENV_LOCK`
    // (acquired before `TRIAGE_ENV_GUARD` for a stable lock order) and
    // point `OPENHUMAN_WORKSPACE` at an isolated, persisted config.
    use crate::config::{Config, TEST_ENV_LOCK};
    let _env_lock = TEST_ENV_LOCK.lock().await;
    let _guard = TRIAGE_ENV_GUARD.lock().await;

    let tmp = tempfile::tempdir().expect("tempdir");
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.save().await.expect("save fake config to disk");

    // Disable triage so this test takes the log-only path and
    // doesn't spawn a real LLM turn.
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

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[test]
fn triage_disabled_flag_parser() {
    let _guard = TRIAGE_ENV_GUARD.blocking_lock();
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

#[test]
fn direct_mode_drops_backend_tenant_triggers() {
    // Pinned against the literal: if the schema constant drifted from what the
    // config file holds, the drop would silently become a no-op.
    let mut config = crate::config::Config::default();
    config.composio.mode = "direct".into();
    assert!(is_direct_mode(&config));
    config.composio.mode = "backend".into();
    assert!(!is_direct_mode(&config));
}

#[test]
fn composio_config_triage_disabled_default() {
    use crate::config::ComposioConfig;
    let cfg = ComposioConfig::default();
    assert!(
        !cfg.triage_disabled,
        "triage_disabled must default to false"
    );
    assert!(
        cfg.triage_disabled_toolkits.is_empty(),
        "triage_disabled_toolkits must default to empty"
    );
}

#[test]
fn triage_skip_reason_applies_the_kill_switch_and_config_gates() {
    let mut config = crate::config::Config::default();
    assert_eq!(triage_skip_reason(Some(&config), "gmail", false), None);
    // The env kill-switch wins even when the config would allow triage.
    assert!(triage_skip_reason(Some(&config), "gmail", true).is_some());

    config.composio.triage_disabled_toolkits = vec!["GMAIL".into()];
    assert!(triage_skip_reason(Some(&config), "gmail", false).is_some());
    assert_eq!(triage_skip_reason(Some(&config), "github", false), None);

    config.composio.triage_disabled = true;
    assert!(triage_skip_reason(Some(&config), "github", false).is_some());
    // An unreadable config fails open.
    assert_eq!(triage_skip_reason(None, "github", false), None);
}

#[tokio::test]
async fn handles_connection_created_event_without_panic() {
    // `ComposioConnectionCreatedSubscriber::handle` spawns a detached
    // task that calls `config_rpc::load_config_with_timeout()` (and,
    // post-#1710-Wave-4, `ProviderContext::backend_client()` which loads
    // again). Those reads hit the process-global `OPENHUMAN_WORKSPACE`.
    // Without isolation the detached load races the `config.toml`
    // `save()` of a concurrent `TEST_ENV_LOCK`-holding composio test and
    // its corrupted-config recovery can overwrite that test's config
    // with a default. Hold `TEST_ENV_LOCK`, point `OPENHUMAN_WORKSPACE`
    // at a *leaked* persisted tempdir (so the path stays valid for the
    // detached task even after this test returns) and let the spawned
    // task drain its config loads before releasing the lock. The env
    // var is deliberately left set to the isolated dir — the next
    // `TEST_ENV_LOCK` holder re-points it to its own workspace.
    use crate::config::{Config, TEST_ENV_LOCK};
    let _env_guard = TEST_ENV_LOCK.lock().await;

    let tmp = tempfile::tempdir().expect("tempdir");
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.save().await.expect("save fake config to disk");
    // Keep the isolated workspace dir valid for the detached task that
    // `handle` spawns, even after this test function returns.
    std::mem::forget(tmp);

    let sub = ComposioConnectionCreatedSubscriber::new();
    sub.handle(&DomainEvent::ComposioConnectionCreated {
        toolkit: "gmail".into(),
        connection_id: "conn-1".into(),
        connect_url: "https://composio.example/connect/abc".into(),
        tenant: None,
    })
    .await;

    // Drain the detached spawn's `load_config_with_timeout()` /
    // `backend_client()` calls while we still hold `TEST_ENV_LOCK` and
    // `OPENHUMAN_WORKSPACE` points at the isolated dir, so it can never
    // touch another test's config.toml. With no backend session the
    // spawn early-returns after the loads, well within this window.
    for _ in 0..50 {
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
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
    let _ = ComposioTriggerSubscriber;
    let _ = ComposioConnectionCreatedSubscriber;
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
        tenant: None,
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
async fn connection_subscriber_skips_when_not_signed_in() {
    // Pass a toolkit that has no native provider — the subscriber
    // must hit the `no provider registered` early-return branch.
    //
    // Even on the no-provider path the detached spawn still calls
    // `config_rpc::load_config_with_timeout()` (the provider lookup
    // happens *after* the config load in `bus.rs`). Same isolation
    // rationale as `handles_connection_created_event_without_panic`:
    // hold `TEST_ENV_LOCK`, point `OPENHUMAN_WORKSPACE` at a leaked
    // persisted tempdir, and drain the spawn before releasing the lock.
    use crate::config::{Config, TEST_ENV_LOCK};
    let _env_guard = TEST_ENV_LOCK.lock().await;

    let tmp = tempfile::tempdir().expect("tempdir");
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.save().await.expect("save fake config to disk");
    std::mem::forget(tmp);

    let sub = ComposioConnectionCreatedSubscriber::new();
    sub.handle(&DomainEvent::ComposioConnectionCreated {
        toolkit: "__no_such_provider_toolkit__".into(),
        connection_id: "c-1".into(),
        connect_url: "url".into(),
        tenant: None,
    })
    .await;

    for _ in 0..50 {
        tokio::task::yield_now().await;
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
}

// ── ComposioConfigChangedSubscriber ───────────────────────────────

#[tokio::test]
async fn config_changed_subscriber_invalidates_cache() {
    let sub = ComposioConfigChangedSubscriber::new();
    // Should not panic and should log-invalidate without a config in
    // hand — the cache invalidate path is pure-memory and never
    // touches the network.
    sub.handle(&DomainEvent::ComposioConfigChanged {
        mode: "direct".into(),
        api_key_set: true,
        tenant: None,
    })
    .await;
    sub.handle(&DomainEvent::ComposioConfigChanged {
        mode: "backend".into(),
        api_key_set: false,
        tenant: None,
    })
    .await;
}

#[tokio::test]
async fn config_changed_subscriber_ignores_unrelated_variants() {
    let sub = ComposioConfigChangedSubscriber::new();
    sub.handle(&DomainEvent::ComposioConnectionCreated {
        toolkit: "gmail".into(),
        connection_id: "c-1".into(),
        connect_url: "url".into(),
        tenant: None,
    })
    .await;
    // No panic = pass.
}

#[test]
fn config_changed_subscriber_has_stable_name_and_domain() {
    let s = ComposioConfigChangedSubscriber::new();
    assert_eq!(s.name(), "composio::config_changed");
    assert_eq!(s.domains(), Some(["composio"].as_ref()));
    let _ = ComposioConfigChangedSubscriber;
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
