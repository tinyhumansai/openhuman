//! Tests for the surrounding module.

use super::{builtin_cloud_providers, builtin_presets, is_slug_reserved, AuthStyle};
use tinyinference_providers::builtin::endpoint_host;

#[test]
fn reserved_slugs() {
    for s in ["", " ", "cloud", "openhuman", "pid"] {
        assert!(is_slug_reserved(s), "{s:?} must stay reserved");
    }
}

#[test]
fn auth_style_parse_accepts_rpc_spellings_and_preserves_unknown_error() {
    for (value, expected) in [
        ("BEARER", AuthStyle::Bearer),
        ("anthropic", AuthStyle::Anthropic),
        ("openhuman_jwt", AuthStyle::OpenhumanJwt),
        ("OpenHumanJwt", AuthStyle::OpenhumanJwt),
        ("none", AuthStyle::None),
    ] {
        assert_eq!(AuthStyle::parse(value).unwrap(), expected);
    }
    assert_eq!(
        AuthStyle::parse("Strange").unwrap_err(),
        "unknown auth_style 'strange'; valid: bearer, anthropic, openhuman_jwt, none"
    );
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
        let entry = super::builtin_cloud_provider(slug).expect("builtin preset");
        assert_eq!(entry.slug, slug);
        assert_eq!(entry.label, label);
        assert_eq!(entry.endpoint, endpoint);
        assert_eq!(entry.auth_style, auth_style);
    }
}

#[test]
fn builtin_cloud_provider_slugs_are_unique() {
    let mut slugs = std::collections::HashSet::new();
    for provider in builtin_cloud_providers().iter() {
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
        assert!(
            builtin_presets().is_builtin_cloud_slug(slug),
            "{slug} is a built-in preset"
        );
    }
    for slug in ["my-proxy", "custom-openai", "totally-unknown", ""] {
        assert!(
            !builtin_presets().is_builtin_cloud_slug(slug),
            "{slug:?} is not a built-in preset"
        );
    }
}

#[test]
fn builtin_responses_capabilities_follow_provider_catalog() {
    assert!(builtin_presets().builtin_cloud_supports_responses_api("openai"));
    // The merged provider catalog documents these OpenAI-compatible gateways
    // as supporting /responses too. Preserve that upstream protocol contract.
    assert!(builtin_presets().builtin_cloud_supports_responses_api("openrouter"));
    assert!(builtin_presets().builtin_cloud_supports_responses_api("xai"));
    for slug in ["deepseek", "groq", "mistral", "fireworks", "together"] {
        assert!(
            !builtin_presets().builtin_cloud_supports_responses_api(slug),
            "{slug} is chat-completions-only and must not advertise the Responses API"
        );
    }
}

#[test]
fn hosted_presets_and_classification_are_context_scoped() {
    use crate::core::slots::Extensions;

    let extensions = Extensions::new();
    super::register_hosted_cloud_preset_in(
        &extensions,
        "tenant-hosted",
        super::BuiltinCloudProvider {
            slug: "tenant-hosted",
            label: "Tenant Hosted",
            endpoint: "https://tenant.example/v1",
            auth_style: AuthStyle::Bearer,
        },
    );
    super::register_hosted_cloud_preset_in(
        &extensions,
        "host-openai",
        super::BuiltinCloudProvider {
            slug: "openai",
            label: "Host OpenAI Override",
            endpoint: "https://override.example/v1",
            auth_style: AuthStyle::Anthropic,
        },
    );
    super::register_hosted_cloud_provider_policy_in(
        &extensions,
        std::sync::Arc::new(|slug| slug == "tenant-hosted"),
        std::sync::Arc::new(|style| style == AuthStyle::OpenhumanJwt),
    );
    let context = crate::core::runtime::CoreContext::for_test_with_extensions(
        crate::core::runtime::DomainSet::full(),
        None,
        extensions,
    );

    assert!(super::builtin_cloud_provider("tenant-hosted").is_none());
    crate::core::runtime::CoreContext::sync_scope(context, || {
        let presets = super::builtin_cloud_providers();
        assert_eq!(presets[0].slug, "tenant-hosted");
        assert_eq!(presets[0].label, "Tenant Hosted");
        assert_eq!(presets[0].endpoint, "https://tenant.example/v1");
        assert!(presets.iter().any(|preset| preset.slug == "openai"));
        assert_eq!(
            presets
                .iter()
                .filter(|preset| preset.slug == "openai")
                .count(),
            1
        );
        assert_eq!(
            super::builtin_cloud_provider("openai").unwrap().endpoint,
            "https://api.openai.com/v1",
            "host additions must not replace a generic provider"
        );
        assert!(super::builtin_presets().host_is_builtin_cloud_provider("tenant.example"));
        assert!(super::builtin_presets()
            .endpoint_host_is_chat_completions_only("https://tenant.example/v1"));
        assert!(super::builtin_cloud_provider("tenant-hosted").is_some());
        assert!(super::is_hosted_cloud_identity("tenant-hosted"));
        assert!(!super::is_hosted_auth_style(AuthStyle::Bearer));
        assert!(super::is_hosted_auth_style(AuthStyle::OpenhumanJwt));
    });
    assert!(super::builtin_cloud_provider("tenant-hosted").is_none());
    assert!(!super::is_hosted_cloud_identity("tenant-hosted"));
    assert!(!super::is_hosted_auth_style(AuthStyle::OpenhumanJwt));
}

/// Drift guard: keep the host catalog aligned with the generic provider
/// catalog's documented Responses API support.
#[test]
fn responses_api_capability_is_coupled_to_the_preset_list() {
    for provider in builtin_cloud_providers().iter() {
        let expected = matches!(provider.slug, "openai" | "openrouter" | "xai");
        assert_eq!(
            builtin_presets().builtin_cloud_supports_responses_api(provider.slug),
            expected,
            "built-in {} Responses-API capability drifted from the provider catalog",
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
            builtin_presets().host_is_builtin_cloud_provider(host),
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
            !builtin_presets().host_is_builtin_cloud_provider(host),
            "{host:?} is not a built-in cloud inference host"
        );
    }
    // Every registry endpoint's own host must classify as builtin.
    for provider in builtin_cloud_providers().iter() {
        let host = endpoint_host(provider.endpoint).expect("preset endpoint has a host");
        assert!(
            builtin_presets().host_is_builtin_cloud_provider(&host),
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
    assert!(builtin_presets()
        .endpoint_host_is_chat_completions_only("https://integrate.api.nvidia.com/v1"));
    // Other chat-only built-in hosts too.
    for endpoint in [
        "https://api.deepseek.com/v1",
        "https://api.groq.com/openai/v1",
        "https://api.mistral.ai/v1",
    ] {
        assert!(
            builtin_presets().endpoint_host_is_chat_completions_only(endpoint),
            "{endpoint} is a chat-completions-only built-in host"
        );
    }
}

#[test]
fn openai_host_and_unknown_proxies_keep_the_responses_fallback() {
    // OpenAI's first-party host serves /responses — must NOT be gated off,
    // even via a custom proxy slug pointed at it.
    assert!(!builtin_presets().endpoint_host_is_chat_completions_only("https://api.openai.com/v1"));
    // Genuinely unknown proxy hosts keep the permissive default (they may be
    // real OpenAI proxies that implement /responses).
    for endpoint in [
        "https://my-llm-proxy.internal.example/v1",
        "https://litellm.mycorp.dev/v1",
        "",
    ] {
        assert!(
            !builtin_presets().endpoint_host_is_chat_completions_only(endpoint),
            "{endpoint:?} is an unknown host and must keep the fallback"
        );
    }
}

/// Drift guard: the host-based gate must agree with the slug-based
/// capability for every built-in preset's own endpoint, so adding a preset
/// can't silently desync the two gates.
#[test]
fn host_gate_agrees_with_slug_capability_for_every_builtin() {
    for provider in builtin_cloud_providers().iter() {
        // OpenhumanJwt / Anthropic presets never route through the
        // OpenAI-compatible Responses fallback; the gate only matters for
        // the Bearer OpenAI-compatible hosts.
        if provider.auth_style != AuthStyle::Bearer {
            continue;
        }
        let host_chat_only =
            builtin_presets().endpoint_host_is_chat_completions_only(provider.endpoint);
        let slug_supports = builtin_presets().builtin_cloud_supports_responses_api(provider.slug);
        assert_eq!(
            host_chat_only, !slug_supports,
            "host gate for built-in {} disagrees with its slug capability",
            provider.slug
        );
    }
}

#[test]
fn auth_styles_expose_stable_wire_properties() {
    let styles = [
        (AuthStyle::Bearer, "bearer"),
        (AuthStyle::Anthropic, "anthropic"),
        (AuthStyle::OpenhumanJwt, "openhuman_jwt"),
        (AuthStyle::None, "none"),
    ];
    for (style, expected) in styles {
        assert_eq!(style.as_str(), expected);
    }
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

#[test]
fn builtin_table_is_only_the_upstream_provider_catalog() {
    use tinyinference_providers::builtin::BUILTIN_CLOUD_PROVIDERS as UPSTREAM;
    let host = builtin_cloud_providers();
    assert_eq!(host.len(), UPSTREAM.len());
    for (mapped, upstream) in host.iter().zip(UPSTREAM) {
        assert_eq!(mapped.slug, upstream.slug);
        assert_eq!(mapped.label, upstream.label);
        assert_eq!(mapped.endpoint, upstream.endpoint);
        let expected = match upstream.auth_style {
            tinyinference_providers::builtin::BuiltinAuthStyle::Bearer => AuthStyle::Bearer,
            tinyinference_providers::builtin::BuiltinAuthStyle::Anthropic => AuthStyle::Anthropic,
        };
        assert_eq!(mapped.auth_style, expected, "{}", mapped.slug);
    }
}
