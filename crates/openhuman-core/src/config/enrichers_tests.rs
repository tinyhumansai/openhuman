use super::*;
use tempfile::TempDir;

fn fill(config: &mut Config) {
    config.resolved_endpoints.control_plane_url = Some("https://control.example".into());
    config.resolved_endpoints.attribution_headers = vec![("x-product".into(), "demo".into())];
    config.resolved_endpoints.product_identity = Some("demo".into());
}

#[test]
fn nothing_registered_leaves_the_config_untouched() {
    let config = enriched(Config::default());
    assert!(config.resolved_endpoints.is_empty());
}

#[test]
fn a_registered_enricher_fills_the_resolved_endpoints() {
    register_enricher("test-fill", fill);
    let config = enriched(Config::default());
    unregister_enricher("test-fill");
    assert_eq!(
        config.resolved_endpoints.control_plane_url.as_deref(),
        Some("https://control.example")
    );
    assert_eq!(
        config.resolved_endpoints.product_identity.as_deref(),
        Some("demo")
    );
}

#[test]
fn enrichers_run_in_registration_order() {
    register_enricher("first", |config: &mut Config| {
        config.resolved_endpoints.socket_url = Some("first".into());
    });
    register_enricher("second", |config: &mut Config| {
        let previous = config
            .resolved_endpoints
            .socket_url
            .take()
            .unwrap_or_default();
        config.resolved_endpoints.socket_url = Some(format!("{previous}>second"));
    });
    let config = enriched(Config::default());
    unregister_enricher("first");
    unregister_enricher("second");
    assert_eq!(
        config.resolved_endpoints.socket_url.as_deref(),
        Some("first>second")
    );
}

#[test]
fn resolved_endpoints_never_reach_the_serialized_config() {
    let mut config = Config::default();
    fill(&mut config);
    let toml = toml::to_string(&config).expect("serialize");
    assert!(!toml.contains("control.example"));
    assert!(!toml.contains("resolved_endpoints"));
}

#[test]
fn clones_keep_the_resolved_endpoints() {
    let mut config = Config::default();
    fill(&mut config);
    assert_eq!(config.clone().resolved_endpoints, config.resolved_endpoints);
}

#[tokio::test]
async fn the_snapshot_loader_runs_registered_enrichers() {
    let tmp = TempDir::new().unwrap();
    register_enricher("test-load", fill);
    let loaded =
        Config::load_from_config_path(&tmp.path().join("config.toml"), &tmp.path().join("ws"))
            .await;
    unregister_enricher("test-load");
    assert_eq!(
        loaded
            .unwrap()
            .resolved_endpoints
            .control_plane_url
            .as_deref(),
        Some("https://control.example")
    );
}

#[tokio::test]
async fn an_embedder_config_is_enriched_by_the_context_loader() {
    register_enricher("test-embedder", fill);
    let mut config = Config::default();
    crate::config::ops::normalize_loaded_config(&mut config).await;
    unregister_enricher("test-embedder");
    assert_eq!(
        config.resolved_endpoints.product_identity.as_deref(),
        Some("demo")
    );
}
