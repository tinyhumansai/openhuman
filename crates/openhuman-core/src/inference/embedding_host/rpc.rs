//! RPC handlers for the embeddings domain.

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;

mod api_keys;
mod embed;
mod settings;

pub use api_keys::{clear_api_key, set_api_key};
pub use embed::{embed, test_connection};
pub use settings::{get_settings, update_settings};

#[cfg(test)]
use tinyinference_embeddings::probe::redact_secrets;

#[cfg(test)]
use settings::remember_active_custom_profile;
#[cfg(test)]
use tinyinference_embeddings::probe::{classify_embed_probe, EmbedProbe};
#[cfg(test)]
use tinyinference_embeddings::served_models::{
    check_requested_model_served, fetch_served_model_ids, normalize_embed_model_id,
    reject_model_not_served,
};

use crate::config::Config;
use crate::security::credentials::AuthService;

use super::factory::create_embedding_provider_with_config;

const LOG_PREFIX: &str = "[embeddings::rpc]";

/// Build an embedding provider from the live config — the same construction
/// [`embed`] uses, exposed so other domains (e.g. `codegraph`) can obtain a
/// provider for `signature()` + direct embedding without a JSON-RPC round-trip.
pub fn provider_from_config(
    config: &Config,
) -> anyhow::Result<Box<dyn tinyinference_embeddings::EmbeddingModel>> {
    build_embedder(
        config,
        &config.memory.embedding_provider,
        &config.memory.embedding_model,
        config.memory.embedding_dimensions,
    )
}

/// Construct an embedding provider for an explicit `(provider_name, model,
/// dims)` triple, resolving the stored API key + inline `custom:<url>` endpoint
/// the same way [`embed`] / [`test_connection`] do. Single construction seam so
/// the save-time probe in [`update_settings`] and the live embed path can't
/// drift on slug-normalization / credential-lookup rules.
fn build_embedder(
    config: &Config,
    provider_name: &str,
    model: &str,
    dims: usize,
) -> anyhow::Result<Box<dyn tinyinference_embeddings::EmbeddingModel>> {
    let api_key = resolve_api_key(config, provider_name);
    let custom_endpoint = provider_name.strip_prefix("custom:").map(|s| s.to_string());
    let provider_slug = if provider_name.starts_with("custom:") {
        "custom"
    } else {
        provider_name
    };
    create_embedding_provider_with_config(
        config,
        provider_slug,
        model,
        dims,
        &api_key,
        custom_endpoint.as_deref(),
    )
}

pub(crate) fn resolve_api_key(config: &Config, provider_name: &str) -> String {
    let slug = if provider_name.starts_with("custom:") {
        "custom"
    } else {
        provider_name
    };
    let cred_provider = format!("embeddings:{slug}");
    let auth = AuthService::from_config(config);
    auth.get_provider_bearer_token(&cred_provider, None)
        .ok()
        .flatten()
        .unwrap_or_default()
}
