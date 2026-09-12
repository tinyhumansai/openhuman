use super::*;

#[test]
fn only_first_party_anthropic_endpoint_selects_messages_api() {
    assert!(endpoint_is_anthropic_messages(
        "https://api.anthropic.com/v1"
    ));
    assert!(!endpoint_is_anthropic_messages(
        "https://anthropic-proxy.example/v1"
    ));
    assert!(!endpoint_is_anthropic_messages("https://api.openai.com/v1"));
}

#[test]
fn builds_a_native_anthropic_model_with_the_configured_profile() {
    let model = build_crate_anthropic_model(CrateAnthropicConfig {
        endpoint: "https://api.anthropic.com/v1",
        api_key: "sk-ant-secret",
        model: "claude-sonnet-4-6",
        temperature_override: Some(0.2),
        temperature_unsupported_models: &[],
    });
    let profile = model.profile().expect("anthropic models expose a profile");
    assert_eq!(profile.provider.as_deref(), Some("anthropic"));
    assert_eq!(profile.model.as_deref(), Some("claude-sonnet-4-6"));
    assert!(profile.tool_calling, "the native adapter speaks tool_use");
    assert!(profile.streaming, "the native adapter streams SSE");
    // The identity the response cache scopes on names the endpoint + model and
    // never the credential.
    let identity = model
        .cache_identity()
        .expect("anthropic models identify themselves");
    assert!(identity.contains("api.anthropic.com"));
    assert!(!identity.contains("sk-ant-secret"));
}

#[test]
fn temperature_override_follows_the_effective_model() {
    let patterns = vec!["claude-3-5-*".to_string()];
    assert_eq!(
        temperature_for_model("claude-3-5-sonnet", Some(0.7), &patterns, Some(0.2)),
        None
    );
    assert_eq!(
        temperature_for_model("claude-sonnet-4-6", Some(0.7), &patterns, Some(0.2)),
        Some(0.2)
    );
    assert_eq!(
        temperature_for_model("claude-sonnet-4-6", Some(0.7), &patterns, None),
        Some(0.7)
    );
    assert_eq!(
        temperature_for_model("claude-sonnet-4-6", None, &patterns, None),
        None
    );
}
