use super::*;

/// A bare-core first install persists the generic defaults and remains stable
/// when reopened. Hosted provider seeding is covered by TinyHumans' registered
/// normalizer tests, not by core's config creation path.
#[tokio::test]
async fn a_brand_new_workspace_persists_and_reopens_generic_config() {
    let tmp = tempfile::tempdir().unwrap();
    crate::inference::registry::register_config_normalizer(
        "fresh-install-test",
        std::sync::Arc::new(|config| {
            if !config.cloud_providers.is_empty() {
                return false;
            }
            config
                .cloud_providers
                .push(crate::config::schema::CloudProviderCreds {
                    id: "fixture-provider-id".to_string(),
                    slug: "fixture-provider".to_string(),
                    label: "Fixture Provider".to_string(),
                    endpoint: "https://fixture.example/v1".to_string(),
                    auth_style: crate::config::schema::AuthStyle::Bearer,
                    default_model: None,
                });
            true
        }),
    );
    let first = load_or_init_for_workspace(tmp.path()).await;
    assert!(first.default_model.is_none());
    assert_eq!(first.cloud_providers.len(), 1);
    assert_eq!(first.cloud_providers[0].slug, "fixture-provider");

    let first_text = tokio::fs::read_to_string(&first.config_path).await.unwrap();
    assert!(first_text.contains("schema_version"));
    assert!(first_text.contains("slug = \"fixture-provider\""));

    let second = load_or_init_for_workspace(tmp.path()).await;
    assert!(second.default_model.is_none());
    assert_eq!(second.cloud_providers.len(), 1);
    assert_eq!(second.cloud_providers[0].id, "fixture-provider-id");
}

#[tokio::test]
async fn bare_core_advances_schema_without_inventing_hosted_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = load_or_init_for_workspace(tmp.path()).await;
    assert!(config.default_model.is_none());
    assert!(config
        .cloud_providers
        .iter()
        .all(|provider| provider.slug != "openhuman"));

    config.schema_version = 7;
    config.default_model = Some("reasoning-v1".to_string());
    crate::config::migrations::run_pending(&mut config).await;
    assert_eq!(
        config.schema_version,
        crate::config::migrations::CURRENT_SCHEMA_VERSION
    );
    assert_eq!(config.default_model.as_deref(), Some("reasoning-v1"));
}
