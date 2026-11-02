use super::*;

#[tokio::test]
async fn provider_ca_cert_is_scoped_to_its_slug_and_removed_with_provider() {
    use crate::config::schema::cloud_providers::CloudProviderCreds;
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let provider = CloudProviderCreds {
        slug: "team-gateway".into(),
        endpoint: "https://gateway.example.test/v1".into(),
        ..Default::default()
    };
    let pem = include_str!("../util/tls/test-ca.pem").to_string();
    apply_model_settings(
        &mut cfg,
        ModelSettingsPatch {
            cloud_providers: Some(vec![provider]),
            cloud_provider_ca_certs: Some(std::collections::HashMap::from([(
                "team-gateway".into(),
                pem.clone(),
            )])),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(cfg.cloud_provider_ca_certs["team-gateway"], pem);
    let snapshot = crate::config::ops::loader::client_config_json(&cfg).await;
    assert_eq!(snapshot["cloud_providers"][0]["ca_cert_pem"], pem);

    apply_model_settings(
        &mut cfg,
        ModelSettingsPatch {
            cloud_provider_ca_certs: Some(std::collections::HashMap::from([(
                "team-gateway".into(),
                String::new(),
            )])),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(cfg.cloud_provider_ca_certs.is_empty());
    cfg.cloud_provider_ca_certs
        .insert("team-gateway".into(), pem);

    apply_model_settings(
        &mut cfg,
        ModelSettingsPatch {
            cloud_providers: Some(vec![]),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(cfg.cloud_provider_ca_certs.is_empty());
}
