use super::*;

#[tokio::test]
async fn unmanaged_openhuman_jwt_route_never_builds_a_direct_unauthenticated_model() {
    let _guard = crate::inference::inference_test_guard();
    let _env = crate::config::TEST_ENV_LOCK.blocking_lock();
    let tmp = tempfile::TempDir::new().expect("isolated workspace");
    let extensions = crate::core::slots::Extensions::new();
    crate::config::schema::cloud_providers::register_hosted_cloud_provider_policy_in(
        &extensions,
        Arc::new(|_| false),
        Arc::new(|_| false),
    );
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_managed".to_string(),
        slug: "managed".to_string(),
        label: "Managed".to_string(),
        endpoint: "https://api.example/v1".to_string(),
        auth_style: AuthStyle::OpenhumanJwt,
        ..Default::default()
    });

    let error = super::super::cloud_slug::try_create_cloud_slug_chat_model_from_string_with_native_tools_for(
        "chat",
        "managed:model",
        &config,
        true,
        &EgressLedger::none(),
        Some(&extensions),
    )
    .await
    .expect("configured slug");
    let error = match error {
        Err(error) => error,
        Ok(_) => panic!("unmanaged JWT route must not build an unauthenticated model"),
    };

    assert!(
        error
            .to_string()
            .contains("hosted cloud authentication is unavailable"),
        "unexpected error: {error}"
    );
}
