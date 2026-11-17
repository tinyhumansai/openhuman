use super::*;

#[test]
fn a_missing_block_enforces_the_checks() {
    let config: SecurityModeConfig = toml::from_str("").unwrap();
    assert_eq!(config.mode, SecurityMode::Enforce);
    assert!(config.enforces());
    assert!(SecurityModeConfig::default().enforces());
}

#[test]
fn mode_serializes_snake_case() {
    assert_eq!(
        serde_json::to_string(&SecurityMode::Enforce).unwrap(),
        "\"enforce\""
    );
    assert_eq!(
        serde_json::to_string(&SecurityMode::Off).unwrap(),
        "\"off\""
    );
}

#[test]
fn off_parses_from_toml_and_stops_enforcing() {
    let config: SecurityModeConfig = toml::from_str("mode = \"off\"").unwrap();
    assert_eq!(config, SecurityModeConfig::off());
    assert!(!config.enforces());
}

#[test]
fn an_unknown_mode_is_rejected_rather_than_read_as_off() {
    assert!(toml::from_str::<SecurityModeConfig>("mode = \"disabled\"").is_err());
    assert!(toml::from_str::<crate::config::Config>("[security]\nmode = \"none\"\n").is_err());
}

#[test]
fn a_config_file_reads_the_security_table_and_defaults_without_it() {
    let off: crate::config::Config = toml::from_str("[security]\nmode = \"off\"\n").unwrap();
    assert_eq!(off.security, SecurityModeConfig::off());

    let default: crate::config::Config = toml::from_str("").unwrap();
    assert!(default.security.enforces());

    let saved = toml::to_string(&off).unwrap();
    let reloaded: crate::config::Config = toml::from_str(&saved).unwrap();
    assert_eq!(reloaded.security, SecurityModeConfig::off());
}

#[test]
fn the_judge_is_on_unless_a_config_says_otherwise() {
    let config: SecurityModeConfig = toml::from_str("").unwrap();
    assert_eq!(config.judge, JudgeMode::On);
    assert_eq!(SecurityModeConfig::default().judge, JudgeMode::On);
}

#[test]
fn the_judge_can_be_turned_off_on_its_own() {
    let config: SecurityModeConfig = toml::from_str("judge = \"off\"").unwrap();
    assert_eq!(config.judge, JudgeMode::Off);
    assert!(config.enforces());
}

#[test]
fn a_judge_route_never_prints_its_key() {
    let route = SecurityJudgeRoute::open_router("a-working-key");
    assert!(!format!("{route:?}").contains("a-working-key"));
}
