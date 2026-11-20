use super::*;

#[test]
fn hosted_openhuman_preset_and_legacy_auth_policy_are_registered_before_config_load() {
    register();

    let preset = super::openhuman_preset();
    assert_eq!(preset.endpoint, "https://api.openhuman.ai/v1");
    assert_eq!(preset.auth_style, AuthStyle::OpenhumanJwt);
    assert!(super::provider_presets().endpoint_host_is_chat_completions_only("api.openhuman.ai"));
    assert!(
        openhuman_embed::__host::config::schema::cloud_providers::is_hosted_cloud_identity(
            "openhuman"
        )
    );
    assert!(super::provider_presets().endpoint_host_is_chat_completions_only(preset.endpoint));

    assert!(
        !openhuman_embed::__host::config::schema::cloud_providers::is_hosted_auth_style(
            AuthStyle::Bearer
        )
    );
    assert!(
        !openhuman_embed::__host::config::schema::cloud_providers::is_hosted_cloud_identity(
            "legacy-custom-slug"
        )
    );
    assert!(
        openhuman_embed::__host::config::schema::cloud_providers::is_hosted_auth_style(
            AuthStyle::OpenhumanJwt
        )
    );
}

#[test]
fn hosted_llmtr_preset_is_registered_as_a_bearer_byok_provider() {
    register();

    let preset = super::llmtr_preset();
    assert_eq!(preset.slug, "llmtr");
    assert_eq!(preset.label, "LLMTR");
    assert_eq!(preset.endpoint, "https://llmtr.com/v1");
    assert_eq!(preset.auth_style, AuthStyle::Bearer);
    assert!(super::provider_presets().endpoint_host_is_chat_completions_only("llmtr.com"));

    let entry =
        openhuman_embed::__host::config::schema::cloud_providers::builtin_cloud_provider("llmtr")
            .expect("llmtr hosted preset");
    assert_eq!(entry.endpoint, "https://llmtr.com/v1");
    assert_eq!(entry.auth_style, AuthStyle::Bearer);
    assert!(
        !openhuman_embed::__host::config::schema::cloud_providers::is_hosted_cloud_identity(
            "llmtr"
        )
    );
}
