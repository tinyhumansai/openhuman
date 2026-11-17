use super::*;

#[tokio::test]
async fn trigger_settings_are_saved_to_config() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.workspace_dir = temp.path().join("workspace");
    config.config_path = temp.path().join("config.toml");
    std::fs::create_dir_all(&config.workspace_dir).unwrap();

    apply_composio_trigger_settings(
        &mut config,
        ComposioTriggerSettingsPatch {
            triage_disabled: Some(true),
            triage_disabled_toolkits: Some(vec!["github".into(), "slack".into()]),
        },
    )
    .await
    .expect("settings should be accepted");

    assert!(config.composio.triage_disabled);
    assert_eq!(
        config.composio.triage_disabled_toolkits,
        ["github", "slack"]
    );

    let saved = std::fs::read_to_string(&config.config_path).unwrap();
    let reloaded: Config = toml::from_str(&saved).unwrap();
    assert!(reloaded.composio.triage_disabled);
    assert_eq!(
        reloaded.composio.triage_disabled_toolkits,
        ["github", "slack"]
    );
}
