use super::*;

#[test]
fn migration_preserves_explicit_values_and_unknown_provider_models() {
    let mut document: toml::Value = toml::from_str(
        r#"
schema_version = 13
[[cloud_providers]]
type = "anthropic"
slug = "chosen"
label = "Chosen Label"
endpoint = "https://proxy.example/v1"
auth_style = "bearer"
default_model = "claude-sonnet-4"
[[cloud_providers]]
type = " custom-provider "
default_model = "custom-model"
"#,
    )
    .unwrap();
    rewrite_document(&mut document).unwrap();
    let providers = document["cloud_providers"].as_array().unwrap();
    assert_eq!(providers[0]["slug"].as_str(), Some("chosen"));
    assert_eq!(providers[0]["label"].as_str(), Some("Chosen Label"));
    assert_eq!(
        providers[0]["endpoint"].as_str(),
        Some("https://proxy.example/v1")
    );
    assert_eq!(providers[0]["auth_style"].as_str(), Some("bearer"));
    assert_eq!(
        providers[0]["default_model"].as_str(),
        Some("claude-sonnet-4")
    );
    assert_eq!(providers[1]["slug"].as_str(), Some("custom-provider"));
    assert_eq!(providers[1]["label"].as_str(), Some("Custom"));
    assert_eq!(providers[1]["default_model"].as_str(), Some("custom-model"));
    assert!(providers.iter().all(|p| p.get("type").is_none()));
    let once = document.clone();
    rewrite_document(&mut document).unwrap();
    assert_eq!(document, once);
}

#[test]
fn malformed_discriminator_is_not_silently_discarded() {
    let mut document: toml::Value = toml::from_str("[[cloud_providers]]\ntype = 3").unwrap();
    let error = rewrite_document(&mut document).unwrap_err().to_string();
    assert!(error.contains("cloud_providers[0].type"));
}

#[test]
fn current_schema_does_not_reapply_the_decoder_migration() {
    let mut document: toml::Value =
        toml::from_str("schema_version = 14\n[[cloud_providers]]\ntype = \"old\"").unwrap();
    let before = document.clone();
    rewrite_document(&mut document).unwrap();
    assert_eq!(document, before);
}

#[tokio::test]
async fn failed_upgrade_save_retains_version_and_model_for_retry() {
    let dir = tempfile::tempdir().unwrap();
    let blocked = dir.path().join("blocked.toml");
    std::fs::create_dir(&blocked).unwrap();
    let mut config = crate::config::Config {
        schema_version: 13,
        config_path: blocked,
        workspace_dir: dir.path().join("workspace"),
        cloud_providers: vec![crate::config::schema::cloud_providers::CloudProviderCreds {
            id: "provider".into(),
            slug: "anthropic".into(),
            default_model: Some("claude-sonnet-4".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    crate::config::migrations::run_pending(&mut config).await;
    assert_eq!(config.schema_version, 13);
    assert_eq!(
        config.cloud_providers[0].default_model.as_deref(),
        Some("claude-sonnet-4")
    );
    config.config_path = dir.path().join("retry.toml");
    crate::config::migrations::run_pending(&mut config).await;
    assert_eq!(
        config.schema_version,
        crate::config::migrations::CURRENT_SCHEMA_VERSION
    );
    let persisted: crate::config::Config =
        toml::from_str(&std::fs::read_to_string(config.config_path).unwrap()).unwrap();
    assert_eq!(
        persisted.cloud_providers[0].default_model.as_deref(),
        Some("claude-sonnet-4")
    );
}
