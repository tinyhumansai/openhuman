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
fn output_language_directive_strips_control_characters_and_blank_input() {
    let directive = output_language_directive(Some("  Klingon\u{0000}  ")).expect("directive");
    assert!(directive.contains("write all natural-language output in Klingon."));
    assert_eq!(output_language_directive(Some("\u{0000}\u{0001}")), None);
    assert_eq!(output_language_directive(Some("   ")), None);
    assert_eq!(output_language_directive(None), None);
}

#[test]
fn config_parses_orchestrator_and_team_model_pins() {
    let config: Config = toml::from_str(
        r#"
            [orchestrator]
            model = "deepseek/deepseek-r2"

            [teams.tools]
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
        config.configured_agent_model("tools", false),
        Some("deepseek/v3.2")
    );
    assert_eq!(
        config.configured_agent_model("tools", true),
        Some("minimax/m3")
    );
    // `<name>_agent` falls back to `[teams.<name>]`.
    assert_eq!(
        config.configured_agent_model("code_agent", false),
        Some("qwen/qwen3")
    );
    // The retired built-in aliases no longer resolve.
    assert_eq!(config.configured_agent_model("code_executor", false), None);
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
        "tools".to_string(),
        TeamModelConfig {
            lead_model: Some("".to_string()),
            agent_model: Some("  ".to_string()),
        },
    );

    assert_eq!(config.configured_agent_model("orchestrator", true), None);
    assert_eq!(config.configured_agent_model("tools_agent", false), None);
}

#[test]
fn workload_local_model_trims_and_only_honours_ollama_providers() {
    let mut config = Config::default();
    config.chat_provider = Some(" ollama:chat-local ".into());
    config.reasoning_provider = Some("cloud".into());
    config.agentic_provider = Some("ollama:agent-local".into());
    config.coding_provider = Some("ollama:code-local".into());
    config.memory_provider = Some("ollama:memory-local".into());
    config.embeddings_provider = Some("ollama:embed-local".into());
    assert_eq!(
        config.workload_local_model("chat").as_deref(),
        Some("chat-local")
    );
    assert_eq!(config.workload_local_model("reasoning"), None);
    for workload in ["agentic", "coding", "memory", "embeddings"] {
        assert!(config.workload_uses_local(workload), "{workload}");
    }
    assert!(!config.workload_uses_local("unknown"));
    config.chat_provider = Some("ollama:   ".into());
    assert_eq!(config.workload_local_model("chat"), None);
}

#[test]
fn default_temperature_unsupported_models_suppress_reasoning_families_only() {
    use tinyinference_llm::model::effective_temperature;

    let config = Config::default();
    let unsupported = &config.temperature_unsupported_models;
    for model in ["gpt-4o-mini", "claude-3-sonnet"] {
        assert_eq!(
            effective_temperature(model, Some(0.7), None, unsupported),
            Some(0.7),
            "{model} keeps its temperature"
        );
    }
    for model in ["o1-preview", "o3-mini", "o4-turbo", "gpt-5-turbo"] {
        assert_eq!(
            effective_temperature(model, Some(0.7), None, unsupported),
            None,
            "{model} must have temperature suppressed"
        );
    }
}

/// The v1 learning workload is gone, but configs written by older builds still
/// carry its keys. They must keep loading (unknown keys are ignored, never an
/// error) and the surviving workload routes must be unaffected.
#[test]
fn config_with_retired_learning_keys_still_parses() {
    let config: Config = toml::from_str(
        r#"
learning_provider = "cloud"
memory_provider = "ollama:summary-local"

[local_ai]
runtime_enabled = true

[local_ai.usage]
embeddings = true
learning_reflection = true
"#,
    )
    .expect("config with retired learning keys must still load");
    assert_eq!(
        config.workload_local_model("memory").as_deref(),
        Some("summary-local")
    );
    assert_eq!(config.workload_local_model("learning"), None);
    assert!(config.local_ai.usage.embeddings);
}
