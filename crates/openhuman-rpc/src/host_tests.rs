use super::*;

#[cfg(feature = "server")]
use openhuman_tinyhumans::embed::seams::DomainGroup;
use openhuman_tinyhumans::embed::{DomainSet, HostKind, ServiceSet};

#[cfg(feature = "server")]
#[test]
fn cli_builder_is_the_cli_preset_with_server_and_http_host() {
    let summary = cli_builder().into_embed().summary();
    assert_eq!(summary.host_kind, HostKind::detect_standalone());
    assert_eq!(summary.domains, Some(DomainSet::full()));
    assert_eq!(summary.services, Some(ServiceSet::desktop()));
    assert!(
        !summary.fixed_token,
        "the CLI reads its bearer from env/file"
    );
    assert!(summary.has_server_launcher, "`run`/`serve` need the server");
    assert_eq!(summary.controller_extensions, vec![DomainGroup::Platform]);
}

#[cfg(feature = "server")]
#[test]
fn desktop_builder_carries_bearer_listener_and_services() {
    let options = DesktopOptions {
        host: Some("127.0.0.1".into()),
        port: Some(7799),
        socketio: false,
        rpc_token: Some(Arc::new("launch-bearer".into())),
    };
    let summary = desktop_builder(&options).into_embed().summary();
    assert_eq!(summary.host_kind, HostKind::TauriShell);
    assert_eq!(summary.domains, Some(DomainSet::full()));
    let mut expected = ServiceSet::desktop();
    expected.socketio = false;
    expected.update_scheduler = false;
    assert_eq!(summary.services, Some(expected));
    assert!(summary.fixed_token);
    assert_eq!(summary.listen_host.as_deref(), Some("127.0.0.1"));
    assert_eq!(summary.listen_port, Some(7799));
    assert!(summary.has_server_launcher);
    assert_eq!(summary.controller_extensions, vec![DomainGroup::Platform]);
    assert!(
        !summary.has_session_store,
        "the server installs the store for the process, not per runtime"
    );
}

#[cfg(feature = "server")]
#[test]
fn desktop_defaults_leave_listener_and_token_to_the_environment() {
    let options = DesktopOptions::default();
    assert!(options.socketio);
    let summary = desktop_builder(&options).into_embed().summary();
    assert!(!summary.fixed_token);
    assert_eq!(summary.listen_host, None);
    assert_eq!(summary.listen_port, None);
    let mut expected = ServiceSet::desktop();
    expected.update_scheduler = false;
    assert_eq!(summary.services, Some(expected));
}

/// Sentry TAURI-RUST-122R/122S/13B8/13B9: the shell updates through the
/// Tauri updater and releases publish core archives for Linux only, so an
/// in-process core polling for its own archive reported a missing asset
/// every hour on macOS and Windows. The standalone CLI keeps the poller.
#[cfg(feature = "server")]
#[test]
fn only_the_standalone_cli_runs_the_core_update_poller() {
    let desktop = desktop_builder(&DesktopOptions::default())
        .into_embed()
        .summary();
    assert!(
        !desktop
            .services
            .expect("desktop sets services")
            .update_scheduler
    );
    let cli = cli_builder().into_embed().summary();
    assert!(cli.services.expect("cli sets services").update_scheduler);
}

#[cfg(feature = "server")]
#[test]
fn desktop_options_debug_redacts_the_bearer() {
    let options = DesktopOptions {
        rpc_token: Some(Arc::new("secret-bearer".into())),
        ..DesktopOptions::default()
    };
    let rendered = format!("{options:?}");
    assert!(!rendered.contains("secret-bearer"), "{rendered}");
    assert!(rendered.contains("<redacted>"), "{rendered}");
}

#[cfg(feature = "server")]
#[test]
fn desktop_connected_builder_binds_the_transport_and_hosted_controllers() {
    let summary = desktop_builder(&DesktopOptions::default())
        .connect()
        .expect("SDK transport builds offline")
        .summary();
    assert!(summary.controller_extensions.contains(&DomainGroup::Hosted));
    assert!(summary
        .controller_extensions
        .contains(&DomainGroup::Platform));
}

#[cfg(feature = "session-store")]
#[test]
fn tui_builder_runs_every_domain_without_services_and_leaves_the_session_store_to_tui() {
    let summary = tui_builder().into_embed().summary();
    assert_eq!(summary.host_kind, HostKind::detect_standalone());
    assert_eq!(summary.domains, Some(DomainSet::full()));
    assert_eq!(summary.services, Some(ServiceSet::none()));
    assert!(
        !summary.has_session_store,
        "tui() adds the storage-backed session store once the URL is resolved"
    );
    assert!(!summary.has_server_launcher, "the TUI binds no server");
}

#[test]
#[cfg(feature = "server")]
fn cli_storage_is_opened_for_one_shot_commands_only() {
    let args = |parts: &[&str]| parts.iter().map(|p| p.to_string()).collect::<Vec<_>>();
    let uses = |parts: &[&str]| cli_command_uses_storage(&args(parts), |ns| ns == "voice");
    assert!(uses(&["agent", "list"]));
    assert!(uses(&["cron", "list"]));
    assert!(uses(&["--model", "x", "approvals", "list"]));
    assert!(uses(&["--model=x", "-p", "y", "cron", "list"]));
    // Bare `voice` has a domain CLI handler, so it runs and needs storage;
    // other bare namespaces only print help.
    assert!(uses(&["voice"]));
    assert!(uses(&["voice", "--skip-cleanup"]));
    assert!(!uses(&["cron"]));
    assert!(!uses(&["serve"]));
    assert!(!uses(&["--provider=a", "-m", "b", "run"]));
    assert!(!uses(&[]));
    assert!(!uses(&["--help"]));
    assert!(!uses(&["help"]));
    assert!(!uses(&["cron", "--help"]));
    assert!(!uses(&["cron", "help"]));
    assert!(!uses(&["cron", "list", "-h"]));
    assert!(!uses(&["cron", "list", "--help"]));
    assert!(!uses(&["agent"]));
    assert!(!uses(&["agent", "--help"]));
    assert!(!uses(&["agent", "help"]));
    assert!(!uses(&["call", "--help"]));
    assert!(!uses(&["mcp", "-h"]));
    assert!(uses(&["agent", "chat"]));
    // Later `--help` tokens are option values to the namespace parser.
    assert!(uses(&["cron", "list", "--format", "json", "--help"]));
    assert!(uses(&["cron", "add", "--name", "--help"]));
    assert!(uses(&["mcp"]));
    assert!(!uses(&["mcp", "help"]));
    assert!(!uses(&["mcp", "--verbose", "--help"]));
    assert!(!uses(&["agent", "foo", "help"]));
    assert!(!uses(&["call", "foo", "help"]));
    assert!(uses(&["mcp-server"]));
    assert!(!uses(&["mcp", "--help"]));
    assert!(!uses(&["--model", "--help", "cron", "list"]));
    assert!(!uses(&["--model"]));
    // `help` as an option value is not a help request.
    assert!(uses(&["cron", "add", "--name", "help"]));
    assert!(uses(&["--model", "help", "cron", "list"]));
}

#[test]
fn embed_info_is_versioned_and_rejects_unknown_flags() {
    let info = embed_info(&["embed".into(), "info".into(), "--json".into()])
        .unwrap()
        .unwrap();
    assert_eq!(info.schema_version, 1);
    assert!(info.compiled_features.contains_key("voice"));
    assert!(embed_info(&["embed".into(), "info".into(), "--secret".into()]).is_err());
    assert!(embed_info(&["serve".into()]).unwrap().is_none());
}

/// A legacy config whose `api_url` is the managed chat-completions URL must
/// load as managed (URL derived, not moved to `inference_url`) when the host
/// opens its session store before connecting. The load-time migrations know
/// the managed hosts only through the TinyHumans config enricher, which the
/// boot path must register ahead of the first config read.
#[cfg(feature = "session-store")]
#[tokio::test]
async fn boot_session_store_keeps_a_legacy_managed_config_managed() {
    let _turn = crate::STORAGE_SLOT_TEST_LOCK.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("config.toml"),
        "api_url = \"https://api.tinyhumans.ai/openai/v1/chat/completions\"\n",
    )
    .unwrap();
    let _env = crate::server::testing::EnvVarGuard::set_many(vec![
        ("OPENHUMAN_WORKSPACE", tmp.path().into()),
        ("OPENHUMAN_STORAGE_URL", "".into()),
    ]);
    openhuman_tinyhumans::__host::config_enrichers::unregister_enricher(
        openhuman_tinyhumans::client::enricher::ENRICHER_ID,
    );

    let previous = crate::core_host::agent::session_store::installed();
    let previous_backend = crate::core_host::storage::installed();
    let result = session_store_for_boot().await;
    let config = crate::core_host::config::ops::load_config_in(
        &crate::core_host::core::runtime::DispatchCtx::host_default(),
    )
    .await;
    // Put the process-wide slots back before asserting.
    match previous_backend {
        Some(backend) => {
            crate::core_host::storage::install(backend);
        }
        None => {
            crate::core_host::storage::clear();
        }
    }
    drop(previous);
    result.expect("the boot session store opens");
    let config = config.expect("the legacy config loads");
    assert!(
        config.inference_url.is_none(),
        "a managed api_url must not be moved to a custom inference_url"
    );
    assert!(
        config.cloud_providers.iter().all(|p| p.slug != "custom"),
        "no Custom provider may be seeded from a managed endpoint: {:?}",
        config.cloud_providers
    );
    let on_disk = std::fs::read_to_string(tmp.path().join("config.toml")).unwrap();
    assert!(
        !on_disk.contains("slug = \"custom\""),
        "boot must not persist a managed config as Custom:\n{on_disk}"
    );
}
