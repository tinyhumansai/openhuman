use super::*;

fn entry(slug: &str, endpoint: &str, auth_style: AuthStyle) -> CloudProviderCreds {
    CloudProviderCreds {
        id: format!("p_{slug}"),
        slug: slug.to_string(),
        label: slug.to_string(),
        endpoint: endpoint.to_string(),
        auth_style,
        ..Default::default()
    }
}

fn build<'a>(entry: &CloudProviderCreds, model: &str, key: &'a str) -> CloudBuild<'a> {
    CloudBuild {
        spec: cloud_provider_spec(entry, model),
        auth_style: registry_auth_style(entry.auth_style).expect("not managed"),
        key,
        temperature_override: None,
        temperature_unsupported_models: &[],
        native_tool_calling: true,
        extra_headers: &[],
        extra_query_params: &[],
        user_agent: None,
        codex: None,
        http: None,
    }
}

#[test]
fn spec_keeps_the_slug_as_provider_name_and_picks_the_kind_from_auth() {
    let openai = cloud_provider_spec(
        &entry("my-proxy", "https://proxy.example/v1", AuthStyle::Bearer),
        "m",
    );
    assert_eq!(openai.kind, ProviderKind::Compatible);
    assert_eq!(openai.provider, "my-proxy");
    assert_eq!(openai.base_url, "https://proxy.example/v1");
    assert_eq!(openai.model, "m");
    assert!(!openai.requires_api_key);
    let claude = cloud_provider_spec(
        &entry(
            "anthropic",
            "https://api.anthropic.com/v1",
            AuthStyle::Anthropic,
        ),
        "claude",
    );
    assert_eq!(claude.kind, ProviderKind::Anthropic);
}

#[test]
fn managed_jwt_style_never_maps_onto_the_registry() {
    assert!(registry_auth_style(AuthStyle::OpenhumanJwt).is_none());
    assert!(registry_auth_style(AuthStyle::Bearer).is_some());
}

#[test]
fn registry_builds_a_model_for_every_unmanaged_builtin_preset() {
    for preset in crate::config::schema::cloud_providers::BUILTIN_CLOUD_PROVIDERS.iter() {
        if preset.auth_style == AuthStyle::OpenhumanJwt {
            continue;
        }
        let creds = entry(preset.slug, preset.endpoint, preset.auth_style);
        build_cloud_model(build(&creds, "some-model", "sk-test"))
            .unwrap_or_else(|error| panic!("{} failed to build: {error}", preset.slug));
    }
}

#[test]
fn discovery_request_carries_bearer_auth_and_pins_openrouter_sorting() {
    let creds = entry(
        "openrouter",
        "https://openrouter.ai/api/v1",
        AuthStyle::Bearer,
    );
    let request =
        cloud_discovery_request(&build(&creds, "m", "sk-or")).expect("openrouter has a listing");
    assert_eq!(request.endpoint, "https://openrouter.ai/api/v1");
    assert!(request
        .headers
        .iter()
        .any(|(name, value)| name == "Authorization" && value == "Bearer sk-or"));
}

#[test]
fn discovery_request_uses_x_api_key_for_anthropic_and_skips_codex_oauth() {
    let claude = entry(
        "anthropic",
        "https://api.anthropic.com/v1",
        AuthStyle::Anthropic,
    );
    let request = cloud_discovery_request(&build(&claude, "claude", "sk-ant")).expect("listing");
    assert!(request.headers.iter().any(|(name, _)| name == "x-api-key"));

    let openai = entry("openai", "https://api.openai.com/v1", AuthStyle::Bearer);
    let oauth = OpenAiCodexRouting {
        endpoint: "https://chatgpt.com/backend-api/codex".to_string(),
        using_oauth: true,
        account_id: None,
    };
    let mut with_oauth = build(&openai, "gpt", "tok");
    with_oauth.codex = Some(&oauth);
    assert!(cloud_discovery_request(&with_oauth).is_none());
}
