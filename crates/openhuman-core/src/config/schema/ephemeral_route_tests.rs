use super::*;

use crate::config::Config;

#[derive(Clone, Default)]
struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl Capture {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn capture_logs() -> (Capture, tracing::subscriber::DefaultGuard) {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    (capture, tracing::subscriber::set_default(subscriber))
}

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
        headers: Vec::new(),
        ..EphemeralRoute::default()
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
            headers: Vec::new(),
            ..EphemeralRoute::default()
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
            headers: Vec::new(),
            ..EphemeralRoute::default()
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
fn apply_carries_provider_options_and_model_reasoning_effort() {
    let mut config = config_with_model("provider/model");
    let mut route = route();
    route.provider_options = Some(serde_json::json!({"provider": {"sort": "latency"}}));
    route.reasoning_effort = Some("high".into());

    apply(&mut config, route);

    assert_eq!(
        config
            .ephemeral_route
            .as_ref()
            .and_then(|route| route.provider_options.clone()),
        Some(serde_json::json!({"provider": {"sort": "latency"}}))
    );
    assert_eq!(
        config
            .runtime
            .reasoning_effort_by_model
            .get("provider/model"),
        Some(&"high".to_string())
    );
}

#[test]
fn an_ephemeral_route_without_options_keeps_the_provider_default() {
    let mut config = config_with_model("provider/model");
    apply(&mut config, route());

    assert_eq!(
        config.ephemeral_route.as_ref().unwrap().provider_options,
        None
    );
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
            headers: Vec::new(),
            ..EphemeralRoute::default()
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
fn reapplying_the_same_route_is_a_no_op() {
    let mut config = config_with_model("x/y");
    apply(&mut config, route());
    let mut intervening = config
        .cloud_providers
        .iter()
        .find(|entry| entry.slug == EPHEMERAL_ROUTE_SLUG)
        .unwrap()
        .clone();
    intervening.id = "another-provider".into();
    intervening.slug = "another-provider".into();
    config.cloud_providers.push(intervening);
    let providers_before = config.cloud_providers.clone();
    let roles_before = (
        config.chat_provider.clone(),
        config.reasoning_provider.clone(),
        config.agentic_provider.clone(),
        config.coding_provider.clone(),
    );
    let (logs, _guard) = capture_logs();

    apply(&mut config, route());

    assert_eq!(config.cloud_providers, providers_before);
    assert_eq!(
        (
            config.chat_provider,
            config.reasoning_provider,
            config.agentic_provider,
            config.coding_provider,
        ),
        roles_before
    );
    let text = logs.text();
    assert!(text.contains("DEBUG"), "expected a debug log: {text:?}");
    assert!(!text.contains("INFO"));
}

#[test]
fn reapplying_the_same_route_repairs_a_changed_role_pin() {
    let mut config = config_with_model("x/y");
    apply(&mut config, route());
    config.coding_provider = Some("other-provider:model".into());
    let (logs, _guard) = capture_logs();

    apply(&mut config, route());

    assert_eq!(
        config.coding_provider.as_deref(),
        Some(format!("{EPHEMERAL_ROUTE_SLUG}:x/y").as_str())
    );
    let text = logs.text();
    assert!(text.contains("DEBUG"), "expected a debug log: {text:?}");
    assert!(!text.contains("INFO"));
}

#[test]
fn a_changed_route_is_applied_and_logged_once() {
    let mut config = config_with_model("x/y");
    apply(&mut config, route());
    let (logs, _guard) = capture_logs();
    let changed = EphemeralRoute {
        endpoint: "http://127.0.0.1:9999/openai".into(),
        api_key: "mdl-other".into(),
        headers: Vec::new(),
        ..EphemeralRoute::default()
    };

    apply(&mut config, changed.clone());

    assert_eq!(config.ephemeral_route, Some(changed));
    assert_eq!(
        config
            .cloud_providers
            .iter()
            .find(|entry| entry.slug == EPHEMERAL_ROUTE_SLUG)
            .map(|entry| entry.endpoint.as_str()),
        Some("http://127.0.0.1:9999/openai")
    );
    let text = logs.text();
    let info_lines: Vec<_> = text
        .lines()
        .filter(|line| line.contains("INFO") && line.contains("[config][ephemeral-route]"))
        .collect();
    assert_eq!(info_lines.len(), 1, "expected one info log: {info_lines:?}");
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

#[test]
fn debug_never_discloses_route_bearer_or_header_values() {
    let route = EphemeralRoute {
        endpoint: "https://route-user:route-password@fixture.example/route-path-secret/v1?key=route-query-secret#route-fragment-secret".into(),
        api_key: "u2-unique-route-secret".into(),
        headers: vec![("x-private".into(), "u2-unique-header-secret".into())],
        ..EphemeralRoute::default()
    };
    let debug = format!("{route:?}");
    assert!(!debug.contains("u2-unique-route-secret"));
    assert!(!debug.contains("u2-unique-header-secret"));
    assert!(!debug.contains("route-user"));
    assert!(!debug.contains("route-password"));
    assert!(!debug.contains("route-path-secret"));
    assert!(!debug.contains("route-query-secret"));
    assert!(!debug.contains("route-fragment-secret"));
}
