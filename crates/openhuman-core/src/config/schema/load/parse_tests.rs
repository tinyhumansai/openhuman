use super::*;

const SAMPLE: &str = r#"
default_model = "parse-test-model"
default_temperature = 0.4

[agent]
agent_timeout_secs = 77
"#;

#[test]
fn parses_the_same_config_as_direct_toml() {
    let via_value = config_from_toml_str(SAMPLE).expect("sample parses");
    let direct: Config = toml::from_str(SAMPLE).expect("sample parses directly");
    assert_eq!(
        serde_json::to_value(&via_value).unwrap(),
        serde_json::to_value(&direct).unwrap()
    );
    assert_eq!(via_value.default_model.as_deref(), Some("parse-test-model"));
}

#[test]
fn an_empty_document_matches_direct_toml() {
    let via_value = config_from_toml_str("").expect("empty document parses");
    let direct: Config = toml::from_str("").expect("empty document parses directly");
    assert_eq!(
        serde_json::to_value(&via_value).unwrap(),
        serde_json::to_value(&direct).unwrap()
    );
}

#[test]
fn legacy_configuration_with_retired_sections_still_parses() {
    let legacy = format!(
        r#"
        [node]
        enabled = true
        version = "22.0.0"

        [runtime_python]
        enabled = true
        minimum_version = "3.12"

        [runtime_pool]
        enabled = true

        [tokenjuice]
        {}
        {}
        {}
        {}
        {}
        {}
    "#,
        concat!("ml_", "compression_enabled = true"),
        concat!("ml_", "model_id = \"old/model\""),
        concat!("ml_", "target_ratio = 0.6"),
        concat!("ml_", "sidecar_idle_timeout_secs = 60"),
        concat!("ml_", "max_input_chars = 5000"),
        concat!("ml_", "device = \"cpu\""),
    );
    let config = config_from_toml_str(&legacy).expect("legacy config still parses");
    assert!(config.tokenjuice.router_enabled);
}

#[test]
fn stale_configured_agents_are_dropped_before_typed_decode() {
    let legacy = config_from_toml_str(
        r#"
default_model = "preserved-model"

[agents.research]
model = "old/configured-model"
system_prompt = "retired"
"#,
    )
    .expect("legacy config with configured agents still loads");

    assert_eq!(legacy.default_model.as_deref(), Some("preserved-model"));
    let saved = toml::to_string(&legacy).expect("config serializes");
    assert!(!saved.contains("[agents"), "stale table survived: {saved}");

    let reloaded = config_from_toml_str(&saved).expect("migrated config reloads");
    assert_eq!(reloaded.default_model.as_deref(), Some("preserved-model"));
    assert_eq!(toml::to_string(&reloaded).unwrap(), saved);
}

#[test]
fn a_syntax_error_reports_the_toml_location() {
    let err = config_from_toml_str("default_model = \n[broken").unwrap_err();
    assert!(err.to_string().contains("line"), "{err}");
}

#[test]
fn a_type_error_is_an_error() {
    assert!(config_from_toml_str("default_temperature = \"hot\"").is_err());
}

#[test]
fn a_type_error_names_the_field_path() {
    let err = config_from_toml_str("[agent]\nagent_timeout_secs = \"soon\"").unwrap_err();
    assert!(
        err.to_string().starts_with("agent.agent_timeout_secs: "),
        "{err}"
    );
}

#[test]
fn retired_sandbox_backends_fail_with_a_migration_message() {
    for backend in ["firejail", "bubblewrap"] {
        let error = config_from_toml_str(&format!("[sandbox]\nbackend = \"{backend}\""))
            .expect_err("unsupported legacy sandbox backend must be rejected during config load");

        assert!(
            error.to_string().contains("TinyBox"),
            "expected a TinyBox migration hint for {backend}: {error}"
        );
    }
}

#[test]
fn a_non_finite_float_is_rejected_with_its_field_path() {
    let err = config_from_toml_str("default_temperature = nan").unwrap_err();
    assert!(
        err.to_string().starts_with("default_temperature: "),
        "{err}"
    );
}

#[test]
fn a_non_finite_optional_float_is_rejected_not_unset() {
    let err = config_from_toml_str("[agent]\nnested = { values = [1.0, inf] }").unwrap_err();
    assert_eq!(
        err.to_string(),
        "agent.nested.values[1]: non-finite float inf is not a valid config value"
    );
}

#[test]
fn legacy_cloud_provider_document_migrates_before_typed_decode_and_keeps_model() {
    let config = config_from_toml_str(
        r#"
schema_version = 13
[[cloud_providers]]
id = "legacy-anthropic"
type = "anthropic"
default_model = "claude-sonnet-4"
"#,
    )
    .expect("legacy config parses");
    let provider = &config.cloud_providers[0];
    assert_eq!(provider.slug, "anthropic");
    assert_eq!(provider.label, "Anthropic");
    assert_eq!(provider.endpoint, "https://api.anthropic.com/v1");
    assert_eq!(
        provider.auth_style,
        crate::config::schema::cloud_providers::AuthStyle::Anthropic
    );
    let saved = toml::to_string(&config).unwrap();
    let reloaded = config_from_toml_str(&saved).unwrap();
    assert_eq!(
        reloaded.cloud_providers[0].default_model.as_deref(),
        Some("claude-sonnet-4")
    );
    assert!(!saved.contains("type = \"anthropic\""));
}

#[tokio::test]
async fn old_cloud_provider_config_is_persisted_once_without_losing_its_model() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(
        &config_path,
        r#"
schema_version = 13
[[cloud_providers]]
id = "legacy-anthropic"
type = "anthropic"
default_model = "claude-sonnet-4"
"#,
    )
    .unwrap();
    let loaded = Config::load_from_config_path(&config_path, &workspace)
        .await
        .unwrap();
    assert_eq!(loaded.schema_version, 14);
    let once = std::fs::read_to_string(&config_path).unwrap();
    assert!(once.contains("schema_version = 14"));
    assert!(!once.contains("type = \"anthropic\""));
    let reloaded = Config::load_from_config_path(&config_path, &workspace)
        .await
        .unwrap();
    assert_eq!(
        reloaded.cloud_providers[0].default_model.as_deref(),
        Some("claude-sonnet-4")
    );
    assert_eq!(std::fs::read_to_string(config_path).unwrap(), once);
}

#[tokio::test]
async fn legacy_dictation_and_voice_settings_load_and_save_only_the_stt_engine() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(
        &config_path,
        r#"
[dictation]
enabled = true
hotkey = "ctrl+shift+d"
activation_mode = "toggle"
llm_refinement = false
streaming = false
streaming_interval_ms = 700

[voice_server]
auto_start = true
hotkey = "ctrl+shift+v"
activation_mode = "tap"
stt_engine = "elevenlabs"
skip_cleanup = true
min_duration_secs = 0.8
silence_threshold = 0.02
custom_dictionary = ["OpenHuman"]
"#,
    )
    .unwrap();

    let config = Config::load_from_config_path(&config_path, &workspace)
        .await
        .unwrap();
    assert_eq!(
        config.voice_server.stt_engine,
        crate::config::schema::SttEngine::Elevenlabs
    );
    config.save().await.unwrap();

    let saved: toml::Value =
        toml::from_str(&std::fs::read_to_string(config_path).unwrap()).unwrap();
    let voice_server = saved["voice_server"].as_table().unwrap();
    assert_eq!(
        voice_server.len(),
        1,
        "only the active engine setting should remain"
    );
    assert_eq!(voice_server["stt_engine"].as_str(), Some("elevenlabs"));
    assert!(
        saved.get("dictation").is_none(),
        "retired dictation data must be dropped on save"
    );
}

/// TinyMemes left OpenHuman: a config written while it was here still carries
/// `[components] memes` and a `[memes]` section. Both must load as ignored
/// keys (never a parse failure or a fallback to defaults) and be dropped on
/// the next save.
#[tokio::test]
async fn retired_memes_settings_load_as_ignored_keys_and_are_dropped_on_save() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(
        &config_path,
        r#"
default_model = "memes-retired-model"

[components]
memes = false
search = false

[memes]
timeout_ms = 5000
meme_cooldown = 7
"#,
    )
    .unwrap();

    let config = Config::load_from_config_path(&config_path, &workspace)
        .await
        .unwrap();
    assert_eq!(config.default_model.as_deref(), Some("memes-retired-model"));
    assert!(
        !config.components.search,
        "known component keys still apply"
    );
    assert!(
        config.components.flows,
        "unnamed components keep their default"
    );
    let diagnostics = config.load_diagnostics();
    assert_eq!(
        diagnostics.fallback,
        super::super::diagnostics::ConfigLoadFallback::None
    );
    // An unknown section is reported once, as the section itself.
    for key in ["components.memes", "memes"] {
        assert!(
            diagnostics.unknown_keys.iter().any(|k| k == key),
            "{key} should be reported as ignored: {:?}",
            diagnostics.unknown_keys
        );
    }

    config.save().await.unwrap();
    let saved: toml::Value =
        toml::from_str(&std::fs::read_to_string(config_path).unwrap()).unwrap();
    assert!(saved.get("memes").is_none(), "[memes] is dropped on save");
    assert!(
        saved["components"].get("memes").is_none(),
        "components.memes is dropped on save"
    );
    assert_eq!(saved["components"]["search"].as_bool(), Some(false));
}
