use super::*;

fn config_with_mode(mode: &str) -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.composio.mode = mode.to_string();
    config
}

#[test]
fn a_registered_route_builder_resolves_the_backend_mode() {
    let config = config_with_mode("backend");
    register_composio_route_builder("backend", |_| Ok(ComposioRoute::Backend));
    let route = resolve_composio_route(&config);
    unregister_composio_route_builder("backend");
    assert_eq!(route.unwrap().mode(), COMPOSIO_MODE_BACKEND);
}

#[test]
fn the_empty_mode_uses_the_backend_route_builder() {
    let config = config_with_mode("");
    register_composio_route_builder("backend", |_| Ok(ComposioRoute::Backend));
    let route = resolve_composio_route(&config);
    unregister_composio_route_builder("backend");
    assert!(matches!(route, Ok(ComposioRoute::Backend)));
}

#[test]
fn a_route_builder_error_is_returned_unchanged() {
    let config = config_with_mode("backend");
    register_composio_route_builder("backend", |_| anyhow::bail!("no hosted session"));
    let route = resolve_composio_route(&config);
    unregister_composio_route_builder("backend");
    assert_eq!(route.err().unwrap().to_string(), "no hosted session");
}

#[test]
fn without_a_route_builder_the_built_in_resolution_applies() {
    assert!(resolve_composio_route(&config_with_mode("disabled")).is_err());
    let unknown = resolve_composio_route(&config_with_mode("bogus"))
        .err()
        .unwrap()
        .to_string();
    assert!(unknown.contains("unknown composio mode"));
}

#[test]
fn a_host_pinned_credential_wins_over_a_registered_builder() {
    let mut config = config_with_mode("backend");
    config.composio.host_credential = Some(crate::config::ComposioHostCredential::direct("key"));
    register_composio_route_builder("backend", |_| anyhow::bail!("must not run"));
    let route = resolve_composio_route(&config);
    unregister_composio_route_builder("backend");
    assert!(matches!(route, Ok(ComposioRoute::Direct(_))));
}
