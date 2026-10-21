//! Live embedding through the configured provider, and the connectivity test.

use crate::config::Config;
use crate::core::Outcome;

use super::{resolve_api_key, LOG_PREFIX};
use crate::inference::embedding_host::factory::{
    create_embedding_provider_for_inspection, create_embedding_provider_with_config,
};
use tinyinference_embeddings::probe::probe_custom_embeddings;

/// Generates embeddings for the given input texts using the currently
/// configured provider.
pub async fn embed(
    config: &Config,
    inputs: &[String],
) -> Result<Outcome<serde_json::Value>, String> {
    let provider_name = &config.memory.embedding_provider;
    let model = &config.memory.embedding_model;
    let dims = config.memory.embedding_dimensions;

    let api_key = resolve_api_key(config, provider_name);

    let custom_endpoint = if provider_name.starts_with("custom:") {
        provider_name
            .strip_prefix("custom:")
            .map(|s: &str| s.to_string())
    } else {
        None
    };

    let provider_slug = if provider_name.starts_with("custom:") {
        "custom"
    } else {
        provider_name.as_str()
    };

    let embedder = create_embedding_provider_with_config(
        config,
        provider_slug,
        model,
        dims,
        &api_key,
        custom_endpoint.as_deref(),
    )
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

    let api_key = resolve_api_key(config, slug);

    let custom_endpoint = if slug.starts_with("custom:") {
        slug.strip_prefix("custom:").map(|s| s.to_string())
    } else {
        None
    };

    let provider_tag = if slug.starts_with("custom:") {
        "custom"
    } else {
        slug
    };

    tracing::debug!(
        provider = provider_tag,
        model,
        dims,
        "{LOG_PREFIX} test_connection starting"
    );

    let result = if let Some(endpoint) = custom_endpoint.as_deref() {
        probe_custom_embeddings(endpoint, &api_key, model, dims)
            .await
            .map_err(|error| error.to_string())
    } else {
        let embedder = create_embedding_provider_with_config(
            config,
            provider_tag,
            model,
            dims,
            &api_key,
            None,
        )
        .map_err(|e| e.to_string())?;
        embedder
            .embed(&["connection test".to_string()])
            .await
            .map_err(|e| e.to_string())
    };

    match result {
        Ok(vectors) => {
            let actual_dims = vectors.first().map(|v| v.len()).unwrap_or(0);
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
        .map_err(|_| InspectionEmbeddingError::ProviderUnavailable)?;
        let vectors = embedder
            .embed(&["OpenHuman doctor embedding probe".to_owned()])
            .await
            .map_err(|_| InspectionEmbeddingError::ProbeFailed)?;
        vectors.first().map_or(0, Vec::len)
    };
    Ok(InspectionEmbeddingDimensions { configured, actual })
}
