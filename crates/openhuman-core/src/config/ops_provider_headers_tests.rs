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

#[test]
fn client_config_snapshot_shows_header_names_but_not_values() {
    use crate::config::schema::cloud_providers::{
        AuthStyle, CloudProviderCreds, REDACTED_HEADER_VALUE,
    };

    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    cfg.cloud_providers = vec![CloudProviderCreds {
        id: "gw-1".into(),
        slug: "gateway".into(),
        label: "Gateway".into(),
        endpoint: "https://gateway.example/v1".into(),
        auth_style: AuthStyle::Bearer,
        headers: [("x-api-secret".to_string(), "sk-live-123".to_string())].into(),
        default_model: None,
    }];

    let snapshot = crate::config::ops::client_config_json(&cfg);
    let rendered = snapshot.to_string();
    assert!(!rendered.contains("sk-live-123"), "{rendered}");
    let entry = snapshot["cloud_providers"]
        .as_array()
        .and_then(|entries| entries.iter().find(|e| e["slug"] == "gateway"))
        .expect("gateway entry in the snapshot");
    assert_eq!(entry["headers"]["x-api-secret"], REDACTED_HEADER_VALUE);
}

#[tokio::test]
async fn apply_model_settings_restores_stored_values_for_echoed_placeholders() {
    use crate::config::schema::cloud_providers::{
        AuthStyle, CloudProviderCreds, REDACTED_HEADER_VALUE,
    };

    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    cfg.cloud_providers = vec![CloudProviderCreds {
        id: "gw-1".into(),
        slug: "gateway".into(),
        label: "Gateway".into(),
        endpoint: "https://gateway.example/v1".into(),
        auth_style: AuthStyle::Bearer,
        headers: [("x-api-secret".to_string(), "sk-live-123".to_string())].into(),
        default_model: None,
    }];

    // A client that echoes the redacted snapshot back, adding one header and
    // inventing a placeholder for a header that was never stored.
    let echoed = CloudProviderCreds {
        id: "gw-1".into(),
        slug: "gateway".into(),
        label: "Gateway".into(),
        endpoint: "https://gateway.example/v1".into(),
        auth_style: AuthStyle::Bearer,
        headers: [
            (
                "x-api-secret".to_string(),
                REDACTED_HEADER_VALUE.to_string(),
            ),
            ("x-tenant".to_string(), "team-a".to_string()),
            ("x-unknown".to_string(), REDACTED_HEADER_VALUE.to_string()),
        ]
        .into(),
        default_model: None,
    };
    let _ = apply_model_settings(
        &mut cfg,
        ModelSettingsPatch {
            cloud_providers: Some(vec![echoed]),
            ..Default::default()
        },
    )
    .await
    .expect("apply");

    let headers = &cfg.cloud_providers[0].headers;
    assert_eq!(
        headers.get("x-api-secret").map(String::as_str),
        Some("sk-live-123")
    );
    assert_eq!(headers.get("x-tenant").map(String::as_str), Some("team-a"));
    assert!(
        !headers.contains_key("x-unknown"),
        "a placeholder with nothing stored is dropped, never sent"
    );
}
