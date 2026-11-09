//! Primary cloud-provider selection and legacy direct-inference compatibility.

use crate::config::schema::cloud_providers::CloudProviderCreds;
use crate::config::Config;
use crate::inference::provider::factory::{BYOK_INCOMPLETE_SENTINEL, PROVIDER_OPENHUMAN};

/// Keep endpoint paths out of config diagnostics while using TinyInference's
/// shared sanitizer for URL credentials, query values and invalid inputs.
fn redact_inference_origin_for_log(endpoint: &str) -> String {
    let origin = url::Url::parse(endpoint.trim()).map(|url| url.origin().ascii_serialization());
    tinyinference_core::sanitize::redact_url(origin.as_deref().unwrap_or(endpoint))
}

pub(crate) fn resolve_primary_cloud_provider_string(config: &Config) -> String {
    let primary = config
        .primary_cloud
        .as_deref()
        .and_then(|id| config.cloud_providers.iter().find(|entry| entry.id == id));

    if primary.is_some_and(|entry| is_openhuman_cloud_entry(config, entry)) {
        if let Some(legacy) = legacy_custom_inference_provider_string(config) {
            return legacy;
        }
        // Primary is explicitly OpenHuman but inference_url points at a custom
        // endpoint with no matching provider entry — this is a half-migrated BYOK
        // config. Fail closed so the user sees an actionable error rather than
        // silently routing through the managed backend.
        if has_custom_inference_intent(config) {
            log::debug!(
                "[providers][chat-factory] BYOK intent detected (endpoint={}) \
                 but no matching cloud_providers entry found; returning fail-closed sentinel",
                redact_inference_origin_for_log(
                    config.inference_url.as_deref().unwrap_or_default()
                )
            );
            return BYOK_INCOMPLETE_SENTINEL.to_string();
        }
    }

    if let Some(entry) = primary {
        return cloud_entry_provider_string(entry, config);
    }

    // No explicit primary configured. If inference_url signals custom intent but
    // no matching provider entry exists, fail closed instead of falling back to
    // the managed backend.
    legacy_custom_inference_provider_string(config).unwrap_or_else(|| {
        if has_custom_inference_intent(config) {
            log::debug!(
                "[providers][chat-factory] BYOK intent detected (endpoint={}) \
                 with no primary_cloud and no matching provider entry; returning fail-closed sentinel",
                redact_inference_origin_for_log(config.inference_url.as_deref().unwrap_or_default())
            );
            BYOK_INCOMPLETE_SENTINEL.to_string()
        } else {
            PROVIDER_OPENHUMAN.to_string()
        }
    })
}

/// Return `true` when the config contains a non-openhuman `inference_url`,
/// indicating the user intends custom/BYOK routing rather than the managed
/// backend.
fn has_custom_inference_intent(config: &Config) -> bool {
    config
        .inference_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .is_some_and(|url| !looks_like_openhuman_backend(config, url))
}

fn legacy_custom_inference_provider_string(config: &Config) -> Option<String> {
    let inference_url = config
        .inference_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())?;

    if looks_like_openhuman_backend(config, inference_url) {
        return None;
    }

    let normalized_inference = normalize_endpoint_for_compare(inference_url);
    config
        .cloud_providers
        .iter()
        .find(|entry| {
            !is_openhuman_cloud_entry(config, entry)
                && normalize_endpoint_for_compare(&entry.endpoint) == normalized_inference
        })
        .map(|entry| cloud_entry_provider_string(entry, config))
}

/// Resolve the slug of the cloud-provider entry that represents the legacy
/// direct-inference route — the entry whose endpoint matches the configured
/// custom `inference_url`.
///
/// Top-level `config.api_key` was historically paired with `inference_url`
/// for direct endpoint routing, so it is scoped to this single provider. The
/// `lookup_key_for_slug` fallback uses this to avoid leaking the global key to
/// any other provider slug whose auth-profile lookup returned empty.
pub(crate) fn legacy_inference_slug(config: &Config) -> Option<&str> {
    let inference_url = config
        .inference_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())?;

    if looks_like_openhuman_backend(config, inference_url) {
        return None;
    }

    let normalized_inference = normalize_endpoint_for_compare(inference_url);
    config
        .cloud_providers
        .iter()
        .find(|entry| {
            !is_openhuman_cloud_entry(config, entry)
                && normalize_endpoint_for_compare(&entry.endpoint) == normalized_inference
        })
        .map(|entry| entry.slug.as_str())
}

fn cloud_entry_provider_string(entry: &CloudProviderCreds, config: &Config) -> String {
    if is_openhuman_cloud_entry(config, entry) {
        return PROVIDER_OPENHUMAN.to_string();
    }

    let model = entry
        .default_model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .or_else(|| {
            config
                .default_model
                .as_deref()
                .map(str::trim)
                .filter(|model| !model.is_empty())
        });
    let Some(model) = model else {
        return BYOK_INCOMPLETE_SENTINEL.to_string();
    };

    format!("{}:{model}", entry.slug)
}

fn is_openhuman_cloud_entry(config: &Config, entry: &CloudProviderCreds) -> bool {
    crate::config::schema::cloud_providers::is_hosted_cloud_identity(&entry.slug)
        || crate::config::schema::cloud_providers::is_hosted_auth_style(entry.auth_style)
        || looks_like_openhuman_backend(config, &entry.endpoint)
}

fn normalize_endpoint_for_compare(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// Whether `url` names one of the managed hosts the installed host resolved
/// onto the config (`resolved_endpoints.managed_hosts`). With no host
/// registered nothing is managed, so every endpoint is a custom one.
fn looks_like_openhuman_backend(config: &Config, url: &str) -> bool {
    config.resolved_endpoints.is_managed_host(url)
}

#[cfg(test)]
#[path = "primary_cloud_tests.rs"]
mod tests;
