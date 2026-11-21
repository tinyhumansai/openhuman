use openhuman_embed::__host::config::schema::cloud_providers::{
    AuthStyle, BuiltinCloudProvider, register_hosted_cloud_preset,
    register_hosted_cloud_provider_policy,
};
use std::sync::Arc;

pub(crate) fn register() {
    register_hosted_cloud_preset("openhuman", openhuman_preset());
    register_hosted_cloud_preset("llmtr", llmtr_preset());
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

pub(crate) fn llmtr_preset() -> BuiltinCloudProvider {
    BuiltinCloudProvider {
        slug: "llmtr",
        label: "LLMTR",
        endpoint: "https://llmtr.com/v1",
        auth_style: AuthStyle::Bearer,
    }
}

pub(crate) fn provider_presets() -> tinyinference_providers::builtin::BuiltinPresets {
    let openhuman = openhuman_preset();
    let llmtr = llmtr_preset();
    tinyinference_providers::builtin::BuiltinPresets::new([
        tinyinference_providers::builtin::BuiltinCloudProvider {
            slug: openhuman.slug,
            label: openhuman.label,
            endpoint: openhuman.endpoint,
            // This catalog is used for host and protocol classification only;
            // the session JWT remains governed by TinyHumans' auth policy.
            auth_style: tinyinference_providers::builtin::BuiltinAuthStyle::Bearer,
        },
        tinyinference_providers::builtin::BuiltinCloudProvider {
            slug: llmtr.slug,
            label: llmtr.label,
            endpoint: llmtr.endpoint,
            auth_style: tinyinference_providers::builtin::BuiltinAuthStyle::Bearer,
        },
    ])
}

#[cfg(test)]
#[path = "hosted_config_tests.rs"]
mod tests;
