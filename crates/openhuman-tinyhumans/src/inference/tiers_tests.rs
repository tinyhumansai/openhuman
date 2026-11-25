use super::*;

fn normalize_with_hosted_registration(config: &mut Config) -> bool {
    let _registry = crate::registry_test_lock();
    crate::register::register_all(
        &crate::install::InstallOptions::default(),
        crate::register::DoctorRegistration::Process,
    );
    super::normalize_config(config)
}

#[test]
fn hosted_normalizer_rewrites_legacy_tiers_and_preserves_concrete_pins() {
    let mut config = Config {
        default_model: Some("reasoning-v1".to_string()),
        ..Config::default()
    };
    config.orchestrator.model = Some("custom/orchestrator-model".to_string());
    assert!(normalize_with_hosted_registration(&mut config));
    assert_eq!(config.default_model.as_deref(), Some(MODEL_MANAGED_DEFAULT));
    assert_eq!(
        config.orchestrator.model.as_deref(),
        Some("custom/orchestrator-model")
    );
    assert!(!normalize_with_hosted_registration(&mut config));
}

#[test]
fn hosted_normalizer_covers_all_legacy_model_fields_and_preserves_non_tiers() {
    let mut config = Config {
        default_model: Some("  reasoning-v1  ".to_string()),
        ..Config::default()
    };
    config.orchestrator.model = Some("reasoning-quick-v1".to_string());
    config.teams.insert(
        "team".to_string(),
        openhuman_embed::__host::config::schema::TeamModelConfig {
            lead_model: Some("chat-v1".to_string()),
            agent_model: Some("burst-v1".to_string()),
        },
    );
    config
        .model_routes
        .push(openhuman_embed::__host::config::schema::ModelRouteConfig {
            hint: "summary".to_string(),
            model: "summarization-v1".to_string(),
        });
    config.model_routes.extend([
        openhuman_embed::__host::config::schema::ModelRouteConfig {
            hint: "custom".to_string(),
            model: "vendor/pinned".to_string(),
        },
        openhuman_embed::__host::config::schema::ModelRouteConfig {
            hint: "unknown".to_string(),
            model: "hint:unknown".to_string(),
        },
        openhuman_embed::__host::config::schema::ModelRouteConfig {
            hint: "blank".to_string(),
            model: String::new(),
        },
    ]);

    assert!(normalize_with_hosted_registration(&mut config));
    assert_eq!(config.default_model.as_deref(), Some(MODEL_MANAGED_DEFAULT));
    assert_eq!(
        config.orchestrator.model.as_deref(),
        Some(MODEL_MANAGED_DEFAULT)
    );
    assert_eq!(
        config.teams["team"].lead_model.as_deref(),
        Some(MODEL_MANAGED_DEFAULT)
    );
    assert_eq!(
        config.teams["team"].agent_model.as_deref(),
        Some(MODEL_MANAGED_DEFAULT)
    );
    assert_eq!(config.model_routes[0].model, MODEL_MANAGED_DEFAULT);
    assert_eq!(config.model_routes[1].model, "vendor/pinned");
    assert_eq!(config.model_routes[2].model, "hint:unknown");
    assert_eq!(config.model_routes[3].model, "");
    assert!(!normalize_with_hosted_registration(&mut config));

    let mut absent = Config {
        default_model: None,
        ..Config::default()
    };
    assert!(normalize_with_hosted_registration(&mut absent));
    assert_eq!(absent.default_model, None);
    assert!(!normalize_with_hosted_registration(&mut absent));
}

#[test]
fn hosted_normalizer_seeds_its_preset_using_the_configured_api_url_once() {
    let mut config = Config {
        default_model: None,
        api_url: Some("https://custom.example/api".to_string()),
        ..Config::default()
    };

    assert!(normalize_with_hosted_registration(&mut config));
    assert_eq!(config.default_model, None);
    assert_eq!(config.cloud_providers.len(), 1);
    let provider = &config.cloud_providers[0];
    assert_eq!(provider.slug, "openhuman");
    assert_eq!(provider.endpoint, "https://custom.example/api");
    let id = provider.id.clone();

    assert!(!normalize_with_hosted_registration(&mut config));
    assert_eq!(config.cloud_providers.len(), 1);
    assert_eq!(config.cloud_providers[0].id, id);
}

#[test]
fn hosted_normalizer_selects_existing_managed_entry_for_legacy_managed_endpoint() {
    let mut config = Config {
        primary_cloud: None,
        inference_url: Some("https://api.managed.test/v1".into()),
        ..Config::default()
    };
    config.resolved_endpoints.managed_hosts = vec![".managed.test".into()];
    let managed_id = "p_existing_openhuman".to_string();
    config.cloud_providers.push(
        openhuman_embed::__host::config::schema::cloud_providers::CloudProviderCreds {
            id: managed_id.clone(),
            slug: "openhuman".into(),
            endpoint: "https://api.managed.test/v1".into(),
            ..Default::default()
        },
    );

    let _ = normalize_with_hosted_registration(&mut config);
    assert_eq!(config.primary_cloud.as_deref(), Some(managed_id.as_str()));
    assert!(!normalize_with_hosted_registration(&mut config));
    assert_eq!(config.primary_cloud.as_deref(), Some(managed_id.as_str()));
}

#[test]
fn hosted_normalizer_leaves_custom_and_explicit_primary_choices_alone() {
    let mut custom = Config {
        primary_cloud: None,
        inference_url: Some("https://custom.example/v1".into()),
        ..Config::default()
    };
    custom.cloud_providers.push(
        openhuman_embed::__host::config::schema::cloud_providers::CloudProviderCreds {
            id: "p_hosted".into(),
            slug: "openhuman".into(),
            endpoint: "https://api.managed.test/v1".into(),
            ..Default::default()
        },
    );
    assert!(!normalize_with_hosted_registration(&mut custom));
    assert_eq!(custom.primary_cloud, None);

    let mut selected = custom.clone();
    selected.primary_cloud = Some("p_custom".into());
    assert!(!normalize_with_hosted_registration(&mut selected));
    assert_eq!(selected.primary_cloud.as_deref(), Some("p_custom"));
}

#[test]
fn hosted_cost_defaults_include_the_registered_managed_model_price() {
    let _registry = crate::registry_test_lock();
    crate::register::register_all(
        &crate::install::InstallOptions::default(),
        crate::register::DoctorRegistration::Process,
    );

    let prices = openhuman_embed::__host::config::Config::default()
        .cost
        .prices;
    let expected = super::super::tiers::default_pricing().expect("hosted default pricing");
    assert_eq!(expected.0, MODEL_MANAGED_DEFAULT);
    let actual = prices
        .get(MODEL_MANAGED_DEFAULT)
        .expect("hosted cost defaults expose their owned managed price");
    assert!((actual.input - expected.1).abs() < f64::EPSILON);
    assert!((actual.output - expected.2).abs() < f64::EPSILON);
}

#[test]
fn managed_tier_vocabulary_maps_aliases_and_leaves_concrete_ids_raw() {
    for (model, role) in [
        ("reasoning-v1", "reasoning"),
        ("chat-v1", "chat"),
        ("agentic-v1", "agentic"),
        ("burst-v1", "burst"),
        ("coding-v1", "coding"),
        ("vision-v1", "vision"),
        ("summarization-v1", "summarization"),
        ("reasoning-quick-v1", "chat"),
        ("hint:reasoning", "reasoning"),
        ("hint:coding", "coding"),
    ] {
        assert_eq!(role_for_model(model).as_deref(), Some(role));
        assert!(is_alias(model));
    }
    assert!(is_raw_model("vendor/concrete-model"));
    assert!(is_raw_model("chat"));
    assert_eq!(
        resolve_model("chat", &Config::default(), Some("coding")).as_deref(),
        Some("coding")
    );
    assert!(!is_raw_model("hint:unknown"));
}

#[test]
fn managed_hint_roles_follow_the_native_workload_route_order() {
    let native = tinyinference_llm::model::workload::WORKLOAD_ROUTE_TIERS
        .iter()
        .map(|route| route.strip_prefix("hint:").expect("native hint route"))
        .collect::<Vec<_>>();
    for (route, role) in tinyinference_llm::model::workload::WORKLOAD_ROUTE_TIERS
        .iter()
        .zip(native)
    {
        assert_eq!(role_for_model(route), Some(role.to_string()));
    }
}

#[test]
fn hosted_vision_capabilities_cover_managed_aliases_and_media_models() {
    for model in [
        MODEL_MANAGED_DEFAULT,
        "reasoning-v1",
        "hint:reasoning",
        "vision-v1",
        "hint:vision",
        MODEL_MEDIA_UNDERSTANDING,
        MODEL_IMAGE_GENERATION_AGENT,
        MODEL_VIDEO_GENERATION_AGENT,
    ] {
        assert!(supports_vision(model), "{model} should accept images");
    }

    for model in [
        "chat-v1",
        "hint:chat",
        "reasoning-quick-v1",
        "agentic-v1",
        "hint:agentic",
        "burst-v1",
        "hint:burst",
        "coding-v1",
        "hint:coding",
        "summarization-v1",
        "hint:summarization",
        "vendor/custom-text-model",
        "",
    ] {
        assert!(!supports_vision(model), "{model} should remain text-only");
    }
}

#[test]
fn hosted_tier_context_windows_retain_the_legacy_alias_metadata() {
    for (model, expected) in [
        ("reasoning-v1", 1_000_000),
        ("hint:reasoning", 1_000_000),
        ("chat-v1", 1_000_000),
        ("hint:chat", 1_000_000),
        ("reasoning-quick-v1", 1_000_000),
        ("summarization-v1", 1_000_000),
        ("agentic-v1", 200_000),
        ("hint:coding", 200_000),
        ("burst-v1", 128_000),
    ] {
        assert_eq!(super::context_window(model), Some(expected), "{model}");
    }
    assert_eq!(super::context_window("chat"), Some(1_000_000));
    assert!(is_raw_model("chat"));
    assert_eq!(
        resolve_model("chat", &Config::default(), Some("chat")).as_deref(),
        Some("chat")
    );
}
