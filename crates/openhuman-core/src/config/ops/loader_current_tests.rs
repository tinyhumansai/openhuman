use super::*;
use crate::config::schema::{EphemeralRoute, EPHEMERAL_ROUTE_SLUG};
use crate::core::runtime::DispatchCtx;
use crate::core::runtime::{context::CoreContext, DomainSet};

fn context_with(config: Config) -> std::sync::Arc<CoreContext> {
    CoreContext::for_test_with_config(DomainSet::full(), config)
}

fn base_config() -> Config {
    let mut config = Config::default();
    config.default_model = Some("ctx-model".to_string());
    config
}

#[tokio::test]
async fn a_context_route_follows_every_load_of_the_context_config() {
    let mut config = base_config();
    config.ephemeral_route = Some(EphemeralRoute {
        endpoint: "http://127.0.0.1:9/v1".to_string(),
        api_key: "test-key".to_string(),
        headers: Vec::new(),
    });

    let loaded = load_current_or_init(&DispatchCtx::new(context_with(config)))
        .await
        .expect("context config loads");

    let pinned = format!("{EPHEMERAL_ROUTE_SLUG}:ctx-model");
    assert_eq!(loaded.chat_provider.as_deref(), Some(pinned.as_str()));
    assert_eq!(loaded.agentic_provider.as_deref(), Some(pinned.as_str()));
    assert_eq!(
        loaded
            .cloud_providers
            .iter()
            .filter(|entry| entry.slug == EPHEMERAL_ROUTE_SLUG)
            .count(),
        1
    );
}

#[tokio::test]
async fn a_context_without_a_route_loads_unchanged_providers() {
    let loaded = load_current_or_init(&DispatchCtx::new(context_with(base_config())))
        .await
        .expect("context config loads");

    assert!(loaded.ephemeral_route.is_none());
    assert!(loaded
        .cloud_providers
        .iter()
        .all(|entry| entry.slug != EPHEMERAL_ROUTE_SLUG));
    assert_ne!(
        loaded.chat_provider.as_deref(),
        Some(format!("{EPHEMERAL_ROUTE_SLUG}:ctx-model").as_str())
    );
}

#[tokio::test]
async fn the_explicit_context_wins_over_the_tasks_scope_and_tenants_do_not_cross() {
    let mut other = base_config();
    other.default_model = Some("other-model".to_string());
    let scoped = context_with(other);
    let explicit = DispatchCtx::new(context_with(base_config()));

    let loaded = CoreContext::scope(scoped, async { load_current_or_init(&explicit).await })
        .await
        .expect("explicit context config loads");

    assert_eq!(loaded.default_model.as_deref(), Some("ctx-model"));
}
