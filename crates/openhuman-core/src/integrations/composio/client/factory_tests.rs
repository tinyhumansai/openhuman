use super::*;

fn config_with_mode(mode: &str) -> crate::config::Config {
    let mut config = crate::config::Config::default();
    config.composio.mode = mode.to_string();
    config
}

#[tokio::test]
async fn a_registered_route_builder_resolves_the_backend_mode() {
    let config = config_with_mode("backend");
    register_composio_route_builder("backend", |_| {
        Box::pin(async { Ok(ComposioRoute::Backend) })
    });
    let route = resolve_composio_route(&config).await;
    unregister_composio_route_builder("backend");
    assert_eq!(route.unwrap().mode(), COMPOSIO_MODE_BACKEND);
}

#[tokio::test]
async fn the_empty_mode_uses_the_backend_route_builder() {
    let config = config_with_mode("");
    register_composio_route_builder("backend", |_| {
        Box::pin(async { Ok(ComposioRoute::Backend) })
    });
    let route = resolve_composio_route(&config).await;
    unregister_composio_route_builder("backend");
    assert!(matches!(route, Ok(ComposioRoute::Backend)));
}

#[tokio::test]
async fn a_route_builder_error_is_returned_unchanged() {
    let config = config_with_mode("backend");
    register_composio_route_builder("backend", |_| {
        Box::pin(async { anyhow::bail!("no hosted session") })
    });
    let route = resolve_composio_route(&config).await;
    unregister_composio_route_builder("backend");
    assert_eq!(route.err().unwrap().to_string(), "no hosted session");
}

#[tokio::test]
async fn without_a_route_builder_the_built_in_resolution_applies() {
    assert!(resolve_composio_route(&config_with_mode("disabled"))
        .await
        .is_err());
    let unknown = resolve_composio_route(&config_with_mode("bogus"))
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(unknown.contains("unknown composio mode"));
}

#[tokio::test]
async fn a_host_pinned_credential_wins_over_a_registered_builder() {
    let mut config = config_with_mode("backend");
    config.composio.host_credential = Some(crate::config::ComposioHostCredential::direct("key"));
    register_composio_route_builder("backend", |_| {
        Box::pin(async { anyhow::bail!("must not run") })
    });
    let route = resolve_composio_route(&config).await;
    unregister_composio_route_builder("backend");
    assert!(matches!(route, Ok(ComposioRoute::Direct(_))));
}

// Plain-http base URLs are allowed only for loopback; userinfo must not smuggle
// a remote host past that check.
#[test]
fn loopback_http_guard_rejects_smuggled_and_remote_hosts() {
    use super::super::credential::{is_loopback_http_url, DirectCredential};
    for url in [
        "http://127.0.0.1:8080@evil.com/api/v3",
        "http://localhost@evil.com/",
        "http://evil.com/",
        "http://[::2]:9/",
    ] {
        assert!(!is_loopback_http_url(url), "{url}");
        assert!(
            DirectCredential::new_with_base_urls("k", url.into(), url.into()).is_err(),
            "{url}"
        );
    }
    for url in [
        "http://[::1]:9/v3",
        "http://LOCALHOST:9/",
        "http://127.0.0.1:9/",
    ] {
        assert!(is_loopback_http_url(url), "{url}");
    }
}
