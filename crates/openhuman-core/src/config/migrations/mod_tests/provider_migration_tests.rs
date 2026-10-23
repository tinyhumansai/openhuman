use super::*;

/// The re-seed must not touch a workspace that already has providers.
///
/// `seed_cloud_providers` early-returns on a non-empty list, so this is the
/// guard that keeps the new step a no-op for every healthy install — including
/// one whose only entry is a BYO provider the user configured themselves.
#[tokio::test]
async fn a_v10_workspace_that_already_has_providers_is_left_alone() {
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

/// picker's managed pane renders "Could not load models from this provider."
#[tokio::test]
async fn a_v10_workspace_with_no_cloud_providers_is_reseeded() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();

    let mut config = config_in(&tmp);
    config.schema_version = 10;
    config.cloud_providers = Vec::new();

    run_pending(&mut config).await;

    assert!(
        config.cloud_providers.iter().any(|e| e.slug == "openhuman"),
        "a v10 workspace with an empty provider list must gain the managed entry, \
         otherwise the model picker can never load the managed catalog"
    );
    assert_eq!(
        config.schema_version, CURRENT_SCHEMA_VERSION,
        "the re-seed step must advance the schema version"
    );
}
