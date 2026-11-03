use super::*;

#[test]
fn a_missing_block_enforces_the_screen() {
    let config: PromptScreenConfig = toml::from_str("").unwrap();
    assert_eq!(config.mode, PromptScreenMode::Enforce);
    assert!(config.enforces());
    assert!(PromptScreenConfig::default().enforces());
}

#[test]
fn mode_serializes_snake_case() {
    assert_eq!(
        serde_json::to_string(&PromptScreenMode::Enforce).unwrap(),
        "\"enforce\""
    );
    assert_eq!(
        serde_json::to_string(&PromptScreenMode::Off).unwrap(),
        "\"off\""
    );
}

#[test]
fn off_parses_from_toml_and_stops_enforcing() {
    let config: PromptScreenConfig = toml::from_str("mode = \"off\"").unwrap();
    assert_eq!(config, PromptScreenConfig::off());
    assert!(!config.enforces());
}

#[test]
fn an_unknown_mode_is_rejected_rather_than_read_as_off() {
    assert!(toml::from_str::<PromptScreenConfig>("mode = \"disabled\"").is_err());
}
