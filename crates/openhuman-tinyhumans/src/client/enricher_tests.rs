use super::*;
use openhuman_embed::__host::config_enrichers::run_enrichers;

fn config_with(api_url: Option<&str>) -> Config {
    Config {
        api_url: api_url.map(str::to_string),
        ..Config::default()
    }
}

#[test]
fn socket_url_swaps_scheme_and_appends_the_handshake() {
    assert_eq!(
        socket_url_for("https://api.tinyhumans.ai"),
        "wss://api.tinyhumans.ai/socket.io/?EIO=4&transport=websocket"
    );
    assert_eq!(
        socket_url_for("http://127.0.0.1:7788/"),
        "ws://127.0.0.1:7788/socket.io/?EIO=4&transport=websocket"
    );
    assert_eq!(socket_url_for("not a url"), "not a url");
}

#[test]
fn a_loopback_api_url_override_drives_every_derived_field() {
    let _identity = crate::backend::product::product_identity_test_lock();
    let _env = crate::backend::url::backend_env_test_lock();
    let resolved = resolve(&config_with(Some("http://127.0.0.1:18765")));
    assert_eq!(
        resolved.control_plane_url.as_deref(),
        Some("http://127.0.0.1:18765")
    );
    assert_eq!(
        resolved.inference_url.as_deref(),
        Some("http://127.0.0.1:18765")
    );
    assert_eq!(
        resolved.socket_url.as_deref(),
        Some("ws://127.0.0.1:18765/socket.io/?EIO=4&transport=websocket")
    );
    assert_eq!(
        resolved.tracing_ingest_url.as_deref(),
        Some("http://127.0.0.1:18765/telemetry/langfuse/ingestion")
    );
}

#[test]
fn control_plane_and_inference_follow_the_same_rules_as_the_transport() {
    let _identity = crate::backend::product::product_identity_test_lock();
    let _env = crate::backend::url::backend_env_test_lock();
    // Whatever the override, the enricher must agree with what the SDK
    // transport answers to the core's `base_url` questions today.
    use crate::backend::url::{effective_api_url, effective_backend_api_url};
    for api_url in [
        None,
        Some("http://127.0.0.1:18765".to_string()),
        // A local model runner is not a control plane: backend calls fall
        // through to the hosted default while inference follows the override.
        Some("http://localhost:11434/v1".to_string()),
        Some("https://api.tinyhumans.ai/openai/v1/chat/completions".to_string()),
    ] {
        let config = config_with(api_url.as_deref());
        let resolved = resolve(&config);
        assert_eq!(
            resolved.control_plane_url.unwrap(),
            effective_backend_api_url(&api_url),
            "{api_url:?}"
        );
        assert_eq!(
            resolved.inference_url.unwrap(),
            effective_api_url(&api_url),
            "{api_url:?}"
        );
    }
}

#[test]
fn attribution_headers_and_identity_come_from_this_crate() {
    let _identity = crate::backend::product::product_identity_test_lock();
    let _env = crate::backend::url::backend_env_test_lock();
    let resolved = resolve(&config_with(None));
    let identity = crate::backend::product::product_identity();
    assert_eq!(
        resolved.product_identity.as_deref(),
        Some(identity.as_str())
    );
    let (name, _) = crate::backend::product::product_identity_header();
    let sdk_name = resolved
        .attribution_headers
        .iter()
        .find(|(n, _)| n == name.as_str())
        .unwrap_or_else(|| panic!("missing {name}: {:?}", resolved.attribution_headers));
    assert_eq!(sdk_name.1, identity.as_str());
}

#[test]
fn enrich_replaces_resolved_endpoints_and_is_visible_to_the_core_loader_hook() {
    let _registry = crate::registry_test_lock();

    let _identity = crate::backend::product::product_identity_test_lock();
    let _env = crate::backend::url::backend_env_test_lock();
    let mut config = config_with(Some("http://127.0.0.1:1"));
    assert!(config.resolved_endpoints.is_empty());
    enrich(&mut config);
    assert_eq!(
        config.resolved_endpoints.control_plane_url.as_deref(),
        Some("http://127.0.0.1:1")
    );

    // Registered through the core's slot, `run_enrichers` reaches it.
    register();
    let mut via_core = config_with(Some("http://127.0.0.1:2"));
    run_enrichers(&mut via_core);
    assert_eq!(
        via_core.resolved_endpoints.control_plane_url.as_deref(),
        Some("http://127.0.0.1:2")
    );
    // Idempotent: registering twice leaves a single enricher's result.
    register();
    let mut again = config_with(Some("http://127.0.0.1:3"));
    run_enrichers(&mut again);
    assert_eq!(
        again.resolved_endpoints.control_plane_url.as_deref(),
        Some("http://127.0.0.1:3")
    );
}

#[test]
fn deployment_environment_buckets_the_canonical_hosts() {
    assert_eq!(
        deployment_environment("https://api.tinyhumans.ai"),
        "production"
    );
    assert_eq!(
        deployment_environment("https://staging-api.tinyhumans.ai"),
        "staging"
    );
}

/// A hostname that merely *contains* `staging` is not ours. The classifier
/// gates whether a live session token leaves the process, so anything it
/// cannot positively recognise has to land on `external`.
#[test]
fn a_lookalike_staging_host_is_external_not_staging() {
    for base in [
        "https://staging-attacker.invalid",
        "https://staging.evil.example",
        "http://staging-api.tinyhumans.ai.evil.example",
        // Right domain, wrong label position.
        "https://api-staging-mirror.tinyhumans.ai",
        // A public IP literal is never a deployment of ours.
        "https://93.184.216.34",
        // Never classify plaintext as a hosted bucket.
        "http://api.tinyhumans.ai",
        "not a url",
        "",
    ] {
        assert_eq!(deployment_environment(base), "external", "{base:?}");
    }
}

#[test]
fn the_enricher_fills_otlp_environment_and_managed_hosts() {
    let _identity = crate::backend::product::product_identity_test_lock();
    let _env = crate::backend::url::backend_env_test_lock();
    let resolved = resolve(&config_with(Some("http://127.0.0.1:18765")));
    assert_eq!(
        resolved.tracing_otlp_url.as_deref(),
        Some("http://127.0.0.1:18765/telemetry/langfuse/otel/v1/traces")
    );
    assert_eq!(resolved.deployment_environment.as_deref(), Some("external"));
    for url in [
        "https://api.tinyhumans.ai/openai/v1",
        "https://staging-api.tinyhumans.ai",
        "https://api.openhuman.ai",
        "http://openhuman:7788",
    ] {
        assert!(resolved.is_managed_host(url), "{url}");
    }
    assert!(!resolved.is_managed_host("https://openrouter.ai/api/v1"));
}
