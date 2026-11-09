use super::*;

#[tokio::test]
async fn registered_normalizer_runs_at_current_schema_and_retries_failed_save() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();
    let mut config = config_in(&tmp);
    config.schema_version = CURRENT_SCHEMA_VERSION;
    config.default_model = Some("legacy-value".to_string());
    let blocker = tmp.path().join("blocker");
    fs::write(&blocker, "not a directory").unwrap();
    config.config_path = blocker.join("nested").join("config.toml");

    crate::inference::registry::register_config_normalizer(
        "migration-test",
        std::sync::Arc::new(|config| {
            if config.default_model.as_deref() == Some("legacy-value") {
                config.default_model = Some("normalized-value".to_string());
                true
            } else {
                false
            }
        }),
    );
    run_pending(&mut config).await;
    assert_eq!(config.default_model.as_deref(), Some("legacy-value"));

    config.config_path = tmp.path().join("config.toml");
    run_pending(&mut config).await;
    assert_eq!(config.default_model.as_deref(), Some("normalized-value"));
    let on_disk = fs::read_to_string(&config.config_path).unwrap();
    assert!(on_disk.contains("normalized-value"));
}

#[tokio::test]
async fn recovered_current_config_is_normalized_in_memory_without_persisting() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();
    let mut config = config_in(&tmp);
    config.schema_version = CURRENT_SCHEMA_VERSION;
    config.default_model = Some("recover-me".to_string());
    config.save().await.unwrap();
    config.recovered_from_corruption = true;

    crate::inference::registry::register_config_normalizer(
        "recovered-migration-test",
        std::sync::Arc::new(|config| {
            if config.default_model.as_deref() == Some("recover-me") {
                config.default_model = Some("recovered-normalized".to_string());
                true
            } else {
                false
            }
        }),
    );

    run_pending(&mut config).await;

    assert_eq!(
        config.default_model.as_deref(),
        Some("recovered-normalized")
    );
    let on_disk = fs::read_to_string(&config.config_path).unwrap();
    assert!(on_disk.contains("recover-me"));
    assert!(!on_disk.contains("recovered-normalized"));
}
