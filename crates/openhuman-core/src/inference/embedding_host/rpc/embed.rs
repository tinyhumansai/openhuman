//! Live embedding through the configured provider, and the connectivity test.

use crate::config::Config;
use crate::core::Outcome;

use super::{resolve_api_key, LOG_PREFIX};
use crate::inference::embedding_host::factory::{
    create_embedding_provider_for_inspection, create_embedding_provider_with_config,
    credential_resolver, managed_factory, settings,
};
use tinyinference_embeddings::verify::{test_embedding_connection_detailed, ConnectionTestError};
use tinyinference_providers::route::split_custom_provider;

/// Generates embeddings for the given input texts using the currently
/// configured provider.
pub async fn embed(
    config: &Config,
    inputs: &[String],
) -> Result<Outcome<serde_json::Value>, String> {
    let provider_name = &config.memory.embedding_provider;
    let model = &config.memory.embedding_model;
    let dims = config.memory.embedding_dimensions;

    let api_key = resolve_api_key(config, provider_name).await;

    let (provider_slug, custom_endpoint) = split_custom_provider(provider_name);

    let provider_slug = if custom_endpoint.is_some() {
        "custom"
    } else {
        provider_slug.as_str()
    };

    let embedder = create_embedding_provider_with_config(
        config,
        provider_slug,
        model,
        dims,
        &api_key,
        custom_endpoint.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;

    let vectors = embedder.embed(inputs).await.map_err(|e| e.to_string())?;

    let actual_dims = vectors.first().map(|v| v.len()).unwrap_or(0);

    tracing::debug!(
        provider = provider_slug,
        model,
        input_count = inputs.len(),
        vector_count = vectors.len(),
        dims = actual_dims,
        "{LOG_PREFIX} embed completed"
    );

    let payload = serde_json::json!({
        "vectors": vectors,
        "dimensions": actual_dims,
        "count": vectors.len(),
        "provider": provider_slug,
        "model": model,
    });

    Ok(Outcome::new(payload, vec!["embedding completed".into()]))
}

/// Tests connectivity to the configured (or specified) embedding provider.
pub async fn test_connection(
    config: &Config,
    provider_slug: Option<&str>,
    model: Option<&str>,
    dims: Option<usize>,
) -> Result<Outcome<serde_json::Value>, String> {
    let slug = provider_slug.unwrap_or(&config.memory.embedding_provider);
    let model = model.unwrap_or(&config.memory.embedding_model);
    let dims = dims.unwrap_or(config.memory.embedding_dimensions);

    let (provider_slug, custom_endpoint) = split_custom_provider(slug);

    let provider_tag = if custom_endpoint.is_some() {
        "custom"
    } else {
        provider_slug.as_str()
    };

    tracing::debug!(
        provider = provider_tag,
        model,
        dims,
        "{LOG_PREFIX} test_connection starting"
    );

    let custom_provider = custom_endpoint
        .as_deref()
        .map(|endpoint| format!("custom:{endpoint}"));
    let selected = settings(
        custom_provider.as_deref().unwrap_or(provider_tag),
        model,
        dims,
        tinyinference_local::ollama::ollama_base_url_from_override(
            config.local_ai.base_url.as_deref(),
        ),
    );
    let api_key = resolve_api_key(config, slug).await;
    let credentials = credential_resolver(api_key);
    let managed = managed_factory(Some(config.clone()), None);
    let result = test_embedding_connection_detailed(
        &selected,
        &credentials,
        &managed,
        custom_endpoint.as_deref(),
    )
    .await;
    let result = match result {
        Ok(result) => Ok(result.actual_dims),
        Err(ConnectionTestError::Build(error)) => return Err(error.to_string()),
        Err(ConnectionTestError::Embed(error)) => Err(error.to_string()),
    };

    match result {
        Ok(actual_dims) => {
            let payload = serde_json::json!({
                "success": true,
                "provider": provider_tag,
                "model": model,
                "requested_dimensions": dims,
                "actual_dimensions": actual_dims,
            });
            Ok(Outcome::new(payload, vec!["connection test passed".into()]))
        }
        Err(e) => {
            let payload = serde_json::json!({
                "success": false,
                "provider": provider_tag,
                "model": model,
                "error": e.to_string(),
            });
            Ok(Outcome::new(
                payload,
                vec![format!("connection test failed: {e}")],
            ))
        }
    }
}

/// The requested and actual vector widths from one read-only doctor probe.
pub(crate) struct InspectionEmbeddingDimensions {
    pub configured: usize,
    pub actual: usize,
}

/// Safe failure classes for the live doctor embedding probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InspectionEmbeddingError {
    CredentialStoreUnavailable,
    ProviderUnavailable,
    ProbeFailed,
}

/// Embeds one fixed, non-user string through the same configured provider
/// factory, using read-only profile credentials and an inspection-safe managed
/// provider when the selected route is managed.
pub(crate) async fn probe_dimensions_for_inspection(
    config: &Config,
) -> Result<InspectionEmbeddingDimensions, InspectionEmbeddingError> {
    let provider_name = config.memory.embedding_provider.trim();
    let model = config.memory.embedding_model.trim();
    let configured = config.memory.embedding_dimensions;
    let api_key = super::resolve_api_key_read_only(config, provider_name)
        .await
        .map_err(|_| InspectionEmbeddingError::CredentialStoreUnavailable)?;
    let custom_endpoint = provider_name.strip_prefix("custom:").map(str::to_owned);
    let provider_slug = if custom_endpoint.is_some() {
        "custom"
    } else {
        provider_name
    };

    // Ollama's `/api/embed` endpoint does not accept a requested output width.
    // The normal provider constructor uses `configured` as a validation width,
    // which turns a doctor probe into an opaque failure before it can report
    // the actual vector width. Use TinyInference's single-request discovery
    // path for this inspection only; runtime embeddings keep their configured
    // width validation.
    let actual = if provider_slug == "ollama" {
        let base_url = tinyinference_local::ollama::ollama_base_url_from_override(
            config.local_ai.base_url.as_deref(),
        );
        let input = ["OpenHuman doctor embedding probe".to_owned()];
        let (_, vectors) =
            tinyinference_embeddings::OllamaEmbeddingModel::embed_discovering_dimensions(
                &base_url,
                model,
                reqwest::Client::new(),
                &input,
                tinyinference_embeddings::RECOMMENDED_OLLAMA_CONTEXT_TOKENS,
                1,
            )
            .await
            .map_err(|_| InspectionEmbeddingError::ProbeFailed)?;
        vectors.first().map_or(0, Vec::len)
    } else {
        let embedder = create_embedding_provider_for_inspection(
            config,
            provider_slug,
            model,
            configured,
            &api_key,
            custom_endpoint.as_deref(),
        )
        .await
        .map_err(|_| InspectionEmbeddingError::ProviderUnavailable)?;
        let vectors = embedder
            .embed(&["OpenHuman doctor embedding probe".to_owned()])
            .await
            .map_err(|_| InspectionEmbeddingError::ProbeFailed)?;
        vectors.first().map_or(0, Vec::len)
    };
    Ok(InspectionEmbeddingDimensions { configured, actual })
}
