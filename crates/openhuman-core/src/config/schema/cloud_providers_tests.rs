//! Tests for the surrounding module.

use super::{
    builtin_cloud_supports_responses_api, endpoint_host, endpoint_host_is_chat_completions_only,
    host_is_builtin_cloud_provider, is_builtin_cloud_slug, is_slug_reserved, migrate_legacy_fields,
    AuthStyle, CloudProviderCreds, CloudProviderType, BUILTIN_CLOUD_PROVIDERS,
};

#[test]
fn reserved_slugs() {
    for s in ["", " ", "cloud", "openhuman", "pid"] {
        assert!(is_slug_reserved(s), "{s:?} must stay reserved");
    }
}

// Regression: `ollama` was previously reserved, which made the AI settings
// panel unable to persist an `ollama` cloud_providers entry — so the
// model-list dropdown failed with "no cloud provider with id or slug
// 'ollama' found". The factory's chat routing is unaffected by this
// change because the `ollama:<model>` prefix branch fires before any
// cloud_providers lookup.
#[test]
fn ollama_and_lmstudio_are_not_reserved() {
    assert!(
        !is_slug_reserved("ollama"),
        "ollama must be usable as a cloud_providers slug for the /models probe"
    );
    assert!(
        !is_slug_reserved("lmstudio"),
        "lmstudio is a free-form OpenAI-compatible slug"
    );
}

#[test]
fn builtin_cloud_provider_defaults_cover_phase_one_presets() {
    for (slug, label, endpoint, auth_style) in [
        (
            "groq",
            "Groq",
            "https://api.groq.com/openai/v1",
            AuthStyle::Bearer,
        ),
        (
            "deepseek",
            "DeepSeek",
            "https://api.deepseek.com/v1",
            AuthStyle::Bearer,
        ),
        (
            "minimax",
            "MiniMax",
            "https://api.minimax.io/v1",
            AuthStyle::Bearer,
        ),
        (
            "sumopod",
            "SumoPod",
            "https://ai.sumopod.com/v1",
            AuthStyle::Bearer,
        ),
        (
            "modelscope",
            "ModelScope",
            "https://api-inference.modelscope.cn/v1",
            AuthStyle::Bearer,
        ),
    ] {
        let mut entry = CloudProviderCreds {
            id: format!("p_{slug}"),
            legacy_type: Some(slug.to_string()),
            ..Default::default()
        };
        migrate_legacy_fields(&mut entry);

        assert_eq!(entry.slug, slug);
        assert_eq!(entry.label, label);
        assert_eq!(entry.endpoint, endpoint);
        assert_eq!(entry.auth_style, auth_style);
    }
}

#[test]
fn builtin_cloud_provider_slugs_are_unique() {
    let mut slugs = std::collections::HashSet::new();
    for provider in BUILTIN_CLOUD_PROVIDERS {
        assert!(
            slugs.insert(provider.slug),
            "duplicate built-in cloud provider slug {}",
            provider.slug
        );
    }
}

#[test]
fn is_builtin_cloud_slug_matches_presets_only() {
    for slug in ["openai", "deepseek", "groq", "mistral"] {
        assert!(is_builtin_cloud_slug(slug), "{slug} is a built-in preset");
    }
    for slug in ["my-proxy", "custom-openai", "totally-unknown", ""] {
        assert!(
            !is_builtin_cloud_slug(slug),
            "{slug:?} is not a built-in preset"
        );
    }
}

#[test]
fn only_openai_builtin_exposes_responses_api() {
    assert!(builtin_cloud_supports_responses_api("openai"));
    for slug in ["deepseek", "groq", "mistral", "fireworks", "together"] {
        assert!(
            !builtin_cloud_supports_responses_api(slug),
            "{slug} is chat-completions-only and must not advertise the Responses API"
        );
    }
}

/// Drift guard (TAURI-RUST-5EN): couple the capability helper to the
/// preset list so adding a new built-in that wrongly claims the Responses
/// API — or renaming `openai` — fails CI rather than silently re-enabling
/// the guaranteed-404 `/responses` fallback. OpenAI's first-party endpoint
/// is the only built-in that serves `/v1/responses`.
#[test]
fn responses_api_capability_is_coupled_to_the_preset_list() {
    for provider in BUILTIN_CLOUD_PROVIDERS {
        let expected = provider.slug == "openai";
        assert_eq!(
            builtin_cloud_supports_responses_api(provider.slug),
            expected,
            "built-in {} Responses-API capability drifted from the openai-only invariant",
            provider.slug
        );
    }
}

#[test]
fn endpoint_host_parses_scheme_path_and_port() {
    assert_eq!(
        endpoint_host("https://integrate.api.nvidia.com/v1").as_deref(),
        Some("integrate.api.nvidia.com")
    );
    // Missing scheme, mixed case, trailing path.
    assert_eq!(
        endpoint_host("API.OpenAI.com/v1/chat").as_deref(),
        Some("api.openai.com")
    );
    // Userinfo + explicit port are stripped.
    assert_eq!(
        endpoint_host("https://user:pass@api.groq.com:443/openai/v1").as_deref(),
        Some("api.groq.com")
    );
    // Bracketed IPv6 literal with port.
    assert_eq!(
        endpoint_host("http://[::1]:8080/v1").as_deref(),
        Some("::1")
    );
    assert_eq!(endpoint_host("   ").as_deref(), None);
}

/// TAURI-RUST-HW1: the backend-URL resolver uses this to reroute backend
/// domain calls away from a BYO inference host. Every built-in provider host
/// must be recognised; OpenHuman backend hosts and unknown proxies must not.
#[test]
fn host_is_builtin_cloud_provider_recognises_inference_hosts() {
    for host in [
        "openrouter.ai",
        "api.openai.com",
        "api.anthropic.com",
        "api.groq.com",
        "generativelanguage.googleapis.com",
        "API.OPENAI.COM", // case-insensitive
    ] {
        assert!(
            host_is_builtin_cloud_provider(host),
            "{host} is a built-in cloud inference host"
        );
    }
    for host in [
        "api.tinyhumans.ai",
        "staging-api.tinyhumans.ai",
        "my-backend.example",
        "",
    ] {
        assert!(
            !host_is_builtin_cloud_provider(host),
            "{host:?} is not a built-in cloud inference host"
        );
    }
    // Every registry endpoint's own host must classify as builtin.
    for provider in BUILTIN_CLOUD_PROVIDERS {
        let host = endpoint_host(provider.endpoint).expect("preset endpoint has a host");
        assert!(
            host_is_builtin_cloud_provider(&host),
            "{} ({host}) must be recognised",
            provider.slug
        );
    }
}

/// TAURI-RUST-5A1: a *custom* slug pointed at a known chat-only host (NVIDIA)
/// must be classified chat-only so the factory disables the guaranteed-404
/// `/responses` fallback — the builtin-slug gate alone misses this because
/// the slug is not builtin.
#[test]
fn nvidia_host_is_chat_completions_only_regardless_of_slug() {
    assert!(endpoint_host_is_chat_completions_only(
        "https://integrate.api.nvidia.com/v1"
    ));
    // Other chat-only built-in hosts too.
    for endpoint in [
        "https://api.deepseek.com/v1",
        "https://api.groq.com/openai/v1",
        "https://api.mistral.ai/v1",
    ] {
        assert!(
            endpoint_host_is_chat_completions_only(endpoint),
            "{endpoint} is a chat-completions-only built-in host"
        );
    }
}

#[test]
fn openai_host_and_unknown_proxies_keep_the_responses_fallback() {
    // OpenAI's first-party host serves /responses — must NOT be gated off,
    // even via a custom proxy slug pointed at it.
    assert!(!endpoint_host_is_chat_completions_only(
        "https://api.openai.com/v1"
    ));
    // Genuinely unknown proxy hosts keep the permissive default (they may be
    // real OpenAI proxies that implement /responses).
    for endpoint in [
        "https://my-llm-proxy.internal.example/v1",
        "https://litellm.mycorp.dev/v1",
        "",
    ] {
        assert!(
            !endpoint_host_is_chat_completions_only(endpoint),
            "{endpoint:?} is an unknown host and must keep the fallback"
        );
    }
}

/// Drift guard: the host-based gate must agree with the slug-based
/// capability for every built-in preset's own endpoint, so adding a preset
/// can't silently desync the two gates.
#[test]
fn host_gate_agrees_with_slug_capability_for_every_builtin() {
    for provider in BUILTIN_CLOUD_PROVIDERS {
        // OpenhumanJwt / Anthropic presets never route through the
        // OpenAI-compatible Responses fallback; the gate only matters for
        // the Bearer OpenAI-compatible hosts.
        if provider.auth_style != AuthStyle::Bearer {
            continue;
        }
        let host_chat_only = endpoint_host_is_chat_completions_only(provider.endpoint);
        let slug_supports = builtin_cloud_supports_responses_api(provider.slug);
        assert_eq!(
            host_chat_only, !slug_supports,
            "host gate for built-in {} disagrees with its slug capability",
            provider.slug
        );
    }
}

#[test]
fn auth_styles_and_legacy_provider_types_expose_stable_wire_properties() {
    let styles = [
        (AuthStyle::Bearer, "bearer"),
        (AuthStyle::Anthropic, "anthropic"),
        (AuthStyle::OpenhumanJwt, "openhuman_jwt"),
        (AuthStyle::None, "none"),
    ];
    for (style, expected) in styles {
        assert_eq!(style.as_str(), expected);
    }

    let types = [
        (
            CloudProviderType::Openhuman,
            "https://api.openhuman.ai/v1",
            "OpenHuman",
            "openhuman",
            AuthStyle::OpenhumanJwt,
        ),
        (
            CloudProviderType::Openai,
            "https://api.openai.com/v1",
            "OpenAI",
            "openai",
            AuthStyle::Bearer,
        ),
        (
            CloudProviderType::Anthropic,
            "https://api.anthropic.com/v1",
            "Anthropic",
            "anthropic",
            AuthStyle::Anthropic,
        ),
        (
            CloudProviderType::Openrouter,
            "https://openrouter.ai/api/v1",
            "OpenRouter",
            "openrouter",
            AuthStyle::Bearer,
        ),
        (
            CloudProviderType::Orcarouter,
            "https://api.orcarouter.ai/v1",
            "OrcaRouter",
            "orcarouter",
            AuthStyle::Bearer,
        ),
        (
            CloudProviderType::Custom,
            "",
            "Custom",
            "custom",
            AuthStyle::Bearer,
        ),
    ];
    for (provider, endpoint, label, slug, auth) in types {
        assert_eq!(provider.default_endpoint(), endpoint);
        assert_eq!(provider.label(), label);
        assert_eq!(provider.as_str(), slug);
        assert_eq!(provider.auth_style(), auth);
    }
}

#[test]
fn migration_is_idempotent_and_unknown_types_remain_custom() {
    let mut custom = CloudProviderCreds {
        id: "custom-id".into(),
        legacy_type: Some("my-provider".into()),
        ..Default::default()
    };
    migrate_legacy_fields(&mut custom);
    assert_eq!(custom.slug, "my-provider");
    assert_eq!(custom.label, "Custom");
    assert!(custom.endpoint.is_empty());
    assert_eq!(custom.auth_style, AuthStyle::Bearer);

    let once = custom.clone();
    migrate_legacy_fields(&mut custom);
    assert_eq!(custom, once);

    let mut preserved = CloudProviderCreds {
        id: "preserved".into(),
        slug: "chosen".into(),
        label: "Chosen Label".into(),
        endpoint: "https://proxy.example/v1".into(),
        auth_style: AuthStyle::None,
        legacy_type: Some("openai".into()),
        default_model: Some("legacy-model".into()),
    };
    migrate_legacy_fields(&mut preserved);
    assert_eq!(preserved.slug, "chosen");
    assert_eq!(preserved.label, "Chosen Label");
    assert_eq!(preserved.endpoint, "https://proxy.example/v1");
    assert_eq!(preserved.auth_style, AuthStyle::None);
}

#[test]
fn generated_provider_ids_sanitize_and_bound_the_slug_prefix() {
    let id = super::generate_provider_id("provider with spaces/and_symbols!");
    assert!(id.starts_with("p_provider_with_spaces_"));
    let suffix = id.rsplit('_').next().expect("suffix");
    assert_eq!(suffix.len(), 5);
    assert!(suffix
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
}
