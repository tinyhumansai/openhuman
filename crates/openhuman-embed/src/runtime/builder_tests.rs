//! Tests for the pure assembly logic.
//!
//! Nothing here builds a real core: `build()` initializes process-global state
//! (keyring, event bus, `Once`-guarded subscribers) that a unit test cannot
//! undo. The end-to-end path is covered by `tests/harness_embed.rs` and
//! `tests/runtime_agents.rs`, which each own their process. The tests that
//! *do* call `build()` are the ones that fail before it.

use super::*;

#[test]
fn a_runtime_identifies_as_a_library_host_by_default() {
    assert_eq!(RuntimeBuilder::new().host_kind, HostKind::Library);
}

#[test]
fn inherited_workspace_without_a_host_credential_keeps_installed_session_policy() {
    assert_eq!(
        effective_host_kind(HostKind::Library, true, false),
        HostKind::Cli
    );
    assert_eq!(
        effective_host_kind(HostKind::Library, true, true),
        HostKind::Library
    );
    assert_eq!(
        effective_host_kind(HostKind::Library, false, false),
        HostKind::Library
    );
}

#[test]
fn default_services_start_no_background_writers() {
    // cron, the login-gated services and memory sync each write to the workspace on their
    // own schedule. A library call that started them would become a background
    // process the caller never asked for.
    let services = default_services();
    assert!(services.harness_init, "the agent harness must be prepared");
    assert!(!services.cron);
    assert!(!services.login_gated);
    assert!(!services.memory_sync);
    assert!(!services.rpc_http, "a library call binds no port");
    assert!(!services.socketio);
    assert!(!services.channels);
    assert!(!services.update_scheduler);
}

#[test]
fn default_domains_register_what_agents_can_only_narrow() {
    let domains = default_domains();
    assert!(domains.inference, "turns need the inference family");
    #[cfg(feature = "mcp")]
    assert!(domains.mcp, "agents cannot enable mcp later");
    #[cfg(feature = "skills")]
    assert!(domains.skills, "agents cannot enable skills later");
}

#[tokio::test]
async fn a_blank_api_key_is_refused_before_the_slot_is_claimed() {
    let err = RuntimeBuilder::new()
        .api_key("   ")
        .build()
        .await
        .expect_err("blank key");
    assert!(matches!(err, RuntimeError::BlankApiKey), "{err:?}");
    assert!(!RUNTIME_LIVE.load(std::sync::atomic::Ordering::Acquire));
}
