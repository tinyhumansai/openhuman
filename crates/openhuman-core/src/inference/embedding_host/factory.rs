//! OpenHuman bindings for TinyInference's embedding factory.

use std::sync::Arc;

use crate::config::Config;
use tinyinference_embeddings::factory::{
    create_configured_embedding_model, create_default_embedding_model, CredentialResolver,
    EmbeddingFactorySettings, ManagedModelFactory,
};
use tinyinference_embeddings::EmbeddingModel;

use super::cloud::{credential_scope, UnavailableEmbeddingModel};
use crate::inference::provider::factory::PROVIDER_OPENHUMAN;
use crate::inference::registry::EmbeddingRequest;
use crate::security::approval::ApprovalChatContext;

fn settings(
    provider: &str,
    model: &str,
    dimensions: usize,
    ollama_base_url: String,
) -> EmbeddingFactorySettings {
    EmbeddingFactorySettings {
        provider: provider.to_owned(),
        model: model.to_owned(),
        dimensions,
        ollama_base_url,
    }
}

fn managed_factory(config: &Config, chat: Option<ApprovalChatContext>) -> ManagedModelFactory {
    let config = config.clone();
    Arc::new(move |model, dimensions| {
        if let Some(registered) = crate::inference::registry::embedding(PROVIDER_OPENHUMAN) {
            log::debug!("[embeddings::factory] managed model served by a registered factory");
            let (state_dir, secrets_encrypt) = credential_scope(Some(&config));
            return registered(&EmbeddingRequest {
                config: Some(&config),
                model,
                dimensions,
                chat: chat.clone(),
                state_dir,
                secrets_encrypt,
            });
        }
        log::debug!("[embeddings::factory] no managed embedding provider registered");
        Ok(Box::new(UnavailableEmbeddingModel::new(model, dimensions)))
    })
}

fn credential_resolver(config: &Config) -> CredentialResolver {
    let config = config.clone();
    Arc::new(move |provider| super::rpc::resolve_api_key(&config, provider))
}

/// Creates a provider from explicit settings and host credentials.
pub fn create_embedding_provider_with_credentials(
    provider: &str,
    model: &str,
    dimensions: usize,
    api_key: &str,
    custom_endpoint: Option<&str>,
) -> anyhow::Result<Box<dyn EmbeddingModel>> {
    let provider = custom_endpoint
        .filter(|_| provider == "custom")
        .map(|endpoint| format!("custom:{endpoint}"))
        .unwrap_or_else(|| provider.to_owned());
    let api_key = api_key.to_owned();
    let credentials: CredentialResolver = Arc::new(move |_| api_key.clone());
    let managed: ManagedModelFactory = Arc::new(|model, dimensions| {
        if let Some(registered) = crate::inference::registry::embedding(PROVIDER_OPENHUMAN) {
            log::debug!("[embeddings::factory] managed model served by a registered factory");
            let (state_dir, secrets_encrypt) = credential_scope(None);
            return registered(&EmbeddingRequest {
                config: None,
                model,
                dimensions,
                chat: None,
                state_dir,
                secrets_encrypt,
            });
        }
        log::debug!("[embeddings::factory] no managed embedding provider registered");
        Ok(Box::new(UnavailableEmbeddingModel::new(model, dimensions)))
    });
    let model = create_configured_embedding_model(
        &settings(
            &provider,
            model,
            dimensions,
            tinyinference_local::ollama::ollama_base_url(),
        ),
        &credentials,
        &managed,
    )?;
    Ok(model)
}

/// Creates a provider using OpenHuman's credential and endpoint configuration.
/// Its cloud disclosures are unrouted; see [`create_embedding_provider_for_chat`].
pub fn create_embedding_provider_with_config(
    config: &Config,
    provider: &str,
    model: &str,
    dimensions: usize,
    api_key: &str,
    custom_endpoint: Option<&str>,
) -> anyhow::Result<Box<dyn EmbeddingModel>> {
    create_embedding_provider_for_chat(
        config,
        provider,
        model,
        dimensions,
        api_key,
        custom_endpoint,
        None,
    )
}

/// [`create_embedding_provider_with_config`] for a call made in a chat turn:
/// the managed (cloud) model discloses its egress to `chat`, the turn's thread
/// and client, so the disclosure card reaches the chat that triggered it.
pub fn create_embedding_provider_for_chat(
    config: &Config,
    provider: &str,
    model: &str,
    dimensions: usize,
    api_key: &str,
    custom_endpoint: Option<&str>,
    chat: Option<ApprovalChatContext>,
) -> anyhow::Result<Box<dyn EmbeddingModel>> {
    let provider = custom_endpoint
        .filter(|_| provider == "custom")
        .map(|endpoint| format!("custom:{endpoint}"))
        .unwrap_or_else(|| provider.to_owned());
    let selected = settings(
        &provider,
        model,
        dimensions,
        tinyinference_local::ollama::ollama_base_url_from_override(
            config.local_ai.base_url.as_deref(),
        ),
    );
    let api_key = api_key.to_owned();
    let credentials: CredentialResolver = Arc::new(move |_| api_key.clone());
    let model =
        create_configured_embedding_model(&selected, &credentials, &managed_factory(config, chat))?;
    Ok(model)
}

/// Returns the configured provider, falling back to managed embeddings. Its
/// cloud disclosures are unrouted; see [`default_embedding_provider_for_chat`].
pub fn default_embedding_provider_with_config(config: &Config) -> Arc<dyn EmbeddingModel> {
    default_embedding_provider_for_chat(config, None)
}

/// [`default_embedding_provider_with_config`] for a call made in a chat turn:
/// the managed (cloud) model discloses its egress to `chat`.
pub fn default_embedding_provider_for_chat(
    config: &Config,
    chat: Option<ApprovalChatContext>,
) -> Arc<dyn EmbeddingModel> {
    let selected = settings(
        &config.memory.embedding_provider,
        &config.memory.embedding_model,
        config.memory.embedding_dimensions,
        tinyinference_local::ollama::ollama_base_url_from_override(
            config.local_ai.base_url.as_deref(),
        ),
    );
    let model = create_default_embedding_model(
        &selected,
        &credential_resolver(config),
        &managed_factory(config, chat),
    )
    .unwrap_or_else(|error| {
        log::warn!("[embeddings::factory] managed fallback failed to build: {error}");
        Box::new(tinyinference_embeddings::NoopEmbeddingModel)
    });
    Arc::from(model)
}

#[cfg(test)]
#[path = "factory_tests.rs"]
mod tests;
