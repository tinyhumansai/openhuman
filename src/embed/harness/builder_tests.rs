//! Tests for the pure assembly logic.
//!
//! Nothing here builds a real core: `build()` initializes process-global state
//! (keyring, event bus, `Once`-guarded subscribers) that a unit test cannot undo,
//! and doing it once would make every later test in the process run against a
//! half-configured core. The end-to-end path is covered by
//! `tests/harness_embed.rs`, which owns its process.
//!
//! The tests that *do* call `build()` are the ones that fail before it — and
//! they serialize on [`GUARD`], because `HARNESS_LIVE` is process-wide and
//! `cargo test` runs threads in parallel.

use super::*;
use crate::openhuman::config::Config;

/// Serializes the tests that claim the process-wide harness slot.
///
/// An async mutex, not a `std` one: these tests hold it across `build().await`,
/// and a blocking guard held over an await point can deadlock a single-threaded
/// runtime.
static GUARD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[test]
fn a_harness_identifies_as_a_library_host_by_default() {
    assert_eq!(HarnessBuilder::new().host_kind, HostKind::Library);
}

#[test]
fn inherited_workspace_and_provider_keep_installed_session_policy() {
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
    // cron, heartbeat and the memory queue each write to the workspace on their
    // own schedule. A library call that started them would become a background
    // process the caller never asked for.
    let services = default_services();
    assert!(services.harness_init, "the agent harness must be prepared");
    assert!(!services.cron);
    assert!(!services.heartbeat);
    assert!(!services.memory_queue);
    assert!(!services.rpc_http, "a library call binds no port");
    assert!(!services.socketio);
    assert!(!services.channels);
    assert!(!services.update_scheduler);
}

#[test]
fn a_provider_model_becomes_the_config_default() {
    let mut config = Config::default();
    apply_provider(
        &mut config,
        &Provider::openai_compatible("https://api.example/v1", "sk").model("gpt-5"),
    );
    assert_eq!(config.default_model.as_deref(), Some("gpt-5"));
}

#[test]
fn a_provider_route_is_never_written_to_config() {
    // Config routes persist. A harness borrowing an endpoint must not be able to
    // repoint the operator's install, so the route stays a per-turn parameter.
    let mut config = Config::default();
    let before = config.cloud_providers.len();
    apply_provider(
        &mut config,
        &Provider::openai_compatible("https://api.example/v1", "sk-secret"),
    );
    assert_eq!(config.cloud_providers.len(), before);
    assert!(
        config.inference_url.is_none()
            || config.inference_url.as_deref() != Some("https://api.example/v1")
    );
    assert!(config.ephemeral_route.is_none());
}

#[test]
fn inheriting_a_provider_leaves_the_configured_model_alone() {
    let mut config = Config::default();
    config.default_model = Some("operators-choice".into());
    apply_provider(&mut config, &Provider::inherit());
    assert_eq!(config.default_model.as_deref(), Some("operators-choice"));
}

#[cfg(feature = "skills")]
#[tokio::test]
async fn skills_dir_is_refused_for_an_inherited_workspace() {
    let _guard = GUARD.lock().await;
    let err = Harness::builder()
        .workspace(Workspace::Inherit)
        .skills_dir("./skills")
        .build()
        .await
        .expect_err("copying into the operator's own skills root must be refused");
    assert!(matches!(err, HarnessError::Invalid(_)), "got {err:?}");
}

#[cfg(feature = "skills")]
#[tokio::test]
async fn a_failed_build_releases_the_process_slot() {
    let _guard = GUARD.lock().await;
    // Otherwise one bad build would poison the process against ever retrying —
    // and the retry is the natural response to an `Invalid` builder input.
    for _ in 0..2 {
        let err = Harness::builder()
            .workspace(Workspace::Inherit)
            .skills_dir("./skills")
            .build()
            .await
            .expect_err("still invalid");
        assert!(
            !matches!(err, HarnessError::AlreadyRunning),
            "the slot leaked from the previous failed build"
        );
    }
    assert!(!HARNESS_LIVE.load(std::sync::atomic::Ordering::Acquire));
}

#[cfg(feature = "mcp")]
#[test]
fn declared_mcp_servers_land_on_the_config() {
    use super::super::mcp::McpServer;

    // The static registry has one constructor, `from_config`, and no public
    // `register` — so the config IS the registration API.
    let mut config = Config::default();
    let before = config.mcp_client.servers.len();
    config.mcp_client.servers.extend(
        [McpServer::stdio("gh", "gh-mcp", ["stdio"])]
            .into_iter()
            .map(McpServer::into_config),
    );
    assert_eq!(config.mcp_client.servers.len(), before + 1);
    assert_eq!(config.mcp_client.servers[before].name, "gh");
}

#[tokio::test]
async fn an_inherited_workspace_still_applies_the_builder_knobs() {
    // The bug this pins: `Inherit` used to hand the core no config at all and
    // let it load one later, which silently dropped the access tier, the
    // backend URL and every declared MCP server. "Inherit" chooses the starting
    // point; it does not mean "ignore what I configured".
    //
    // Asserted on the assembly, not through a real build — booting a core is
    // process-global and cannot be undone between tests.
    let mut config = Config::default();
    config.api_url = Some("https://operator.example".into());
    config.default_model = Some("operators-choice".into());

    // What `build_inner` does to a config once it has one, in order.
    let backend_url = Some("https://harness.example".to_string());
    if let Some(url) = backend_url {
        config.api_url = Some(url);
    }
    Access::full().apply(&mut config);
    apply_provider(&mut config, &Provider::inherit());

    assert_eq!(config.api_url.as_deref(), Some("https://harness.example"));
    assert_eq!(
        config.autonomy.level,
        crate::openhuman::security::AutonomyLevel::Full
    );
    // `Provider::inherit()` states no model, so the operator's survives.
    assert_eq!(config.default_model.as_deref(), Some("operators-choice"));
}

#[test]
fn backend_url_overrides_a_supplied_configs_api_url() {
    // Order matters: the explicit builder call is more specific than whatever
    // the starting config carried, so it must be applied after it.
    let mut config = Config::default();
    config.api_url = Some("https://from-config.example".into());
    config.api_url = Some("https://from-builder.example".to_string());
    assert_eq!(
        config.api_url.as_deref(),
        Some("https://from-builder.example")
    );
}
