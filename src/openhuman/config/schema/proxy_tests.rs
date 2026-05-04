use super::*;

// ── normalize_proxy_url_option ─────────────────────────────────

#[test]
fn normalize_proxy_url_option_handles_none_empty_and_valid() {
    assert_eq!(normalize_proxy_url_option(None), None);
    assert_eq!(normalize_proxy_url_option(Some("")), None);
    assert_eq!(normalize_proxy_url_option(Some("   ")), None);
    assert_eq!(
        normalize_proxy_url_option(Some("  http://proxy:8080  ")),
        Some("http://proxy:8080".to_string())
    );
}

// ── normalize_comma_values / normalize_service_list / normalize_no_proxy_list ─

#[test]
fn normalize_comma_values_splits_trims_and_dedups() {
    let out = normalize_comma_values(vec!["a,b".into(), " c,a  ".into(), "".into()]);
    assert_eq!(out, vec!["a", "b", "c"]);
}

#[test]
fn normalize_comma_values_empty_input_returns_empty() {
    assert!(normalize_comma_values(vec![]).is_empty());
    assert!(normalize_comma_values(vec!["".into(), " ".into()]).is_empty());
}

#[test]
fn normalize_service_list_lowercases_and_dedups() {
    let out = normalize_service_list(vec!["OPENAI".into(), "openai".into(), "Anthropic".into()]);
    assert_eq!(out, vec!["anthropic", "openai"]);
}

#[test]
fn normalize_no_proxy_list_preserves_case() {
    let out = normalize_no_proxy_list(vec!["localhost,127.0.0.1".into()]);
    assert_eq!(out, vec!["127.0.0.1", "localhost"]);
}

// ── parse_proxy_scope ──────────────────────────────────────────

#[test]
fn parse_proxy_scope_accepts_known_aliases() {
    assert_eq!(
        parse_proxy_scope("environment"),
        Some(ProxyScope::Environment)
    );
    assert_eq!(parse_proxy_scope("env"), Some(ProxyScope::Environment));
    assert_eq!(parse_proxy_scope("ENV"), Some(ProxyScope::Environment));
    assert_eq!(parse_proxy_scope("openhuman"), Some(ProxyScope::OpenHuman));
    assert_eq!(parse_proxy_scope("internal"), Some(ProxyScope::OpenHuman));
    assert_eq!(parse_proxy_scope("core"), Some(ProxyScope::OpenHuman));
    assert_eq!(parse_proxy_scope("services"), Some(ProxyScope::Services));
    assert_eq!(parse_proxy_scope("service"), Some(ProxyScope::Services));
    assert_eq!(
        parse_proxy_scope("  SERVICES  "),
        Some(ProxyScope::Services)
    );
}

#[test]
fn parse_proxy_scope_rejects_unknown() {
    assert!(parse_proxy_scope("").is_none());
    assert!(parse_proxy_scope("other").is_none());
}

// ── parse_proxy_enabled ────────────────────────────────────────

#[test]
fn parse_proxy_enabled_accepts_truthy_and_falsy() {
    for t in ["1", "true", "yes", "on", "TRUE", " YES "] {
        assert_eq!(
            parse_proxy_enabled(t),
            Some(true),
            "`{t}` should parse truthy"
        );
    }
    for f in ["0", "false", "no", "off", "FALSE"] {
        assert_eq!(
            parse_proxy_enabled(f),
            Some(false),
            "`{f}` should parse falsy"
        );
    }
    assert_eq!(parse_proxy_enabled(""), None);
    assert_eq!(parse_proxy_enabled("nope"), None);
}

// ── ProxyConfig::default / has_any_proxy_url ──────────────────

#[test]
fn proxy_config_default_has_no_urls() {
    let c = ProxyConfig::default();
    assert!(!c.has_any_proxy_url());
}

#[test]
fn proxy_config_has_any_proxy_url_detects_each_url_field() {
    let mut c = ProxyConfig::default();
    c.http_proxy = Some("http://h:8080".into());
    assert!(c.has_any_proxy_url());
    let mut c = ProxyConfig::default();
    c.https_proxy = Some("https://h:8443".into());
    assert!(c.has_any_proxy_url());
    let mut c = ProxyConfig::default();
    c.all_proxy = Some("socks5://h:1080".into());
    assert!(c.has_any_proxy_url());
}

#[test]
fn proxy_config_has_any_proxy_url_ignores_whitespace_urls() {
    let mut c = ProxyConfig::default();
    c.http_proxy = Some("   ".into());
    c.https_proxy = Some("".into());
    assert!(!c.has_any_proxy_url());
}

// ── is_supported_proxy_service_selector ────────────────────────

#[test]
fn is_supported_proxy_service_selector_accepts_known_keys_case_insensitive() {
    for key in SUPPORTED_PROXY_SERVICE_KEYS {
        assert!(is_supported_proxy_service_selector(key));
        assert!(is_supported_proxy_service_selector(
            &key.to_ascii_uppercase()
        ));
    }
    for sel in SUPPORTED_PROXY_SERVICE_SELECTORS {
        assert!(is_supported_proxy_service_selector(sel));
    }
    assert!(!is_supported_proxy_service_selector("not-a-selector-xyz"));
}

// ── service_selector_matches ───────────────────────────────────

#[test]
fn service_selector_matches_exact_and_wildcard() {
    assert!(service_selector_matches("openai", "openai"));
    assert!(!service_selector_matches("openai", "anthropic"));
    // Wildcard prefix: `foo.*` matches `foo.bar` but not `foo` or `foobar`.
    assert!(service_selector_matches("foo.*", "foo.bar"));
    assert!(service_selector_matches("foo.*", "foo.bar.baz"));
    assert!(!service_selector_matches("foo.*", "foo"));
    assert!(!service_selector_matches("foo.*", "foobar"));
}

// ── validate_proxy_url ─────────────────────────────────────────

#[test]
fn validate_proxy_url_accepts_supported_schemes_with_host() {
    assert!(validate_proxy_url("http_proxy", "http://proxy:8080").is_ok());
    assert!(validate_proxy_url("https_proxy", "https://proxy:8443").is_ok());
    assert!(validate_proxy_url("all_proxy", "socks5://proxy:1080").is_ok());
    assert!(validate_proxy_url("all_proxy", "socks5h://proxy:1080").is_ok());
}

#[test]
fn validate_proxy_url_rejects_unsupported_schemes() {
    let err = validate_proxy_url("x", "ftp://proxy:21").unwrap_err();
    assert!(err.to_string().contains("Invalid"));
}

#[test]
fn validate_proxy_url_rejects_missing_host() {
    // e.g. scheme-only URL parses but has no host
    let err = validate_proxy_url("x", "http://").unwrap_err();
    assert!(err.to_string().to_lowercase().contains("invalid"));
}

#[test]
fn validate_proxy_url_rejects_malformed_url() {
    let err = validate_proxy_url("x", "not a url").unwrap_err();
    assert!(err.to_string().to_lowercase().contains("invalid"));
}

// ── ProxyConfig::validate ─────────────────────────────────────

#[test]
fn validate_disabled_proxy_always_ok() {
    let c = ProxyConfig::default();
    assert!(c.validate().is_ok());
}

#[test]
fn validate_enabled_without_url_fails() {
    let c = ProxyConfig {
        enabled: true,
        ..Default::default()
    };
    let err = c.validate().unwrap_err();
    assert!(err.to_string().contains("no proxy URL"));
}

#[test]
fn validate_enabled_with_url_ok() {
    let c = ProxyConfig {
        enabled: true,
        http_proxy: Some("http://proxy:8080".into()),
        ..Default::default()
    };
    assert!(c.validate().is_ok());
}

#[test]
fn validate_services_scope_empty_services_fails() {
    let c = ProxyConfig {
        enabled: true,
        http_proxy: Some("http://proxy:8080".into()),
        scope: ProxyScope::Services,
        services: vec![],
        ..Default::default()
    };
    let err = c.validate().unwrap_err();
    assert!(err.to_string().contains("non-empty"));
}

#[test]
fn validate_services_scope_with_valid_services_ok() {
    let c = ProxyConfig {
        enabled: true,
        http_proxy: Some("http://proxy:8080".into()),
        scope: ProxyScope::Services,
        services: vec!["provider.openai".into()],
        ..Default::default()
    };
    assert!(c.validate().is_ok());
}

#[test]
fn validate_unsupported_service_selector_fails() {
    let c = ProxyConfig {
        enabled: true,
        http_proxy: Some("http://proxy:8080".into()),
        scope: ProxyScope::Services,
        services: vec!["not.a.valid.selector".into()],
        ..Default::default()
    };
    let err = c.validate().unwrap_err();
    assert!(err.to_string().contains("Unsupported"));
}

#[test]
fn validate_bad_proxy_url_fails() {
    let c = ProxyConfig {
        enabled: true,
        http_proxy: Some("ftp://bad:21".into()),
        ..Default::default()
    };
    let err = c.validate().unwrap_err();
    assert!(err.to_string().contains("Invalid"));
}

// ── should_apply_to_service ───────────────────────────────────

#[test]
fn should_apply_disabled_always_false() {
    let c = ProxyConfig::default();
    assert!(!c.should_apply_to_service("anything"));
}

#[test]
fn should_apply_environment_scope_always_false() {
    let c = ProxyConfig {
        enabled: true,
        http_proxy: Some("http://p:8080".into()),
        scope: ProxyScope::Environment,
        ..Default::default()
    };
    assert!(!c.should_apply_to_service("provider.openai"));
}

#[test]
fn should_apply_openhuman_scope_always_true() {
    let c = ProxyConfig {
        enabled: true,
        http_proxy: Some("http://p:8080".into()),
        scope: ProxyScope::OpenHuman,
        ..Default::default()
    };
    assert!(c.should_apply_to_service("provider.openai"));
    assert!(c.should_apply_to_service("anything"));
}

#[test]
fn should_apply_services_scope_matches_exact() {
    let c = ProxyConfig {
        enabled: true,
        http_proxy: Some("http://p:8080".into()),
        scope: ProxyScope::Services,
        services: vec!["provider.openai".into()],
        ..Default::default()
    };
    assert!(c.should_apply_to_service("provider.openai"));
    assert!(!c.should_apply_to_service("provider.anthropic"));
}

#[test]
fn should_apply_services_scope_matches_wildcard() {
    let c = ProxyConfig {
        enabled: true,
        http_proxy: Some("http://p:8080".into()),
        scope: ProxyScope::Services,
        services: vec!["provider.*".into()],
        ..Default::default()
    };
    assert!(c.should_apply_to_service("provider.openai"));
    assert!(c.should_apply_to_service("provider.anthropic"));
    assert!(!c.should_apply_to_service("channel.telegram"));
}

#[test]
fn should_apply_services_scope_empty_key_returns_false() {
    let c = ProxyConfig {
        enabled: true,
        http_proxy: Some("http://p:8080".into()),
        scope: ProxyScope::Services,
        services: vec!["provider.*".into()],
        ..Default::default()
    };
    assert!(!c.should_apply_to_service("  "));
}

// ── runtime_proxy_cache_key ───────────────────────────────────

#[test]
fn runtime_proxy_cache_key_with_timeouts() {
    let key = runtime_proxy_cache_key("provider.openai", Some(30), Some(10));
    assert_eq!(key, "provider.openai|timeout=30|connect_timeout=10");
}

#[test]
fn runtime_proxy_cache_key_without_timeouts() {
    let key = runtime_proxy_cache_key("provider.openai", None, None);
    assert_eq!(key, "provider.openai|timeout=none|connect_timeout=none");
}

#[test]
fn runtime_proxy_cache_key_trims_and_lowercases() {
    let key = runtime_proxy_cache_key("  Provider.OpenAI  ", None, None);
    assert!(key.starts_with("provider.openai"));
}

// ── ProxyConfig::normalized_services / normalized_no_proxy ────

#[test]
fn normalized_services_dedup_and_sort() {
    let c = ProxyConfig {
        services: vec![
            "provider.openai,provider.anthropic".into(),
            "provider.openai".into(),
        ],
        ..Default::default()
    };
    let norm = c.normalized_services();
    assert_eq!(norm, vec!["provider.anthropic", "provider.openai"]);
}

#[test]
fn normalized_no_proxy_dedup_and_sort() {
    let c = ProxyConfig {
        no_proxy: vec!["localhost,127.0.0.1".into(), "localhost".into()],
        ..Default::default()
    };
    let norm = c.normalized_no_proxy();
    assert_eq!(norm, vec!["127.0.0.1", "localhost"]);
}

// ── apply_to_reqwest_builder ─────────────────────────────────

#[test]
fn apply_to_reqwest_builder_skips_when_not_applicable() {
    let c = ProxyConfig::default(); // disabled
    let builder = reqwest::Client::builder();
    // Should just return the builder unchanged (no panic)
    let _builder = c.apply_to_reqwest_builder(builder, "anything");
}

#[test]
fn apply_to_reqwest_builder_applies_all_proxy() {
    let c = ProxyConfig {
        enabled: true,
        all_proxy: Some("http://proxy:8080".into()),
        scope: ProxyScope::OpenHuman,
        ..Default::default()
    };
    let builder = reqwest::Client::builder();
    let builder = c.apply_to_reqwest_builder(builder, "provider.openai");
    // Should build successfully
    let client = builder.build();
    assert!(client.is_ok());
}

#[test]
fn apply_to_reqwest_builder_applies_http_and_https_proxy() {
    let c = ProxyConfig {
        enabled: true,
        http_proxy: Some("http://proxy:8080".into()),
        https_proxy: Some("http://proxy:8443".into()),
        scope: ProxyScope::OpenHuman,
        ..Default::default()
    };
    let builder = reqwest::Client::builder();
    let builder = c.apply_to_reqwest_builder(builder, "test");
    assert!(builder.build().is_ok());
}

// ── supported_service_keys / selectors ─────────────────────────

#[test]
fn supported_service_keys_is_nonempty() {
    assert!(!ProxyConfig::supported_service_keys().is_empty());
}

#[test]
fn supported_service_selectors_is_nonempty() {
    assert!(!ProxyConfig::supported_service_selectors().is_empty());
}
