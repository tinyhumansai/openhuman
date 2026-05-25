//! RPC handlers for the embeddings domain.

use std::collections::HashMap;

use crate::openhuman::config::Config;
use crate::openhuman::credentials::AuthService;
use crate::rpc::RpcOutcome;

use super::catalog;
use super::factory::create_embedding_provider_with_credentials;

const LOG_PREFIX: &str = "[embeddings::rpc]";

/// Returns the current embedding settings plus the provider catalog.
pub async fn get_settings(config: &Config) -> Result<RpcOutcome<serde_json::Value>, String> {
    let provider = &config.memory.embedding_provider;
    let model = &config.memory.embedding_model;
    let dimensions = config.memory.embedding_dimensions;
    let rate_limit = config.memory.embedding_rate_limit_per_min;

    let auth = AuthService::from_config(config);
    let providers: Vec<serde_json::Value> = catalog::all_providers()
        .iter()
        .map(|entry| {
            let has_key = if entry.requires_api_key {
                let cred_provider = format!("embeddings:{}", entry.slug);
                auth.get_provider_bearer_token(&cred_provider, None)
                    .ok()
                    .flatten()
                    .is_some()
            } else {
                false
            };
            serde_json::json!({
                "slug": entry.slug,
                "label": entry.label,
                "description": entry.description,
                "requires_api_key": entry.requires_api_key,
                "requires_endpoint": entry.requires_endpoint,
                "has_api_key": has_key,
                "models": entry.models,
            })
        })
        .collect();

    let vector_search_enabled = {
        let slug = if provider.starts_with("custom:") {
            "custom"
        } else {
            provider.as_str()
        };
        slug != "none"
    };

    let payload = serde_json::json!({
        "provider": provider,
        "model": model,
        "dimensions": dimensions,
        "rate_limit_per_min": rate_limit,
        "providers": providers,
        "vector_search_enabled": vector_search_enabled,
    });

    tracing::debug!(
        provider = provider.as_str(),
        model = model.as_str(),
        dimensions,
        vector_search_enabled,
        "{LOG_PREFIX} get_settings"
    );

    Ok(RpcOutcome::new(
        payload,
        vec!["embeddings settings loaded".into()],
    ))
}

/// Updates embedding provider/model/dimensions. If the embedding signature
/// changes, requires `confirm_wipe = true` and wipes memory.
pub async fn update_settings(
    provider: Option<String>,
    model: Option<String>,
    dimensions: Option<usize>,
    custom_endpoint: Option<String>,
    rate_limit_per_min: Option<u32>,
    confirm_wipe: bool,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    use crate::openhuman::config::ops as config_rpc;
    use crate::openhuman::embeddings::format_embedding_signature;

    let mut config = config_rpc::load_config_with_timeout().await?;

    let old_sig = format_embedding_signature(
        &config.memory.embedding_provider,
        &config.memory.embedding_model,
        config.memory.embedding_dimensions,
    );

    let new_provider = provider
        .clone()
        .unwrap_or_else(|| config.memory.embedding_provider.clone());
    let new_model = model
        .clone()
        .unwrap_or_else(|| config.memory.embedding_model.clone());
    let new_dims = dimensions.unwrap_or(config.memory.embedding_dimensions);
    let new_sig = format_embedding_signature(&new_provider, &new_model, new_dims);

    let old_dims = config.memory.embedding_dimensions;
    let dims_changed = new_dims != old_dims;
    let sig_changed = new_sig != old_sig;

    // Only require a wipe when dimensions actually change — switching
    // provider/model at the same dimensionality keeps vectors comparable.
    if dims_changed && !confirm_wipe {
        let payload = serde_json::json!({
            "error": "EMBEDDINGS_DIMENSION_CHANGE_REQUIRES_WIPE",
            "old_dimensions": old_dims,
            "new_dimensions": new_dims,
            "old_signature": old_sig,
            "new_signature": new_sig,
            "message": "Changing embedding dimensions invalidates all stored vectors. \
                        Pass confirm_wipe=true to wipe memory and apply.",
        });
        return Ok(RpcOutcome::new(
            payload,
            vec!["embedding dimension change requires wipe confirmation".into()],
        ));
    }

    if dims_changed {
        tracing::warn!(
            old_dims,
            new_dims,
            "{LOG_PREFIX} embedding dimensions changing — wiping memory"
        );
        crate::openhuman::memory::read_rpc::wipe_all_rpc(&config)
            .await
            .map_err(|e| format!("memory wipe failed: {e}"))?;
    }

    // Apply provider
    if let Some(p) = &provider {
        config.memory.embedding_provider = p.clone();
        // Also update the workload routing to keep them in sync
        config.embeddings_provider = Some(match p.as_str() {
            "managed" | "cloud" => "openhuman".to_string(),
            "ollama" => format!("ollama:{new_model}"),
            other => other.to_string(),
        });
    }
    if let Some(m) = &model {
        config.memory.embedding_model = m.clone();
    }
    if let Some(d) = dimensions {
        config.memory.embedding_dimensions = d;
    }
    if let Some(rl) = rate_limit_per_min {
        config.memory.embedding_rate_limit_per_min = rl;
    }
    // Store custom endpoint in a convention field if provided
    if let Some(ep) = &custom_endpoint {
        if new_provider == "custom" || new_provider.starts_with("custom:") {
            config.memory.embedding_provider = format!("custom:{ep}");
        }
    }

    config.save().await.map_err(|e| e.to_string())?;

    if sig_changed {
        crate::openhuman::memory_queue::ensure_reembed_backfill(&config);
    }

    tracing::info!(
        provider = config.memory.embedding_provider.as_str(),
        model = config.memory.embedding_model.as_str(),
        dimensions = config.memory.embedding_dimensions,
        sig_changed,
        "{LOG_PREFIX} update_settings applied"
    );

    let payload = serde_json::json!({
        "provider": config.memory.embedding_provider,
        "model": config.memory.embedding_model,
        "dimensions": config.memory.embedding_dimensions,
        "signature_changed": sig_changed,
        "new_signature": new_sig,
    });

    Ok(RpcOutcome::new(
        payload,
        vec![format!(
            "embeddings settings updated (sig_changed={sig_changed})"
        )],
    ))
}

/// Stores an API key for a specific embedding provider.
pub async fn set_api_key(
    config: &Config,
    provider_slug: &str,
    api_key: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
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

    Ok(RpcOutcome::new(
        serde_json::json!({ "stored": true, "provider": provider_slug }),
        vec![format!("embedding API key stored for {provider_slug}")],
    ))
}

/// Removes the API key for a specific embedding provider.
pub async fn clear_api_key(
    config: &Config,
    provider_slug: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
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

    Ok(RpcOutcome::new(
        serde_json::json!({ "cleared": removed, "provider": provider_slug }),
        vec![format!("embedding API key cleared for {provider_slug}")],
    ))
}

/// Generates embeddings for the given input texts using the currently
/// configured provider.
pub async fn embed(
    config: &Config,
    inputs: &[String],
) -> Result<RpcOutcome<serde_json::Value>, String> {
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

    let embedder = create_embedding_provider_with_credentials(
        provider_slug,
        model,
        dims,
        &api_key,
        custom_endpoint.as_deref(),
    )
    .map_err(|e| e.to_string())?;

    let refs: Vec<&str> = inputs.iter().map(|s| s.as_str()).collect();
    let vectors = embedder.embed(&refs).await.map_err(|e| e.to_string())?;

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

    Ok(RpcOutcome::new(payload, vec!["embedding completed".into()]))
}

/// Tests connectivity to the configured (or specified) embedding provider.
pub async fn test_connection(
    config: &Config,
    provider_slug: Option<&str>,
    model: Option<&str>,
    dims: Option<usize>,
) -> Result<RpcOutcome<serde_json::Value>, String> {
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

    let embedder = create_embedding_provider_with_credentials(
        provider_tag,
        model,
        dims,
        &api_key,
        custom_endpoint.as_deref(),
    )
    .map_err(|e| e.to_string())?;

    tracing::debug!(
        provider = provider_tag,
        model,
        dims,
        "{LOG_PREFIX} test_connection starting"
    );

    match embedder.embed(&["connection test"]).await {
        Ok(vectors) => {
            let actual_dims = vectors.first().map(|v| v.len()).unwrap_or(0);
            let payload = serde_json::json!({
                "success": true,
                "provider": provider_tag,
                "model": model,
                "requested_dimensions": dims,
                "actual_dimensions": actual_dims,
            });
            Ok(RpcOutcome::new(
                payload,
                vec!["connection test passed".into()],
            ))
        }
        Err(e) => {
            let payload = serde_json::json!({
                "success": false,
                "provider": provider_tag,
                "model": model,
                "error": e.to_string(),
            });
            Ok(RpcOutcome::new(
                payload,
                vec![format!("connection test failed: {e}")],
            ))
        }
    }
}

fn resolve_api_key(config: &Config, provider_name: &str) -> String {
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
