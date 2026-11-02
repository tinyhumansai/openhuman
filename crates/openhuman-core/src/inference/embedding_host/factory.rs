//! OpenHuman bindings for TinyInference's embedding factory.

use std::sync::Arc;

use crate::config::Config;
use tinyinference_embeddings::factory::{
    create_configured_embedding_model, create_default_embedding_model, CredentialResolver,
    EmbeddingFactorySettings, ManagedModelFactory,
};
use tinyinference_embeddings::{EmbeddingModel, UnavailableEmbeddingModel};
use tinyinference_providers::route::custom_provider_string;

use super::embedding_scope::credential_scope;
use crate::inference::provider::factory::PROVIDER_OPENHUMAN;
use crate::inference::registry::ProviderContext;
use crate::security::approval::ApprovalChatContext;
use tinyinference_llm::providers::overrides::EmbeddingRequest;

pub(super) fn settings(
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

pub(super) fn managed_factory(
    config: Option<Config>,
    chat: Option<ApprovalChatContext>,
) -> ManagedModelFactory {
    Arc::new(move |model, dimensions| {
        let model = model.to_owned();
        let config = config.clone();
        let chat = chat.clone();
        Box::pin(async move {
            if let Some(registered) = crate::inference::registry::ambient_provider_overrides()
                .embedding_factory(PROVIDER_OPENHUMAN)
            {
                log::debug!("[embeddings::factory] managed model served by a registered factory");
                let (state_dir, secrets_encrypt) = credential_scope(config.as_ref());
                let context = ProviderContext::for_embedding(
                    config.as_ref(),
                    chat.clone(),
                    state_dir,
                    secrets_encrypt,
                );
                return registered(EmbeddingRequest {
                    model: model.clone(),
                    dimensions,
                    context,
                })
                .await;
            }
            log::debug!("[embeddings::factory] no managed embedding provider registered");
            let message = format!(
                "{} managed embeddings need a hosted provider; none is registered",
                crate::core::observability::BACKEND_UNAVAILABLE_PREFIX
            );
            Ok(
                Box::new(UnavailableEmbeddingModel::new(model, dimensions, message))
                    as Box<dyn EmbeddingModel>,
            )
        })
    })
}

fn inspection_managed_factory(config: &Config) -> ManagedModelFactory {
    let config = config.clone();
    Arc::new(move |model, dimensions| {
        let model = model.to_owned();
        let config = config.clone();
        Box::pin(async move {
            let Some(registered) =
                crate::inference::registry::inspection_embedding(PROVIDER_OPENHUMAN)
            else {
                log::debug!("[embeddings::factory] no inspection-safe managed embedder registered");
                return Ok(Box::new(UnavailableEmbeddingModel::new(
                    model,
                    dimensions,
                    format!(
                        "{} managed embeddings need an inspection-safe hosted provider; none is registered",
                        crate::core::observability::BACKEND_UNAVAILABLE_PREFIX
                    ),
                )) as Box<dyn EmbeddingModel>);
            };
            let (state_dir, secrets_encrypt) = credential_scope(Some(&config));
            let context =
                ProviderContext::for_embedding(Some(&config), None, state_dir, secrets_encrypt);
            registered(EmbeddingRequest {
                model: model.clone(),
                dimensions,
                context,
            })
            .await
            .map_err(|_| {
                tinyinference_embeddings::Error::Validation(
                    "managed embedding inspection is unavailable".to_owned(),
                )
            })
        })
    })
}

pub(super) fn credential_resolver(api_key: String) -> CredentialResolver {
    Arc::new(move |_| api_key.clone())
}

/// Creates a provider from explicit settings and host credentials.
pub async fn create_embedding_provider_with_credentials(
    provider: &str,
    model: &str,
    dimensions: usize,
    api_key: &str,
    custom_endpoint: Option<&str>,
) -> anyhow::Result<Box<dyn EmbeddingModel>> {
    let provider = custom_endpoint
        .filter(|_| provider == "custom")
        .map(|endpoint| custom_provider_string("custom", Some(endpoint)))
        .unwrap_or_else(|| provider.to_owned());
    let api_key = api_key.to_owned();
    let credentials: CredentialResolver = Arc::new(move |_| api_key.clone());
    let model = create_configured_embedding_model(
        &settings(
            &provider,
            model,
            dimensions,
            tinyinference_local::ollama::ollama_base_url(),
        ),
        &credentials,
        &managed_factory(None, None),
    )
    .await?;
    Ok(model)
}

/// Creates a provider using OpenHuman's credential and endpoint configuration.
/// Its cloud disclosures are unrouted; see [`create_embedding_provider_for_chat`].
pub async fn create_embedding_provider_with_config(
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
    .await
}

/// Builds the selected provider through the shared TinyInference factory,
/// using only the supplied read-only credential snapshot for BYOK providers.
/// Managed embeddings use a separate inspection-safe factory and never fall
/// back to the ordinary mutable managed factory.
pub async fn create_embedding_provider_for_inspection(
    config: &Config,
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
    let model = create_configured_embedding_model(
        &selected,
        &credentials,
        &inspection_managed_factory(config),
    )
    .await?;
    Ok(model)
}

/// [`create_embedding_provider_with_config`] for a call made in a chat turn:
/// the managed (cloud) model discloses its egress to `chat`, the turn's thread
/// and client, so the disclosure card reaches the chat that triggered it.
pub async fn create_embedding_provider_for_chat(
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
        .map(|endpoint| custom_provider_string("custom", Some(endpoint)))
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
    let model = create_configured_embedding_model(
        &selected,
        &credentials,
        &managed_factory(Some(config.clone()), chat),
    )
    .await?;
    Ok(model)
}

/// Returns the configured provider, falling back to managed embeddings. Its
/// cloud disclosures are unrouted; see [`default_embedding_provider_for_chat`].
pub async fn default_embedding_provider_with_config(config: &Config) -> Arc<dyn EmbeddingModel> {
    default_embedding_provider_for_chat(config, None).await
}

/// [`default_embedding_provider_with_config`] for a call made in a chat turn:
/// the managed (cloud) model discloses its egress to `chat`.
pub async fn default_embedding_provider_for_chat(
    config: &Config,
    chat: Option<ApprovalChatContext>,
) -> Arc<dyn EmbeddingModel> {
    let api_key = super::rpc::resolve_api_key(config, &config.memory.embedding_provider).await;
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
        &credential_resolver(api_key),
        &managed_factory(Some(config.clone()), chat),
    )
    .await
    .unwrap_or_else(|error| {
        log::warn!("[embeddings::factory] managed fallback failed to build: {error}");
        Box::new(tinyinference_embeddings::NoopEmbeddingModel)
    });
    Arc::from(model)
}

#[cfg(test)]
#[path = "factory_tests.rs"]
mod tests;
