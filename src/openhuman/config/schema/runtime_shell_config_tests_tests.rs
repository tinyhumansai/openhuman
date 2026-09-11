use super::ShellConfig;

#[test]
fn shell_config_defaults_hide_window_off() {
    // Backward compatibility: absent `[shell]` section must not change
    // behaviour, so `hide_window` defaults to false.
    assert!(!ShellConfig::default().hide_window);
}

#[test]
fn shell_config_parses_hide_window_from_toml() {
    let cfg: ShellConfig = toml::from_str("hide_window = true").unwrap();
    assert!(cfg.hide_window);
}

#[test]
fn shell_config_empty_table_keeps_default() {
    // An empty `[shell]` table relies on `#[serde(default)]` for the field.
    let cfg: ShellConfig = toml::from_str("").unwrap();
    assert!(!cfg.hide_window);
}
