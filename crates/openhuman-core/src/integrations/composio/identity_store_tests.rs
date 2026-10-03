use super::*;

fn config_in(dir: &tempfile::TempDir) -> Config {
    Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    }
}

fn slack_profile(connection: &str) -> ProviderUserProfile {
    ProviderUserProfile {
        toolkit: "Slack".into(),
        connection_id: Some(connection.into()),
        display_name: Some("Ada Lovelace".into()),
        email: Some("Ada@Example.com".into()),
        username: Some("U123ABC".into()),
        extras: serde_json::json!({ "handle": "@ada" }),
        ..ProviderUserProfile::default()
    }
}

#[tokio::test]
async fn persist_then_load_groups_fields_per_connection() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(&dir);

    let written = persist_provider_profile(&config, &slack_profile("conn-1"))
        .await
        .unwrap();
    assert_eq!(written, 4);

    let identities = load_connected_identities(&config).await.unwrap();
    assert_eq!(identities.len(), 1);
    let identity = &identities[0];
    assert_eq!(identity.source, "slack");
    assert_eq!(identity.display_name.as_deref(), Some("Ada Lovelace"));
    assert_eq!(identity.user_id.as_deref(), Some("U123ABC"));
    assert!(identity.email.is_some());
    assert!(identity.handle.is_some());
    assert!(file_store::path(&config, IDENTITIES_FILE).exists());
}

#[tokio::test]
async fn persist_merges_into_the_stored_identity() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(&dir);
    persist_provider_profile(&config, &slack_profile("conn-1"))
        .await
        .unwrap();

    let partial = ProviderUserProfile {
        toolkit: "slack".into(),
        connection_id: Some("conn-1".into()),
        display_name: Some("Ada L.".into()),
        ..ProviderUserProfile::default()
    };
    persist_provider_profile(&config, &partial).await.unwrap();

    let identities = load_connected_identities(&config).await.unwrap();
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].display_name.as_deref(), Some("Ada L."));
    assert_eq!(identities[0].user_id.as_deref(), Some("U123ABC"));
}

#[tokio::test]
async fn empty_profile_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(&dir);
    let empty = ProviderUserProfile {
        toolkit: "gmail".into(),
        ..ProviderUserProfile::default()
    };
    assert_eq!(persist_provider_profile(&config, &empty).await.unwrap(), 0);
    assert!(!file_store::path(&config, IDENTITIES_FILE).exists());
    assert!(load_connected_identities(&config).await.unwrap().is_empty());
}

#[tokio::test]
async fn delete_removes_only_the_named_connection() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(&dir);
    persist_provider_profile(&config, &slack_profile("conn-1"))
        .await
        .unwrap();
    persist_provider_profile(&config, &slack_profile("conn-2"))
        .await
        .unwrap();

    let deleted = delete_connected_identity_facets(&config, "SLACK", "conn-1")
        .await
        .unwrap();
    assert_eq!(deleted, 4);

    let identities = load_connected_identities(&config).await.unwrap();
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].identifier, "conn-2");

    assert_eq!(
        delete_connected_identity_facets(&config, "slack", "conn-1")
            .await
            .unwrap(),
        0
    );
}
