//! RPC input conversion for cloud-provider settings.

use crate::config::schema::cloud_providers::{
    builtin_cloud_provider, generate_provider_id, is_slug_reserved, AuthStyle, CloudProviderCreds,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CloudProviderUpdate {
    pub(crate) id: Option<String>,
    pub(crate) slug: String,
    #[serde(default)]
    pub(crate) label: Option<String>,
    pub(crate) endpoint: String,
    #[serde(default)]
    pub(crate) auth_style: Option<String>,
    #[serde(default)]
    pub(crate) ca_cert_pem: Option<String>,
    // keep until v0.66 (review after 2027-01-11): incoming RPC discriminator
    // remains accepted until shipped-client parameter coverage exists.
    #[serde(rename = "type", default)]
    pub(crate) legacy_type: Option<String>,
    #[serde(default)]
    pub(crate) default_model: Option<String>,
}

/// Convert cloud-provider settings RPC inputs into validated config entries.
///
/// Reserved built-in slugs are dropped here because the config update operation
/// re-injects their stored entries after replacing the user-provided list.
pub fn build_cloud_provider_creds(
    entries: Vec<CloudProviderUpdate>,
) -> Result<Vec<CloudProviderCreds>, String> {
    let reserved_count = entries
        .iter()
        .filter(|entry| {
            let slug = entry.slug.trim();
            !slug.is_empty() && is_slug_reserved(slug)
        })
        .count();
    if reserved_count > 0 {
        log::debug!(
            "[inference] update_model_settings: dropping {} reserved cloud provider slug(s)",
            reserved_count
        );
    }

    entries
        .into_iter()
        // Silently drop entries whose (non-empty) slug is reserved —
        // typically migration-seeded built-ins that the frontend echoes back.
        // Empty slugs still fall through so validation reports frontend bugs.
        .filter(|entry| {
            let slug = entry.slug.trim();
            slug.is_empty() || !is_slug_reserved(slug)
        })
        .map(|entry| {
            let slug = entry.slug.trim().to_string();
            if slug.is_empty() {
                return Err("cloud provider slug must not be empty".to_string());
            }

            let legacy_provider = entry
                .legacy_type
                .as_deref()
                .map(str::trim)
                .and_then(builtin_cloud_provider);
            let legacy_auth = legacy_provider
                .map(|provider| provider.auth_style.as_str())
                .unwrap_or("bearer");
            let endpoint = if entry.endpoint.is_empty() {
                legacy_provider
                    .map(|provider| provider.endpoint.to_string())
                    .unwrap_or(entry.endpoint)
            } else {
                entry.endpoint
            };
            let auth_style = AuthStyle::parse(entry.auth_style.as_deref().unwrap_or(legacy_auth))?;
            let id = entry
                .id
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| generate_provider_id(&slug));
            let label = entry
                .label
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| slug.clone());

            Ok(CloudProviderCreds {
                id,
                slug,
                label,
                endpoint,
                auth_style,
                default_model: entry.default_model,
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "cloud_providers_tests.rs"]
mod tests;
