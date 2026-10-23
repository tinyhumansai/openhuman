use super::*;

/// Generic schema progression preserves a user-owned provider entry.
#[tokio::test]
async fn bare_core_v10_workspace_preserves_user_cloud_providers() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();

    let mut config = config_in(&tmp);
    config.schema_version = 10;
    config.cloud_providers = vec![crate::config::schema::cloud_providers::CloudProviderCreds {
        id: "prov_user_owned".to_string(),
        slug: "custom".to_string(),
        label: "My own endpoint".to_string(),
        endpoint: "https://llm.example.com/v1".to_string(),
        ..Default::default()
    }];

    run_pending(&mut config).await;

    assert_eq!(
        config.cloud_providers.len(),
        1,
        "an existing provider list must not be added to"
    );
    assert_eq!(config.cloud_providers[0].id, "prov_user_owned");
}

/// The user-visible consequence is that `inference_list_models("openhuman")`
/// fails its provider lookup before making any HTTP request, so the model
/// picker's managed pane renders "Could not load models from this provider."
#[tokio::test]
async fn bare_core_v10_workspace_advances_schema_without_hosted_policy() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();

    let mut config = config_in(&tmp);
    config.schema_version = 10;
    config.cloud_providers = Vec::new();

    run_pending(&mut config).await;

    assert!(config.cloud_providers.is_empty());
    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
}
