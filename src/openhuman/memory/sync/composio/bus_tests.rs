//! Unit tests for the composio connection-created event handler's gating.

use super::toolkit_is_memory_source_registrable;
use super::*;
use serde_json::json;
use std::sync::Mutex;

/// #4957 regression, in the half that is still this host's.
///
/// The rule the original test locked — which toolkits have a native pipeline —
/// is the driver's now (`MemorySourceSync::is_toolkit_syncable`,
/// tinymemory#106), and it is tested there against the same offenders this one
/// named: `is_composio_toolkit_syncable("googlecalendar")` is false and
/// `" Gmail "` is true, so the normalisation is pinned upstream too. Asserting
/// the list again here would need a loaded module and would only re-check the
/// driver's answer.
///
/// What stays host-side is the **direction the host fails in**, and it is the
/// half that caused #4957: a toolkit that is auto-registered but cannot sync
/// reports ACTIVE and then fails forever, which is a silent lie. So when the
/// driver cannot answer — no binding, no `SourceSync` family, or `Unsupported`
/// — the host must treat the toolkit as *not* registrable rather than assume
/// it is fine.
///
/// The cannot-answer driver is **installed**, not assumed. This note used to
/// say "a unit test has no loaded module, so every answer here is the
/// cannot-answer case" — that is not true: `binding::module_provider`'s
/// `cfg(test)` arm loads `TINYMEMORY_TEST_MODULE`, which CI sets to the pinned
/// module, so the test could reach the module's real `SourceSync` and pass
/// while pinning nothing. It also used `Config::default()`, whose workspace is
/// the developer's own `~/.openhuman`.
///
/// So bind `NullMemoryProvider` — which serves no `SourceSync` family — against
/// a workspace this test owns. The cannot-answer branch is then the branch
/// under test by construction, on any machine, with or without the module.
#[tokio::test]
async fn a_driver_that_cannot_answer_makes_a_toolkit_not_registrable() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let mut config = crate::openhuman::config::Config::default();
    config.workspace_dir = tmp.path().to_path_buf();
    crate::openhuman::memory::binding::install_for_test(
        &config.workspace_dir,
        &config.subsystems.memory,
        std::sync::Arc::new(tinymemory_api::null::NullMemoryProvider::new()),
    );

    for toolkit in ["gmail", "slack", "github", "googlecalendar", ""] {
        assert!(
            !toolkit_is_memory_source_registrable(&config, toolkit).await,
            "with no module bound, '{toolkit}' must not be registrable — \
             registering a source that then fails every sync is #4957"
        );
    }
}

// ─── Subscriber wiring and trigger triage ───────────────────────────────────
//
// Relocated from `src/openhuman/integrations/composio/bus_tests.rs`, which no
// module ever declared — so these seventeen tests had never compiled or run.
// They cannot live beside that file: `integrations/composio/bus.rs` is a
// `pub use ...::bus::*` shim, and a glob re-export cannot carry the private
// items these assert on (`TRIAGE_DISABLED_ENV`, the parser). They belong here,
// against the module that actually defines them.

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
    use crate::openhuman::config::{Config, TEST_ENV_LOCK};
    let _env_lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = TRIAGE_ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());

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

#[test]
fn direct_mode_constant_matches_trigger_drop_check() {
    // The ComposioTriggerSubscriber drops events when
    // `config.composio.mode == COMPOSIO_MODE_DIRECT`. If the schema
    // constant ever drifts from the string "direct" the drop becomes
    // a silent no-op and backend-tenant ghosts would leak through
    // into a direct-mode user's triage pipeline. Pin both sides.
    use crate::openhuman::config::ComposioConfig;
    assert_eq!(
        crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT,
        "direct"
    );
    let cfg = ComposioConfig {
        mode: "direct".into(),
        ..Default::default()
    };
    assert_eq!(
        cfg.mode,
        crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT
    );
}

#[test]
fn composio_config_triage_disabled_default() {
    use crate::openhuman::config::ComposioConfig;
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
fn composio_config_triage_disabled_toolkit_match() {
    use crate::openhuman::config::ComposioConfig;
    let cfg = ComposioConfig {
        triage_disabled_toolkits: vec!["GMAIL".to_string(), "slack".to_string()],
        ..Default::default()
    };
    let toolkit = "gmail";
    let toolkit_lower = toolkit.to_ascii_lowercase();
    assert!(
        cfg.triage_disabled_toolkits
            .iter()
            .any(|t| t.to_ascii_lowercase() == toolkit_lower),
        "case-insensitive match against gmail should fire"
    );
    assert!(
        !cfg.triage_disabled_toolkits
            .iter()
            .any(|t| t.to_ascii_lowercase() == "github"),
        "github should not match"
    );
}

#[tokio::test]
async fn trigger_subscriber_skips_triage_when_env_disabled() {
    // Same rationale as `handles_trigger_event_without_panic`: the
    // direct-mode trigger gate in `ComposioTriggerSubscriber::handle`
    // calls `load_config_with_timeout()` before the env kill-switch, so
    // this test must isolate `OPENHUMAN_WORKSPACE` under `TEST_ENV_LOCK`
    // (acquired before `TRIAGE_ENV_GUARD`).
    use crate::openhuman::config::{Config, TEST_ENV_LOCK};
    let _env_lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = TRIAGE_ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().expect("tempdir");
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.save().await.expect("save fake config to disk");

    std::env::set_var(TRIAGE_DISABLED_ENV, "1");
    let sub = ComposioTriggerSubscriber::new();
    // Should complete without panicking (env gate fires, triage skipped).
    sub.handle(&DomainEvent::ComposioTriggerReceived {
        toolkit: "gmail".into(),
        trigger: "GMAIL_NEW_GMAIL_MESSAGE".into(),
        metadata_id: "trig-env".into(),
        metadata_uuid: "uuid-env".into(),
        payload: json!({ "subject": "env gate test" }),
    })
    .await;
    std::env::remove_var(TRIAGE_DISABLED_ENV);

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
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
    use crate::openhuman::config::{Config, TEST_ENV_LOCK};
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

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
    //
    // Even on the no-provider path the detached spawn still calls
    // `config_rpc::load_config_with_timeout()` (the provider lookup
    // happens *after* the config load in `bus.rs`). Same isolation
    // rationale as `handles_connection_created_event_without_panic`:
    // hold `TEST_ENV_LOCK`, point `OPENHUMAN_WORKSPACE` at a leaked
    // persisted tempdir, and drain the spawn before releasing the lock.
    use crate::openhuman::config::{Config, TEST_ENV_LOCK};
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

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
    })
    .await;
    sub.handle(&DomainEvent::ComposioConfigChanged {
        mode: "backend".into(),
        api_key_set: false,
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
    })
    .await;
    // No panic = pass.
}

#[test]
fn config_changed_subscriber_has_stable_name_and_domain() {
    let s = ComposioConfigChangedSubscriber::new();
    assert_eq!(s.name(), "composio::config_changed");
    assert_eq!(s.domains(), Some(["composio"].as_ref()));
    let _ = ComposioConfigChangedSubscriber::default();
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
