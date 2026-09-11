use super::*;

#[test]
fn maps_every_host_auth_style_one_to_one() {
    assert_eq!(map_auth_style(HostAuthStyle::None), CrateAuthStyle::None);
    assert_eq!(
        map_auth_style(HostAuthStyle::Bearer),
        CrateAuthStyle::Bearer
    );
    assert_eq!(
        map_auth_style(HostAuthStyle::XApiKey),
        CrateAuthStyle::XApiKey
    );
    assert_eq!(
        map_auth_style(HostAuthStyle::Anthropic),
        CrateAuthStyle::Anthropic
    );
    assert_eq!(
        map_auth_style(HostAuthStyle::Custom("x-key".to_string())),
        CrateAuthStyle::Custom("x-key".to_string())
    );
}

#[test]
fn builds_a_chat_model_with_the_configured_profile() {
    let model = build_crate_openai_model(CrateOpenAiConfig {
        provider_name: "deepseek",
        endpoint: "https://api.deepseek.com/v1",
        api_key: "secret",
        auth_style: HostAuthStyle::Bearer,
        model: "deepseek-chat",
        temperature_unsupported_models: &[],
        temperature_override: None,
        merge_system_into_user: false,
        extra_headers: &[],
        native_tool_calling: None,
        vision: None,
        default_provider_options: None,
        responses_api_primary: false,
        responses_omit_max_output_tokens: false,
        extra_query_params: &[],
        user_agent: None,
    });
    // The built model carries the configured provider + model on its profile.
    let profile = model.profile().expect("openai models expose a profile");
    assert_eq!(profile.provider.as_deref(), Some("deepseek"));
    assert_eq!(profile.model.as_deref(), Some("deepseek-chat"));
    assert!(profile.tool_calling);
}

#[test]
fn factory_level_builder_carries_provider_and_model() {
    let model = make_crate_openai_chat_model(
        "groq",
        "https://api.groq.com/openai/v1",
        "secret",
        HostAuthStyle::Bearer,
        "llama-3.3-70b-versatile",
        &["o1*".to_string()],
        None,
        false,
    );
    let profile = model.profile().expect("openai models expose a profile");
    assert_eq!(profile.provider.as_deref(), Some("groq"));
    assert_eq!(profile.model.as_deref(), Some("llama-3.3-70b-versatile"));
}

#[test]
fn builder_applies_local_none_auth_without_panicking() {
    // Local runtime shape: no auth, empty key, merge-system on.
    let _model = build_crate_openai_model(CrateOpenAiConfig {
        provider_name: "ollama",
        endpoint: "http://localhost:11434/v1",
        api_key: "",
        auth_style: HostAuthStyle::None,
        model: "llama3.2",
        temperature_unsupported_models: &["o1*".to_string()],
        temperature_override: Some(0.0),
        merge_system_into_user: true,
        extra_headers: &[("X-Attr".to_string(), "openhuman".to_string())],
        native_tool_calling: Some(false),
        vision: Some(false),
        default_provider_options: None,
        responses_api_primary: false,
        responses_omit_max_output_tokens: false,
        extra_query_params: &[],
        user_agent: None,
    });
}

#[test]
fn local_runtime_builder_disables_native_tools_and_vision() {
    let model = make_crate_local_runtime_chat_model(
        "ollama",
        "http://localhost:11434/v1",
        "",
        HostAuthStyle::None,
        "qwen2.5",
        &[],
        None,
        Some(8192),
    );
    let profile = model.profile().expect("openai models expose a profile");
    assert_eq!(profile.provider.as_deref(), Some("ollama"));
    assert_eq!(profile.model.as_deref(), Some("qwen2.5"));
    // Local runtimes must not advertise native tools or vision.
    assert!(!profile.tool_calling);
    assert!(!profile.modalities.image_in);
}
