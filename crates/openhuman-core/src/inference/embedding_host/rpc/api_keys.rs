//! Storing and clearing a provider's embedding API key.

use std::collections::HashMap;

use crate::config::Config;
use crate::core::Outcome;
use crate::security::credentials::AuthService;

use super::LOG_PREFIX;

/// Stores an API key for a specific embedding provider.
pub async fn set_api_key(
    config: &Config,
    provider_slug: &str,
    api_key: &str,
) -> Result<Outcome<serde_json::Value>, String> {
    if provider_slug.is_empty() {
        return Err("provider slug is required".into());
    }
    if api_key.trim().is_empty() {
        return Err("api_key cannot be empty".into());
    }

    let cred_provider = format!("embeddings:{provider_slug}");
    let auth = AuthService::from_config(config);
    auth.store_provider_token(&cred_provider, "default", api_key, HashMap::new(), true)
        .map_err(|e| format!("failed to store embedding API key: {e}"))?;

    tracing::info!(provider = provider_slug, "{LOG_PREFIX} set_api_key stored");

    Ok(Outcome::new(
        serde_json::json!({ "stored": true, "provider": provider_slug }),
        vec![format!("embedding API key stored for {provider_slug}")],
    ))
}

/// Removes the API key for a specific embedding provider.
pub async fn clear_api_key(
    config: &Config,
    provider_slug: &str,
) -> Result<Outcome<serde_json::Value>, String> {
    if provider_slug.is_empty() {
        return Err("provider slug is required".into());
    }

    let cred_provider = format!("embeddings:{provider_slug}");
    let auth = AuthService::from_config(config);
    let removed = auth
        .remove_profile(&cred_provider, "default")
        .map_err(|e| format!("failed to clear embedding API key: {e}"))?;

    tracing::info!(
        provider = provider_slug,
        removed,
        "{LOG_PREFIX} clear_api_key"
    );

    Ok(Outcome::new(
        serde_json::json!({ "cleared": removed, "provider": provider_slug }),
        vec![format!("embedding API key cleared for {provider_slug}")],
    ))
}
