use super::*;

#[test]
fn load_missing_file_returns_safe_defaults() {
    let dir = std::env::temp_dir().join("oh_cc_settings_missing_test");
    let _ = std::fs::remove_dir_all(&dir);
    let s = load(&dir);
    assert!(!s.full_access, "missing settings must default to OFF");
}

#[test]
fn save_then_load_roundtrips() {
    let dir = std::env::temp_dir().join("oh_cc_settings_roundtrip_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    save(&dir, &ClaudeCodeSettings { full_access: true }).unwrap();
    assert!(
        load(&dir).full_access,
        "saved full_access=true must persist"
    );
    save(&dir, &ClaudeCodeSettings { full_access: false }).unwrap();
    assert!(
        !load(&dir).full_access,
        "toggling back to false must persist"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn load_corrupt_file_returns_safe_defaults() {
    let dir = std::env::temp_dir().join("oh_cc_settings_corrupt_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(settings_path(&dir), b"{not json").unwrap();
    assert!(
        !load(&dir).full_access,
        "corrupt settings must fail safe to OFF"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Pins **where** the settings file lands (#6075 part 2): beside `config.toml`
/// in the OpenHuman config dir.
///
/// The RPC schema description and this module's docs both name that location,
/// and they previously said "under the workspace" — which reads as
/// `Config::workspace_dir` (`~/.openhuman/workspace`) or the user's project
/// root. Building a config whose three candidate directories are all distinct
/// makes the assertion discriminating: it fails if the resolution ever moves to
/// either of the other two, so the docs cannot silently go stale again.
#[test]
fn save_for_config_writes_beside_config_toml_not_workspace_or_action_dir() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let config_dir = tmp.path().join("config-dir");
    let workspace_dir = config_dir.join("workspace");
    let action_dir = tmp.path().join("projects");
    std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
    std::fs::create_dir_all(&action_dir).expect("action dir");

    let config = crate::openhuman::config::Config {
        config_path: config_dir.join("config.toml"),
        workspace_dir: workspace_dir.clone(),
        action_dir: action_dir.clone(),
        ..Default::default()
    };

    let saved = save_full_access_for_config(&config, true).expect("save settings");
    assert!(saved.full_access);

    assert!(
        config_dir.join(SETTINGS_FILE).is_file(),
        "settings must land next to config.toml in {}",
        config_dir.display()
    );
    assert!(
        !workspace_dir.join(SETTINGS_FILE).exists(),
        "settings must NOT land in Config::workspace_dir"
    );
    assert!(
        !action_dir.join(SETTINGS_FILE).exists(),
        "settings must NOT land in Config::action_dir"
    );

    assert!(
        load_for_config(&config).full_access,
        "load_for_config must read back from the same directory it wrote to"
    );
}
