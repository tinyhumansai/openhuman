use super::*;

#[tokio::test]
async fn apply_model_settings_keeps_stored_provider_headers_when_the_patch_has_none() {
    // #7865: the settings panel echoes every entry back without `headers`
    // (it has no editor for them), so a table set in config.toml must survive
    // an unrelated edit; a patch that sends headers replaces them.
    use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};

    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let stored_headers: std::collections::BTreeMap<String, String> =
        [("x-tenant".to_string(), "team-a".to_string())].into();
    cfg.cloud_providers = vec![CloudProviderCreds {
        id: "gw-1".into(),
        slug: "gateway".into(),
        label: "Gateway".into(),
        endpoint: "https://gateway.example/v1".into(),
        auth_style: AuthStyle::Bearer,
        headers: stored_headers.clone(),
        default_model: None,
    }];

    let without_headers = |label: &str| CloudProviderCreds {
        id: "gw-1".into(),
        slug: "gateway".into(),
        label: label.into(),
        endpoint: "https://gateway.example/v1".into(),
        auth_style: AuthStyle::Bearer,
        ..Default::default()
    };

    let _ = apply_model_settings(
        &mut cfg,
        ModelSettingsPatch {
            cloud_providers: Some(vec![without_headers("Gateway (renamed)")]),
            ..Default::default()
        },
    )
    .await
    .expect("apply");
    let entry = &cfg.cloud_providers[0];
    assert_eq!(entry.label, "Gateway (renamed)");
    assert_eq!(
        entry.headers, stored_headers,
        "stored headers survive a save"
    );

    // Matched by slug when the id changed.
    let mut re_identified = without_headers("Gateway (new id)");
    re_identified.id = "gw-2".into();
    let _ = apply_model_settings(
        &mut cfg,
        ModelSettingsPatch {
            cloud_providers: Some(vec![re_identified]),
            ..Default::default()
        },
    )
    .await
    .expect("apply");
    assert_eq!(cfg.cloud_providers[0].headers, stored_headers);

    // An explicit map replaces the stored one.
    let replacement: std::collections::BTreeMap<String, String> =
        [("x-tenant".to_string(), "team-b".to_string())].into();
    let mut with_headers = without_headers("Gateway");
    with_headers.headers = replacement.clone();
    let _ = apply_model_settings(
        &mut cfg,
        ModelSettingsPatch {
            cloud_providers: Some(vec![with_headers]),
            ..Default::default()
        },
    )
    .await
    .expect("apply");
    assert_eq!(cfg.cloud_providers[0].headers, replacement);

    // A brand-new entry with no stored counterpart gets nothing.
    let mut fresh = without_headers("Other");
    fresh.id = "other-1".into();
    fresh.slug = "other".into();
    let _ = apply_model_settings(
        &mut cfg,
        ModelSettingsPatch {
            cloud_providers: Some(vec![fresh]),
            ..Default::default()
        },
    )
    .await
    .expect("apply");
    assert_eq!(cfg.cloud_providers.len(), 1);
    assert!(cfg.cloud_providers[0].headers.is_empty());
}
