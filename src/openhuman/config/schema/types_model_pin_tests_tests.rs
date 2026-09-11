use super::*;

#[test]
fn output_language_directive_maps_locales_and_preserves_json_keys() {
    for (tag, expected) in [
        ("zh-CN", "Simplified Chinese"),
        ("zh-TW", "Traditional Chinese"),
        ("zh_Hant", "Traditional Chinese"),
        ("ko", "Korean"),
        ("ja", "Japanese"),
        ("de", "German"),
        ("th", "Thai"),
        ("vi", "Vietnamese"),
        ("tr", "Turkish"),
    ] {
        let directive = output_language_directive(Some(tag)).expect("directive");
        assert!(
            directive.contains(expected),
            "{tag} should map to {expected}: {directive}"
        );
        assert!(directive.contains("Keep JSON keys"));
    }
}

#[test]
fn output_language_directive_accepts_language_names() {
    let directive = output_language_directive(Some("Kannada")).expect("directive");
    assert!(directive.contains("Kannada"));
}

#[test]
fn config_parses_orchestrator_and_team_model_pins() {
    let config: Config = toml::from_str(
        r#"
            [orchestrator]
            model = "deepseek/deepseek-r2"

            [teams.research]
            lead_model = "minimax/m3"
            agent_model = "deepseek/v3.2"

            [teams.code]
            agent_model = "qwen/qwen3"
        "#,
    )
    .expect("config should parse model pin tables");

    assert_eq!(
        config.configured_agent_model("orchestrator", true),
        Some("deepseek/deepseek-r2")
    );
    assert_eq!(
        config.configured_agent_model("researcher", false),
        Some("deepseek/v3.2")
    );
    assert_eq!(
        config.configured_agent_model("researcher", true),
        Some("minimax/m3")
    );
    assert_eq!(
        config.configured_agent_model("code_executor", false),
        Some("qwen/qwen3")
    );
}

#[test]
fn config_ignores_legacy_screen_intelligence_table() {
    let config: Config = toml::from_str(
        r#"
            [screen_intelligence]
            enabled = true
            baseline_fps = 30.0
        "#,
    )
    .expect("legacy screen intelligence TOML should be ignored");

    assert!(
        !toml::to_string(&config)
            .expect("config should serialize")
            .contains("screen_intelligence"),
        "legacy screen intelligence data must not be persisted again"
    );
}

#[test]
fn config_parses_capability_provider_entries() {
    let config: Config = toml::from_str(
        r#"
            [[capability_providers]]
            id = "Acme Tools"
            display_name = "Acme Tools"
            source_uri = "https://example.com/openhuman/acme-tools"
            source_digest = "sha256:abc123"
            trust_state = "trusted"
            enabled = true
        "#,
    )
    .expect("config should parse capability providers");

    assert_eq!(config.capability_providers.len(), 1);
    assert_eq!(config.capability_providers[0].id, "Acme Tools");
    assert_eq!(
        config.capability_providers[0].trust_state,
        CapabilityProviderTrustState::Trusted
    );
    assert!(config.capability_providers[0].enabled);
}

#[test]
fn empty_model_pin_values_fall_back_to_auto_routing() {
    let mut config = Config::default();
    config.orchestrator.model = Some("   ".to_string());
    config.teams.insert(
        "research".to_string(),
        TeamModelConfig {
            lead_model: Some("".to_string()),
            agent_model: Some("  ".to_string()),
        },
    );

    assert_eq!(config.configured_agent_model("orchestrator", true), None);
    assert_eq!(config.configured_agent_model("researcher", false), None);
}
