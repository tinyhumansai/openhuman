//! Each host preset must carry the domain/service/host-kind triple that host
//! boots with today (rpc `server/shims.rs`, tui `runner.rs`).

use super::*;

fn triple(builder: &RuntimeBuilder) -> (HostKind, Option<DomainSet>, Option<ServiceSet>) {
    (builder.host_kind, builder.domains, builder.services)
}

#[test]
fn library_is_the_plain_builder() {
    let library = RuntimeBuilder::library();
    let new = RuntimeBuilder::new();
    assert_eq!(triple(&library), triple(&new));
    assert_eq!(library.host_kind, HostKind::Library);
    assert!(matches!(library.workspace, Workspace::Ephemeral));
    assert_eq!(library.config_source, ConfigSource::Resolved);
    assert!(matches!(library.token, TokenSource::EnvOrFile));
}

#[test]
fn desktop_matches_the_embedded_server_shim() {
    let desktop = RuntimeBuilder::desktop();
    assert_eq!(
        triple(&desktop),
        (
            HostKind::TauriShell,
            Some(DomainSet::full()),
            Some(ServiceSet::desktop())
        )
    );
    assert!(matches!(desktop.workspace, Workspace::Inherit));
    assert_eq!(desktop.config_source, ConfigSource::Discovered);
    assert!(desktop.validate().is_ok(), "the preset must build as is");
}

#[test]
fn cli_matches_the_standalone_server_shim() {
    let cli = RuntimeBuilder::cli();
    assert_eq!(
        triple(&cli),
        (
            HostKind::detect_standalone(),
            Some(DomainSet::full()),
            Some(ServiceSet::desktop())
        )
    );
    assert!(matches!(cli.workspace, Workspace::Inherit));
    assert_eq!(cli.config_source, ConfigSource::Discovered);
    assert!(matches!(cli.token, TokenSource::EnvOrFile));
    assert!(cli.validate().is_ok());
}

#[test]
fn tui_runs_every_domain_and_no_services() {
    let tui = RuntimeBuilder::tui();
    assert_eq!(
        triple(&tui),
        (
            HostKind::detect_standalone(),
            Some(DomainSet::full()),
            Some(ServiceSet::none())
        )
    );
    assert!(matches!(tui.workspace, Workspace::Inherit));
    assert_eq!(tui.config_source, ConfigSource::Discovered);
    assert!(
        tui.tool_groups.is_none(),
        "the TUI keeps the withheld default"
    );
    assert!(tui.validate().is_ok());
}

#[test]
fn presets_stay_overridable() {
    let headless = RuntimeBuilder::cli().services(ServiceSet::headless_api());
    assert_eq!(headless.services, Some(ServiceSet::headless_api()));
    let fixed = RuntimeBuilder::desktop()
        .token(TokenSource::Fixed(std::sync::Arc::new("t".into())))
        .listen("127.0.0.1", 7790);
    assert!(matches!(fixed.token, TokenSource::Fixed(_)));
    assert_eq!(fixed.listen_port, Some(7790));
}

#[test]
fn library_defaults_start_no_background_writers() {
    assert_eq!(
        default_services(),
        ServiceSet {
            ..ServiceSet::none()
        }
    );
    assert!(default_domains().inference);
}
