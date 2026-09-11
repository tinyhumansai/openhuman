use super::*;

#[test]
fn openai_compatible_carries_both_halves() {
    let provider = Provider::openai_compatible("https://api.example/v1", "sk-test");
    let route = provider.route().expect("routed");
    assert_eq!(route.base_url, "https://api.example/v1");
    assert_eq!(route.api_key, "sk-test");
    assert!(provider.is_routed());
}

#[test]
fn inherit_states_no_route() {
    let provider = Provider::inherit();
    assert!(provider.route().is_none());
    assert!(!provider.is_routed());
    assert!(provider.model_id().is_none());
}

#[test]
fn usable_routes_require_nonblank_endpoint_and_credential() {
    assert!(Provider::openai_compatible("https://api.example/v1", "sk").has_usable_route());
    assert!(!Provider::openai_compatible("   ", "sk").has_usable_route());
    assert!(!Provider::openai_compatible("https://api.example/v1", "   ").has_usable_route());
}

#[test]
fn a_blank_model_is_not_a_model() {
    // The core treats a blank `model_override` as absent, and a route with no
    // resolved model registers nothing at all. Collapsing it here means the
    // builder never writes `default_model = Some("")`, which would look set and
    // behave unset.
    assert!(Provider::inherit().model("   ").model_id().is_none());
    assert!(Provider::inherit().model("").model_id().is_none());
    assert_eq!(Provider::inherit().model("gpt-5").model_id(), Some("gpt-5"));
}

#[test]
fn the_default_provider_inherits() {
    assert_eq!(Provider::default(), Provider::inherit());
}

#[test]
fn debug_redacts_the_provider_bearer() {
    // `Provider` wraps `Route`, whose derived Debug would print the api_key.
    // Formatting a provider (e.g. in a log) must never expose the bearer.
    let provider =
        Provider::openai_compatible("https://api.example/v1", "sk-super-secret").model("gpt-5");
    let debug = format!("{provider:?}");

    assert!(debug.contains("api.example"), "base_url stays readable");
    assert!(
        !debug.contains("sk-super-secret"),
        "bearer leaked into Debug: {debug}"
    );
    assert!(debug.contains("redacted"));
}
