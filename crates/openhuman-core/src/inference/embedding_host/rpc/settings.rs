//! Embedding settings: reading the current configuration and applying an
//! update, including the save-time verification of a custom endpoint.

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;
use crate::credentials::AuthService;
use tinyinference_providers::route::{custom_provider_string, split_custom_provider};

use super::{resolve_api_key, LOG_PREFIX};
use tinyinference_embeddings::catalog;
use tinyinference_embeddings::verify::{
    effective_embedder_slug, verify_custom_endpoint, VerifyRejection,
};

fn probe_rejection_outcome(
    reject: tinyinference_embeddings::probe::EmbeddingProbeRejection,
) -> Outcome<serde_json::Value> {
    let mut body = serde_json::json!({ "error": reject.error, "message": reject.message });
    if let Some(detail) = reject.detail {
        body["detail"] = serde_json::Value::String(detail);
    }
    Outcome::new(body, vec![reject.summary.to_string()])
}

fn model_rejection_outcome(
    reject: tinyinference_embeddings::served_models::ModelNotServed,
) -> Outcome<serde_json::Value> {
    let mut body = serde_json::json!({
        "error": reject.error,
        "message": reject.message,
        "requested_model": reject.requested_model,
        "available_models": reject.available_models,
    });
    if let Some(suggestion) = reject.suggested_model {
        body["suggested_model"] = serde_json::Value::String(suggestion);
    }
    Outcome::new(body, vec![reject.summary.to_string()])
}

fn active_custom_profile(config: &Config) -> Option<crate::config::schema::CustomEmbeddingsConfig> {
    let (_, endpoint) = split_custom_provider(&config.memory.embedding_provider);
    let endpoint = endpoint?;
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return None;
    }
    Some(crate::config::schema::CustomEmbeddingsConfig {
        endpoint: endpoint.to_string(),
        model: config.memory.embedding_model.clone(),
        dimensions: config.memory.embedding_dimensions,
    })
}

pub(super) fn remember_active_custom_profile(config: &mut Config) {
    if let Some(profile) = active_custom_profile(config) {
        config.custom_embeddings = Some(profile);
    }
}

/// Returns the current embedding settings plus the provider catalog.
pub async fn get_settings(config: &Config) -> Result<Outcome<serde_json::Value>, String> {
    let provider = &config.memory.embedding_provider;
    let model = &config.memory.embedding_model;
    let dimensions = config.memory.embedding_dimensions;
    let rate_limit = config.memory.embedding_rate_limit_per_min;

    // Older configs encode the endpoint only in the active provider string.
    // Prefer that live value when Custom is selected, and otherwise return the
    // retained profile so disabling embeddings does not blank the edit form.
    let custom_settings = active_custom_profile(config)
        .or_else(|| config.custom_embeddings.clone())
        .map(|profile| {
            serde_json::json!({
                "endpoint": profile.endpoint,
                "model": profile.model,
                "dimensions": profile.dimensions,
            })
        });

    let auth = AuthService::from_config(config);
    let mut providers = Vec::new();
    for entry in catalog::all_providers() {
        let has_key = if entry.requires_api_key {
            let cred_provider = format!("embeddings:{}", entry.slug);
            auth.get_provider_bearer_token(&cred_provider, None)
                .await
                .ok()
                .flatten()
                .is_some()
        } else {
            false
        };
        providers.push(serde_json::json!({
            "slug": entry.slug,
            "label": entry.label,
            "description": entry.description,
            "requires_api_key": entry.requires_api_key,
            "requires_endpoint": entry.requires_endpoint,
            "has_api_key": has_key,
            "models": entry.models,
        }));
    }

    let vector_search_enabled = {
        let (_, endpoint) = split_custom_provider(provider);
        let slug = if endpoint.is_some() {
            "custom"
        } else {
            provider.as_str()
        };
        slug != "none"
    };

    // The embedder ingestion will *actually* use. `provider` above is the
    // per-section setting the picker writes; it is NOT authoritative for how
    // embeddings are funded, because the Local AI "Memory embeddings" toggle
    // routes to local Ollama without rewriting it. Additive field — callers that only need the picker
    // value are unaffected; callers asking "does this bill the managed budget?"
    // must read this one (#5402).
    let local_model = config.workload_local_model("embeddings");
    let session_exists = auth
        .get_provider_bearer_token(crate::credentials::APP_SESSION_PROVIDER, None)
        .await
        .ok()
        .flatten()
        .is_some_and(|token| !token.trim().is_empty());
    let effective_provider = effective_embedder_slug(
        config
            .embeddings_provider
            .as_deref()
            .map(str::trim)
            .is_some_and(|provider| provider == "none"),
        local_model.as_deref(),
        Some(config.memory.embedding_provider.as_str()),
        session_exists,
    );

    let payload = serde_json::json!({
        "provider": provider,
        "effective_provider": effective_provider,
        "model": model,
        "dimensions": dimensions,
        "rate_limit_per_min": rate_limit,
        "custom_settings": custom_settings,
        "providers": providers,
        "vector_search_enabled": vector_search_enabled,
    });

    tracing::debug!(
        provider = provider.as_str(),
        effective_provider,
        model = model.as_str(),
        dimensions,
        vector_search_enabled,
        "{LOG_PREFIX} get_settings"
    );

    Ok(Outcome::new(
        payload,
        vec!["embeddings settings loaded".into()],
    ))
}

/// Updates embedding provider/model/dimensions. Nothing persisted depends on
/// the embedding signature any more (memory v2 engines embed server-side), so a
/// change applies directly.
pub async fn update_settings(
    ctx: &DispatchCtx,
    provider: Option<String>,
    model: Option<String>,
    dimensions: Option<usize>,
    custom_endpoint: Option<String>,
    rate_limit_per_min: Option<u32>,
) -> Result<Outcome<serde_json::Value>, String> {
    use tinyinference_embeddings::format_embedding_signature;

    let mut config = ctx.load_config().await?;

    // Upgrade-in-place for users whose endpoint predates the retained Custom
    // profile: if they disable or switch away now, capture the active profile
    // before `memory.embedding_provider` is overwritten below.
    remember_active_custom_profile(&mut config);

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
    // Resolve the provider string exactly as it will be persisted so the
    // signature reported to callers describes the saved configuration.
    let effective_provider = match &custom_endpoint {
        Some(ep)
            if new_provider == "custom" || split_custom_provider(&new_provider).1.is_some() =>
        {
            custom_provider_string("custom", Some(ep))
        }
        _ => new_provider.clone(),
    };
    // Custom verification can auto-detect the endpoint's vector width.
    let mut new_dims = dimensions.unwrap_or(config.memory.embedding_dimensions);
    let mut new_sig = format_embedding_signature(&effective_provider, &new_model, new_dims);

    let mut sig_changed = new_sig != old_sig;

    // Only custom endpoints are probed: named catalog providers are
    // embedding-capable by construction, and probing managed/cloud before
    // login would false-fail. TinyInference owns endpoint verification and
    // its structured rejection policy.
    if let (_, Some(endpoint)) = split_custom_provider(&effective_provider) {
        let api_key = resolve_api_key(&config, "custom").await;
        tracing::debug!(
            provider = effective_provider.as_str(),
            "{LOG_PREFIX} update_settings verifying embeddings endpoint with a test embed"
        );
        match verify_custom_endpoint(&endpoint, &api_key, &new_model, new_dims).await {
            Ok(verified) => {
                if verified.dimensions != new_dims {
                    tracing::info!(
                        provider = effective_provider.as_str(),
                        model = new_model.as_str(),
                        requested = new_dims,
                        detected = verified.dimensions,
                        "{LOG_PREFIX} update_settings auto-detected custom embedding dimension from probe"
                    );
                }
                new_dims = verified.dimensions;
                new_sig = verified.signature;
                sig_changed = new_sig != old_sig;
                tracing::debug!(
                    provider = effective_provider.as_str(),
                    new_dims,
                    "{LOG_PREFIX} update_settings test embed passed — accepting config"
                );
            }
            Err(rejection) => {
                tracing::warn!(
                    provider = effective_provider.as_str(),
                    reject_code = rejection.error(),
                    "{LOG_PREFIX} update_settings rejected — embeddings endpoint failed verification"
                );
                return Ok(match rejection {
                    VerifyRejection::Probe(reject) => probe_rejection_outcome(reject),
                    VerifyRejection::ModelNotServed(reject) => model_rejection_outcome(*reject),
                });
            }
        }
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
    // Persist `new_dims`, not the raw `dimensions` arg: the Custom verification
    // probe may have auto-detected the endpoint's real length (issue #4056), and
    // `new_dims` already defaults to the stored value when neither a new arg nor
    // detection changed it — so this is a no-op for the unchanged case.
    config.memory.embedding_dimensions = new_dims;
    if let Some(rl) = rate_limit_per_min {
        config.memory.embedding_rate_limit_per_min = rl;
    }
    // Store custom endpoint in a convention field if provided
    if let Some(ep) = &custom_endpoint {
        if new_provider == "custom" || split_custom_provider(&new_provider).1.is_some() {
            config.memory.embedding_provider = custom_provider_string("custom", Some(ep));
            config.custom_embeddings = Some(crate::config::schema::CustomEmbeddingsConfig {
                endpoint: ep.clone(),
                model: new_model.clone(),
                dimensions: new_dims,
            });
        }
    }

    config
        .save()
        .await
        .map_err(crate::config::ops::config_save_error)?;

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

    Ok(Outcome::new(
        payload,
        vec![format!(
            "embeddings settings updated (sig_changed={sig_changed})"
        )],
    ))
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
