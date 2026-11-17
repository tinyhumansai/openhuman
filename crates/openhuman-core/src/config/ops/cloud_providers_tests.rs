use super::*;

fn update(slug: &str) -> CloudProviderUpdate {
    CloudProviderUpdate {
        id: None,
        slug: slug.to_string(),
        label: None,
        endpoint: String::new(),
        auth_style: None,
        ca_cert_pem: None,
        legacy_type: None,
        default_model: None,
    }
}

#[test]
fn builder_preserves_legacy_preset_and_blank_fallback_rules() {
    let mut entry = update("  openai-compatible ");
    entry.id = Some("  ".to_string());
    entry.label = Some("  ".to_string());
    entry.legacy_type = Some("openai".to_string());
    let providers = build_cloud_provider_creds(vec![entry]).expect("provider should build");

    assert_eq!(providers.len(), 1);
    let provider = &providers[0];
    assert!(provider.id.starts_with("p_openai-compatible_"));
    assert_eq!(provider.slug, "openai-compatible");
    assert_eq!(provider.label, "openai-compatible");
    assert_eq!(provider.endpoint, "https://api.openai.com/v1");
    assert_eq!(provider.auth_style, AuthStyle::Bearer);
}

#[test]
fn builder_preserves_nonblank_id_label_and_default_model_verbatim() {
    let mut entry = update("custom");
    entry.id = Some(" stable-id ".to_string());
    entry.label = Some(" Display label ".to_string());
    entry.endpoint = "https://provider.example/v1".to_string();
    entry.default_model = Some(" model-name ".to_string());

    let provider = build_cloud_provider_creds(vec![entry]).unwrap().remove(0);
    assert_eq!(provider.id, " stable-id ");
    assert_eq!(provider.label, " Display label ");
    assert_eq!(provider.default_model.as_deref(), Some(" model-name "));
}

#[test]
fn builder_keeps_explicit_whitespace_endpoint_and_drops_reserved_slugs() {
    let mut entry = update("custom");
    entry.endpoint = " ".to_string();
    let mut reserved = update("openhuman");
    reserved.endpoint = "https://echoed.invalid".to_string();

    let providers = build_cloud_provider_creds(vec![entry, reserved]).expect("valid update");
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].endpoint, " ");
    assert_eq!(providers[0].auth_style, AuthStyle::Bearer);
}

#[test]
fn builder_preserves_auth_aliases_and_unknown_error() {
    for (input, expected) in [
        ("ANTHROPIC", AuthStyle::Anthropic),
        ("OpenHumanJwt", AuthStyle::OpenhumanJwt),
        ("openhuman_jwt", AuthStyle::OpenhumanJwt),
        ("none", AuthStyle::None),
    ] {
        let mut entry = update("custom");
        entry.auth_style = Some(input.to_string());
        assert_eq!(
            build_cloud_provider_creds(vec![entry]).unwrap()[0].auth_style,
            expected
        );
    }

    let mut entry = update("custom");
    entry.auth_style = Some("OddAuth".to_string());
    assert_eq!(
        build_cloud_provider_creds(vec![entry]).unwrap_err(),
        "unknown auth_style 'oddauth'; valid: bearer, anthropic, openhuman_jwt, none"
    );

    let mut padded = update("custom");
    padded.auth_style = Some(" anthropic ".to_string());
    assert_eq!(
        build_cloud_provider_creds(vec![padded]).unwrap_err(),
        "unknown auth_style ' anthropic '; valid: bearer, anthropic, openhuman_jwt, none"
    );
}

#[test]
fn builder_accepts_the_legacy_type_rpc_field() {
    let value = serde_json::json!({
        "slug": "openai-compatible",
        "endpoint": "",
        "type": "openai"
    });
    let update: CloudProviderUpdate = serde_json::from_value(value).expect("legacy payload");
    let provider = build_cloud_provider_creds(vec![update]).unwrap().remove(0);
    assert_eq!(provider.endpoint, "https://api.openai.com/v1");
}

#[test]
fn builder_rejects_empty_slug() {
    assert_eq!(
        build_cloud_provider_creds(vec![update("  ")]).unwrap_err(),
        "cloud provider slug must not be empty"
    );
}
