//! Tests for the pure assembly logic.
//!
//! Nothing here builds a real core: `build()` initializes process-global state
//! (keyring, event bus, `Once`-guarded subscribers) that a unit test cannot
//! undo. The end-to-end path is covered by `tests/harness_embed.rs` and
//! `tests/runtime_agents.rs`, which each own their process. The tests that
//! *do* call `build()` are the ones that fail before it.

use super::*;
use openhuman_core::core::runtime::TokenSource;

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
fn a_discovered_build_judges_the_route_by_the_provider_model() {
    let routed = Provider::openai_compatible("https://llm.example", "sk-test");
    // Discovered: no config yet, so the provider's own model decides.
    assert!(routed_provider_effective(
        &routed.clone().model("gpt-test"),
        None
    ));
    assert!(!routed_provider_effective(&routed, None));
    assert!(!routed_provider_effective(
        &Provider::inherit().model("gpt-test"),
        None
    ));
    // Resolved: the assembled config's model decides.
    let mut config = openhuman_core::config::Config {
        default_model: Some("gpt-test".into()),
        ..Default::default()
    };
    assert!(routed_provider_effective(&routed, Some(&config)));
    config.default_model = Some("  ".into());
    assert!(!routed_provider_effective(&routed, Some(&config)));
}

#[test]
fn default_services_start_no_background_writers() {
    // cron, the login-gated services and memory sync each write to the workspace on their
    // own schedule. A library call that started them would become a background
    // process the caller never asked for.
    let services = default_services();
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

#[test]
fn only_selected_services_start_background_work() {
    assert!(!requests_background_services(default_services()));
    assert!(!requests_background_services(ServiceSet::none()));
    assert!(requests_background_services(ServiceSet {
        cron: true,
        ..default_services()
    }));
    assert!(requests_background_services(ServiceSet {
        channels: true,
        ..ServiceSet::none()
    }));
}

#[test]
fn token_defaults_to_env_or_file_and_is_overridable() {
    assert!(matches!(
        RuntimeBuilder::new().token,
        TokenSource::EnvOrFile
    ));
    let fixed = RuntimeBuilder::new().token(TokenSource::Fixed(Arc::new("bearer".into())));
    assert!(matches!(&fixed.token, TokenSource::Fixed(t) if t.as_str() == "bearer"));
}

#[test]
fn listen_records_host_and_port_for_the_transport() {
    let builder = RuntimeBuilder::new();
    assert_eq!(
        (builder.listen_host.as_deref(), builder.listen_port),
        (None, None)
    );
    let builder = RuntimeBuilder::new().listen("0.0.0.0", 9000);
    assert_eq!(builder.listen_host.as_deref(), Some("0.0.0.0"));
    assert_eq!(builder.listen_port, Some(9000));
    let port_only = RuntimeBuilder::new().listen_port(7799);
    assert_eq!(
        (port_only.listen_host, port_only.listen_port),
        (None, Some(7799))
    );
}

#[test]
fn raw_paths_are_recorded_beside_the_workspace_variant() {
    let builder = RuntimeBuilder::new()
        .workspace(Workspace::dir("/srv/oh/root"))
        .workspace_dir("/srv/oh/state")
        .action_dir("/srv/oh/action");
    assert!(matches!(builder.workspace, Workspace::Dir(_)));
    assert_eq!(
        builder.workspace_dir.as_deref(),
        Some(std::path::Path::new("/srv/oh/state"))
    );
    assert_eq!(
        builder.action_dir.as_deref(),
        Some(std::path::Path::new("/srv/oh/action"))
    );
    assert!(builder.validate().is_ok(), "Resolved accepts raw paths");
}

#[test]
fn the_default_config_source_is_resolved() {
    assert_eq!(RuntimeBuilder::new().config_source, ConfigSource::Resolved);
}

#[test]
fn discovered_config_needs_the_inherited_workspace() {
    let err = RuntimeBuilder::new()
        .config_source(ConfigSource::Discovered)
        .validate()
        .expect_err("ephemeral + discovered");
    assert!(
        matches!(err, RuntimeError::Invalid(ref m) if m.contains("Workspace::Inherit")),
        "{err:?}"
    );
}

#[test]
fn discovered_config_refuses_every_knob_that_edits_the_boot_config() {
    let discovered = || {
        RuntimeBuilder::new()
            .workspace(Workspace::Inherit)
            .config_source(ConfigSource::Discovered)
    };
    assert!(discovered().validate().is_ok());
    let cases: Vec<(&str, RuntimeBuilder)> = vec![
        (
            "config",
            discovered().config(openhuman_core::config::Config::default()),
        ),
        (
            "backend_url",
            discovered().backend_url("http://127.0.0.1:1"),
        ),
        (
            "workspace_dir",
            discovered().workspace_dir("/tmp/never-created"),
        ),
        ("action_dir", discovered().action_dir("/tmp/never-created")),
        ("api_key", discovered().api_key("th_live_x")),
    ];
    for (knob, builder) in cases {
        let err = builder.validate().expect_err(knob);
        assert!(
            matches!(err, RuntimeError::Invalid(ref m) if m.contains(knob)),
            "{knob}: {err:?}"
        );
    }
}
