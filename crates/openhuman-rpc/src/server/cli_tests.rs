use super::*;

use crate::core_host::core::runtime::Mode;
use openhuman_tinyhumans::embed::{DomainSet, TokenSource};

fn request() -> ServeRequest {
    ServeRequest {
        host: None,
        port: None,
        socketio_enabled: true,
        headless_api: false,
        mode: Mode::SingleUser,
        saas_config: None,
        host_boot: None,
    }
}

fn host_builder() -> RuntimeBuilder {
    let mut domains = DomainSet::full();
    domains.mcp = false;
    let mut services = ServiceSet::desktop();
    services.cron = false;
    RuntimeBuilder::cli()
        .domains(domains)
        .services(services)
        .token(TokenSource::Fixed(std::sync::Arc::new("t".into())))
        .listen_host("10.0.0.1")
        .listen_port(7811)
}

#[test]
fn no_flags_keep_the_host_builders_domains_services_token_and_listener() {
    let before = host_builder().summary();
    let after = flagged_builder(host_builder(), &request()).summary();
    assert_eq!(after.domains, before.domains);
    assert_eq!(after.services, before.services);
    assert!(after.fixed_token);
    assert_eq!(after.listen_host.as_deref(), Some("10.0.0.1"));
    assert_eq!(after.listen_port, Some(7811));
}

#[test]
fn explicit_flags_win_over_the_host_builder() {
    let mut req = request();
    req.host = Some("127.0.0.1".into());
    req.port = Some(9000);
    req.socketio_enabled = false;
    let summary = flagged_builder(host_builder(), &req).summary();
    assert_eq!(summary.listen_host.as_deref(), Some("127.0.0.1"));
    assert_eq!(summary.listen_port, Some(9000));
    let services = summary.services.expect("services");
    assert!(!services.socketio, "--jsonrpc-only clears socketio");
    assert!(!services.cron, "but keeps the builder's other choices");

    let mut headless = request();
    headless.headless_api = true;
    assert_eq!(
        flagged_builder(host_builder(), &headless)
            .summary()
            .services,
        Some(ServiceSet::headless_api())
    );
}

#[test]
fn without_a_host_builder_the_cli_preset_is_the_base() {
    let summary = flagged_builder(RuntimeBuilder::cli(), &request()).summary();
    assert_eq!(summary.services, Some(ServiceSet::desktop()));
    assert_eq!(summary.domains, Some(DomainSet::full()));
}
