use super::*;

use crate::openhuman::config::Config;

/// A config with a model resolved for the call, as `agent_chat` leaves it after
/// applying `model_override`.
fn config_with_model(model: &str) -> Config {
    Config {
        default_model: Some(model.to_string()),
        ..Config::default()
    }
}

fn route() -> EphemeralRoute {
    EphemeralRoute {
        endpoint: "http://127.0.0.1:41234/openai".to_string(),
        api_key: "mdl-token".to_string(),
    }
}

// ── from_params ──────────────────────────────────────────────────────────────

#[test]
fn from_params_needs_both_halves() {
    assert_eq!(
        EphemeralRoute::from_params(Some("http://x/openai".into()), Some("k".into())),
        Some(EphemeralRoute {
            endpoint: "http://x/openai".into(),
            api_key: "k".into(),
        })
    );
    // An endpoint with no credential and a credential with no endpoint are both
    // partial statements; guessing the other half would run the turn somewhere
    // the caller never named.
    assert_eq!(
        EphemeralRoute::from_params(Some("http://x/openai".into()), None),
        None
    );
    assert_eq!(EphemeralRoute::from_params(None, Some("k".into())), None);
    assert_eq!(EphemeralRoute::from_params(None, None), None);
}

#[test]
fn from_params_treats_blank_as_absent_and_trims() {
    assert_eq!(
        EphemeralRoute::from_params(Some("  ".into()), Some("k".into())),
        None
    );
    assert_eq!(
        EphemeralRoute::from_params(Some("http://x/openai".into()), Some("\t".into())),
        None
    );
    assert_eq!(
        EphemeralRoute::from_params(Some(" http://x/openai ".into()), Some(" k ".into())),
        Some(EphemeralRoute {
            endpoint: "http://x/openai".into(),
            api_key: "k".into(),
        })
    );
}

// ── apply ────────────────────────────────────────────────────────────────────

#[test]
fn apply_registers_the_entry_and_pins_the_turn_roles() {
    let mut config = config_with_model("anthropic/claude-sonnet-4");
    apply(&mut config, route());

    let entry = config
        .cloud_providers
        .iter()
        .find(|entry| entry.slug == EPHEMERAL_ROUTE_SLUG)
        .expect("the route registers a cloud_providers entry");
    assert_eq!(entry.endpoint, "http://127.0.0.1:41234/openai");
    assert_eq!(entry.auth_style, AuthStyle::Bearer);

    let expected = format!("{EPHEMERAL_ROUTE_SLUG}:anthropic/claude-sonnet-4");
    assert_eq!(config.chat_provider.as_deref(), Some(expected.as_str()));
    assert_eq!(
        config.reasoning_provider.as_deref(),
        Some(expected.as_str())
    );
    assert_eq!(config.agentic_provider.as_deref(), Some(expected.as_str()));
    assert_eq!(config.coding_provider.as_deref(), Some(expected.as_str()));
}

#[test]
fn apply_replaces_an_explicit_role_pin() {
    // The route is the caller's whole statement about where this turn runs, and
    // `provider_for_role` reads the pins first — leaving one in place would send
    // the turn to the account's provider and make the route look ignored.
    let mut config = config_with_model("x/y");
    config.coding_provider = Some("anthropic:claude-3-5-sonnet".into());
    apply(&mut config, route());
    assert_eq!(
        config.coding_provider.as_deref(),
        Some(format!("{EPHEMERAL_ROUTE_SLUG}:x/y").as_str())
    );
}

#[test]
fn apply_leaves_background_roles_alone() {
    // Embeddings and memory run tier-specific models a chat endpoint generally
    // cannot serve. Ignoring the route for those workloads beats sending them
    // somewhere that will 404 or answer with prose.
    let mut config = config_with_model("x/y");
    config.embeddings_provider = Some("openhuman".into());
    config.memory_provider = Some("openhuman".into());
    config.vision_provider = Some("openhuman".into());
    apply(&mut config, route());
    assert_eq!(config.embeddings_provider.as_deref(), Some("openhuman"));
    assert_eq!(config.memory_provider.as_deref(), Some("openhuman"));
    assert_eq!(config.vision_provider.as_deref(), Some("openhuman"));
}

#[test]
fn apply_leaves_the_persisted_inference_route_untouched() {
    // `inference_url`/`api_key`/`primary_cloud` are the account's own BYOK
    // route. A per-call route that rewrote them would be indistinguishable from
    // the operator changing their settings.
    let mut config = config_with_model("x/y");
    config.inference_url = Some("https://api.example.test/v1".into());
    config.api_key = Some("account-key".into());
    config.primary_cloud = Some("some-id".into());
    apply(&mut config, route());
    assert_eq!(
        config.inference_url.as_deref(),
        Some("https://api.example.test/v1")
    );
    assert_eq!(config.api_key.as_deref(), Some("account-key"));
    assert_eq!(config.primary_cloud.as_deref(), Some("some-id"));
}

#[test]
fn apply_without_a_model_changes_nothing() {
    // `"<slug>:"` would trade a working default for a "resolved to an empty
    // model id" bail, which is strictly worse than ignoring the route.
    let mut config = Config::default();
    config.default_model = Some("   ".into());
    apply(&mut config, route());
    assert!(config.ephemeral_route.is_none());
    assert!(config.chat_provider.is_none());
    assert!(config
        .cloud_providers
        .iter()
        .all(|entry| entry.slug != EPHEMERAL_ROUTE_SLUG));
}

#[test]
fn apply_twice_leaves_one_entry() {
    let mut config = config_with_model("x/y");
    apply(&mut config, route());
    apply(
        &mut config,
        EphemeralRoute {
            endpoint: "http://127.0.0.1:9999/openai".into(),
            api_key: "mdl-other".into(),
        },
    );
    let entries: Vec<_> = config
        .cloud_providers
        .iter()
        .filter(|entry| entry.slug == EPHEMERAL_ROUTE_SLUG)
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].endpoint, "http://127.0.0.1:9999/openai");
}

#[test]
fn the_route_cannot_be_serialized_into_a_config_file() {
    // The containment the whole design rests on: there is no field on the wire
    // to write it into, so a caller that saves this config cannot repoint the
    // operator's install.
    let mut config = config_with_model("x/y");
    apply(&mut config, route());
    let encoded = toml::to_string(&config).expect("config serializes");
    assert!(!encoded.contains("mdl-token"));
    assert!(!encoded.contains("ephemeral_route"));
}
