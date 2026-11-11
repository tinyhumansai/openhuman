use super::*;

#[test]
fn shell_timeout_defaults_to_the_historic_120_seconds() {
    let config = CronConfig::default();
    assert_eq!(config.shell_timeout_secs, 120);
    assert_eq!(config.shell_timeout(), std::time::Duration::from_secs(120));
    // An older config without the key keeps the default.
    let parsed: CronConfig = toml::from_str("enabled = true\n").expect("parse");
    assert_eq!(parsed.shell_timeout_secs, 120);
}

#[test]
fn shell_timeout_is_read_from_config_and_never_zero() {
    let parsed: CronConfig = toml::from_str("shell_timeout_secs = 900\n").expect("parse");
    assert_eq!(parsed.shell_timeout(), std::time::Duration::from_secs(900));
    let zero = CronConfig {
        shell_timeout_secs: 0,
        ..CronConfig::default()
    };
    assert_eq!(zero.shell_timeout(), std::time::Duration::from_secs(1));
}
