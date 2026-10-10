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
