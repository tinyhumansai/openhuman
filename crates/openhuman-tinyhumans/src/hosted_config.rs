use openhuman_embed::__host::config::schema::cloud_providers::{
    register_hosted_cloud_preset, register_hosted_cloud_provider_policy, AuthStyle,
    BuiltinCloudProvider,
};
use std::sync::Arc;

pub(crate) fn register() {
    register_hosted_cloud_preset("openhuman", openhuman_preset());
    register_hosted_cloud_provider_policy(
        Arc::new(|slug| slug == "openhuman"),
        Arc::new(|style| style == AuthStyle::OpenhumanJwt),
    );
}

pub(crate) fn openhuman_preset() -> BuiltinCloudProvider {
    BuiltinCloudProvider {
        slug: "openhuman",
        label: "OpenHuman",
        endpoint: "https://api.openhuman.ai/v1",
        auth_style: AuthStyle::OpenhumanJwt,
    }
}

pub(crate) fn provider_presets() -> tinyinference_providers::builtin::BuiltinPresets {
    let preset = openhuman_preset();
    tinyinference_providers::builtin::BuiltinPresets::new([
        tinyinference_providers::builtin::BuiltinCloudProvider {
            slug: preset.slug,
            label: preset.label,
            endpoint: preset.endpoint,
            // This catalog is used for host and protocol classification only;
            // the session JWT remains governed by TinyHumans' auth policy.
            auth_style: tinyinference_providers::builtin::BuiltinAuthStyle::Bearer,
        },
    ])
}

#[cfg(test)]
#[path = "hosted_config_tests.rs"]
mod tests;
