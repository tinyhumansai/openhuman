use super::*;

#[test]
fn parser_records_unknown_keys_with_their_full_path() {
    let config =
        config_from_toml_str("[memroy]\nengine = \"x\"\n[agent]\nmemroy = \"x\"\n").unwrap();

    assert_eq!(
        config.load_diagnostics().unknown_keys,
        ["agent.memroy", "memroy"]
    );
}

#[tokio::test]
async fn recovery_records_backup_source_and_original_parse_error() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("config.toml");
    let backup_path = directory.path().join("config.toml.bak");
    let workspace = directory.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(&config_path, "default_model = \"unterminated\n").unwrap();
    std::fs::write(&backup_path, "default_model = \"backup-model\"\n").unwrap();

    let config = Config::load_from_config_path(&config_path, &workspace)
        .await
        .unwrap();

    let diagnostics = config.load_diagnostics();
    assert_eq!(diagnostics.fallback, ConfigLoadFallback::Backup);
    let parse_error = diagnostics.parse_error.as_ref().unwrap();
    assert_eq!(parse_error.path, config_path);
    assert!(
        parse_error.message.contains("line"),
        "{}",
        parse_error.message
    );
    assert_eq!(config.default_model.as_deref(), Some("backup-model"));
}

#[tokio::test]
async fn non_utf8_recovery_records_backup_source_and_config_path() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("config.toml");
    let backup_path = directory.path().join("config.toml.bak");
    let workspace = directory.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(&config_path, [0xff, 0xfe]).unwrap();
    std::fs::write(&backup_path, "default_model = \"backup-model\"\n").unwrap();

    let config = Config::load_from_config_path(&config_path, &workspace)
        .await
        .unwrap();

    let diagnostics = config.load_diagnostics();
    assert_eq!(diagnostics.fallback, ConfigLoadFallback::Backup);
    let parse_error = diagnostics.parse_error.as_ref().unwrap();
    assert_eq!(parse_error.path, config_path);
    assert_eq!(
        parse_error.message,
        "config file could not be decoded as UTF-8"
    );
    assert_eq!(config.default_model.as_deref(), Some("backup-model"));
}

#[tokio::test]
async fn loader_records_legacy_inference_url_migration() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("config.toml");
    let workspace = directory.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(
        &config_path,
        "api_url = \"https://example.test/v1/chat/completions\"\n",
    )
    .unwrap();

    let config = Config::load_from_config_path(&config_path, &workspace)
        .await
        .unwrap();

    assert!(config
        .load_diagnostics()
        .migrated_keys
        .iter()
        .any(|key| key == "api_url"));
    assert_eq!(
        config.inference_url.as_deref(),
        Some("https://example.test/v1/chat/completions")
    );
}

#[test]
fn default_config_has_empty_load_diagnostics() {
    let config = Config::default();

    assert!(config.load_diagnostics().unknown_keys.is_empty());
    assert!(config.load_diagnostics().parse_error.is_none());
    assert_eq!(config.load_diagnostics().fallback, ConfigLoadFallback::None);
    assert!(config.load_diagnostics().migrated_keys.is_empty());
    assert!(serde_json::to_value(&config)
        .unwrap()
        .get("load_diagnostics")
        .is_none());
}
