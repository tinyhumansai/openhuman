//! Core-owned endpoint checks for model-settings persistence.
/// The renderer's masked credential inventory is not an authority for egress.

use crate::config::schema::cloud_providers::AuthStyle;
use crate::config::Config;
use crate::security::credentials::AuthService;
use crate::security::transport::{
    is_safe_endpoint_for_bearer, is_valid_provider_endpoint,
};

pub(super) fn validate_keyed_cloud_destinations(config: &Config) -> Result<(), String> {
    // A direct inference pair may not have a cloud entry until default_model
    // exists. Validate independently so that omitting it cannot bypass egress.
    if let Some(url) = config.inference_url.as_deref().filter(|url| !url.trim().is_empty()) {
        if !is_valid_provider_endpoint(url) {
            return Err("Invalid provider endpoint URL".to_owned());
        }
        if config.api_key.as_deref().is_some_and(|key| !key.trim().is_empty())
            && !is_safe_endpoint_for_bearer(url)
        {
            return Err("Credentialed cloud providers require HTTPS or loopback HTTP".to_owned());
        }
    }

    for entry in &config.cloud_providers {
        if matches!(entry.auth_style, AuthStyle::OpenhumanJwt)
            || entry.endpoint.trim().is_empty()
        {
            continue;
        }
        // Reject malformed URLs even for unkeyed endpoints; callers outside
        // React must not bypass the form's strict URL checks.
        if !is_valid_provider_endpoint(&entry.endpoint) {
            return Err("Invalid provider endpoint URL".to_owned());
        }
        if !matches!(entry.auth_style, AuthStyle::Bearer | AuthStyle::Anthropic)
            || is_safe_endpoint_for_bearer(&entry.endpoint)
        {
            continue;
        }

        // The stored credential state, not a cached/failed auth-list display,
        // determines whether an endpoint may be persisted as remote HTTP.
        let has_stored = |provider: &str| -> Result<bool, String> {
            AuthService::get_provider_bearer_token_read_only(config, provider, None)
                .map(|value| value.is_some())
                .map_err(|_| "could not verify stored credentials for an insecure endpoint".into())
        };
        let profile_key = format!("provider:{}", entry.slug);
        let keyed = has_stored(&profile_key)?
            || has_stored(&entry.slug)?
            || (super::primary_cloud::legacy_inference_slug(config)
                == Some(entry.slug.as_str())
                && config.api_key.as_deref().is_some_and(|key| !key.trim().is_empty()));
        if keyed {
            return Err("Credentialed cloud providers require HTTPS or loopback HTTP".to_owned());
        }
    }
    Ok(())
}
