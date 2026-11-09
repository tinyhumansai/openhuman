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
