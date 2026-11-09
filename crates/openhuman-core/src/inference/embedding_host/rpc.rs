//! RPC handlers for the embeddings domain.

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;

mod api_keys;
mod embed;
mod settings;

pub use api_keys::{clear_api_key, set_api_key};
pub use embed::{embed, test_connection};
pub(crate) use embed::{probe_dimensions_for_inspection, InspectionEmbeddingError};
pub use settings::{get_settings, update_settings};

#[cfg(test)]
use settings::remember_active_custom_profile;

use crate::config::Config;
use crate::credentials::AuthService;
use tinyinference_providers::route::split_custom_provider;

use super::factory::create_embedding_provider_with_config;

const LOG_PREFIX: &str = "[embeddings::rpc]";

/// Build an embedding provider from the live config — the same construction
/// [`embed`] uses, exposed so other domains (e.g. `codegraph`) can obtain a
/// provider for `signature()` + direct embedding without a JSON-RPC round-trip.
pub async fn provider_from_config(
    config: &Config,
) -> anyhow::Result<Box<dyn tinyinference_embeddings::EmbeddingModel>> {
    build_embedder(
        config,
        &config.memory.embedding_provider,
        &config.memory.embedding_model,
        config.memory.embedding_dimensions,
    )
    .await
}

/// Construct an embedding provider for an explicit `(provider_name, model,
/// dims)` triple, resolving the stored API key + inline `custom:<url>` endpoint
/// the same way [`embed`] / [`test_connection`] do. Single construction seam so
/// the save-time probe in [`update_settings`] and the live embed path can't
/// drift on slug-normalization / credential-lookup rules.
async fn build_embedder(
    config: &Config,
    provider_name: &str,
    model: &str,
    dims: usize,
) -> anyhow::Result<Box<dyn tinyinference_embeddings::EmbeddingModel>> {
    let api_key = resolve_api_key(config, provider_name).await;
    let (provider_slug, custom_endpoint) = split_custom_provider(provider_name);
    let provider_slug = if custom_endpoint.is_some() {
        "custom"
    } else {
        &provider_slug
    };
    create_embedding_provider_with_config(
        config,
        provider_slug,
        model,
        dims,
        &api_key,
        custom_endpoint.as_deref(),
    )
    .await
}

pub(crate) async fn resolve_api_key(config: &Config, provider_name: &str) -> String {
    let (slug, endpoint) = split_custom_provider(provider_name);
    let slug = if endpoint.is_some() { "custom" } else { &slug };
    let cred_provider = format!("embeddings:{slug}");
    let auth = AuthService::from_config(config);
    auth.get_provider_bearer_token(&cred_provider, None)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

pub(crate) async fn resolve_api_key_read_only(
    config: &Config,
    provider_name: &str,
) -> Result<String, crate::credentials::profiles::ReadOnlyAuthProfileError> {
    let slug = if provider_name.starts_with("custom:") {
        "custom"
    } else {
        provider_name
    };
    let cred_provider = format!("embeddings:{slug}");
    Ok(
        AuthService::get_provider_bearer_token_read_only(config, &cred_provider, None)
            .await?
            .unwrap_or_default(),
    )
}
