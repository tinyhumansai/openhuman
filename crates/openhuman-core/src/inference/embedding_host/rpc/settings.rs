//! Embedding settings: reading the current configuration and applying an
//! update, including the save-time verification of a custom endpoint.

use crate::config::Config;
use crate::core::Outcome;
use crate::security::credentials::AuthService;

use super::{resolve_api_key, LOG_PREFIX};
use tinyinference_embeddings::catalog;
use tinyinference_embeddings::probe::{
    classify_embed_probe, final_probe_dims, probe_custom_embeddings, EmbedProbe,
    EmbeddingProbeRejection,
};
use tinyinference_embeddings::served_models::{
    check_requested_model_served, fetch_served_model_ids, ModelNotServed,
};

fn probe_rejection_outcome(reject: EmbeddingProbeRejection) -> Outcome<serde_json::Value> {
    let mut body = serde_json::json!({ "error": reject.error, "message": reject.message });
    if let Some(detail) = reject.detail {
        body["detail"] = serde_json::Value::String(detail);
    }
    Outcome::new(body, vec![reject.summary.to_string()])
}

fn model_rejection_outcome(reject: ModelNotServed) -> Outcome<serde_json::Value> {
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

/// Slug naming the embedder ingestion will actually use, resolved host-side
/// from the `Config` fields the resolution ladder reads.
///
/// Resolution order:
/// 1. Deliberate opt-out — `embeddings_provider` trimmed equals `"none"` → `"none"`.
/// 2. Local Ollama via unified workload setting — `workload_local_model("embeddings")`
///    is `Some` → `"ollama"`.
/// 3. User OpenAI-compatible endpoint — `memory.embedding_provider` is
///    `"openai"`, `"custom"`, or starts with `"custom:"` → `"custom"`.
/// 4. Managed cloud session — `auth-profiles.json` exists next to the config
///    file → `"cloud"`.
/// 5. Nothing usable → `"unconfigured"`.
fn effective_embedder_slug_from_config(config: &Config) -> &'static str {
    // 1. Deliberate opt-out.
    if config
        .embeddings_provider
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| s == "none")
    {
        return "none";
    }
    // 2. Local Ollama via unified workload setting.
    if config.workload_local_model("embeddings").is_some() {
        return "ollama";
    }
    // 3. User OpenAI-compatible endpoint.
    let picker = config.memory.embedding_provider.trim();
    if picker == "openai" || picker == "custom" || picker.starts_with("custom:") {
        return "custom";
    }
    // 4. Managed cloud session.
    let session_exists = config
        .config_path
        .parent()
        .map(|dir| dir.join("auth-profiles.json").exists())
        .unwrap_or(false);
    if session_exists {
        return "cloud";
    }
    "unconfigured"
}

fn active_custom_profile(config: &Config) -> Option<crate::config::schema::CustomEmbeddingsConfig> {
    let endpoint = config
        .memory
        .embedding_provider
        .strip_prefix("custom:")?
        .trim();
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

    // The embedder ingestion will *actually* use. `provider` above is the
    // per-section setting the picker writes; it is NOT authoritative for how
    // embeddings are funded, because the Local AI "Memory embeddings" toggle
    // routes to local Ollama without rewriting it. Additive field — callers that only need the picker
    // value are unaffected; callers asking "does this bill the managed budget?"
    // must read this one (#5402).
    let effective_provider = effective_embedder_slug_from_config(config);

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
    provider: Option<String>,
    model: Option<String>,
    dimensions: Option<usize>,
    custom_endpoint: Option<String>,
    rate_limit_per_min: Option<u32>,
) -> Result<Outcome<serde_json::Value>, String> {
    use crate::config::ops as config_rpc;
    use crate::inference::embedding_host::format_embedding_signature;

    let mut config = config_rpc::load_config_with_timeout().await?;

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
    // `new_dims`/`new_sig` are recomputed after the Custom
    // verification probe auto-detects the endpoint's real vector length
    // (issue #4056), so they must be mutable.
    let mut new_dims = dimensions.unwrap_or(config.memory.embedding_dimensions);
    let mut new_sig = format_embedding_signature(&new_provider, &new_model, new_dims);

    let mut sig_changed = new_sig != old_sig;

    // Setup-time verification gate (TAURI-RUST-5JR / 4P4): a Custom
    // (OpenAI-compatible) embeddings endpoint — e.g. LM Studio — must prove it
    // can actually embed *before* we accept it. We run one live test embed and
    // only persist the config if it succeeds; any failure (no `/embeddings`
    // route, no model loaded, timeout, 5xx, empty/zero-dim vector) rejects the
    // save so a config that can't embed is never stored (and we never wipe
    // memory for one). Verifying at setup is the fix — we deliberately do NOT
    // try to classify-and-suppress the resulting embed flood in code; any
    // residual flood (e.g. the user unloads the model *after* a good save) is
    // handled on the Sentry side.
    //
    // Only custom endpoints are probed: named catalog providers are
    // embedding-capable by construction, and probing `managed`/`cloud`
    // pre-login would false-fail. Resolve the provider string exactly as it
    // will be stored so the probe targets the real endpoint.
    let effective_provider = match &custom_endpoint {
        Some(ep) if new_provider == "custom" || new_provider.starts_with("custom:") => {
            format!("custom:{ep}")
        }
        _ => new_provider.clone(),
    };
    if effective_provider.starts_with("custom:") {
        // Probe dimension-agnostically for non-`text-embedding-3-*` models so the
        // user's guessed `dimensions` can't fail an otherwise-valid endpoint; the
        // real length is detected from the returned vector below (issue #4056).
        let endpoint = effective_provider
            .strip_prefix("custom:")
            .expect("custom provider was checked above");
        let api_key = resolve_api_key(&config, "custom");
        let probe = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            probe_custom_embeddings(endpoint, &api_key, &new_model, new_dims),
        )
        .await;
        {
            // Time-box the probe so a black-hole host can't hang the RPC.
            tracing::debug!(
                provider = effective_provider.as_str(),
                "{LOG_PREFIX} update_settings verifying embeddings endpoint with a test embed"
            );
            // Normalize the timeout/result into one shape, then apply the
            // pure verification policy (`classify_embed_probe`, unit-tested).
            let outcome = match probe {
                Ok(Ok(vectors)) => EmbedProbe::Returned(vectors),
                Ok(Err(e)) => EmbedProbe::Failed(e.to_string()),
                Err(_elapsed) => EmbedProbe::TimedOut,
            };
            // Peek the actual vector length before the policy consumes the
            // outcome — on a pass this is the endpoint's real dimension.
            let probe_actual_dims = match &outcome {
                EmbedProbe::Returned(vectors) => vectors.first().map(|v| v.len()).unwrap_or(0),
                _ => 0,
            };
            if let Some(reject) = classify_embed_probe(outcome) {
                // Log the classified error code (never the raw detail — it can
                // carry endpoint response bodies) so support can distinguish
                // auth vs wrong-model vs unreachable failures (issue #5017).
                let reject_code = reject.error;
                tracing::warn!(
                        provider = effective_provider.as_str(),
                        reject_code,
                        "{LOG_PREFIX} update_settings rejected — embeddings endpoint failed verification"
                    );
                // Right-feedback (issue #3761): the probe failed. If the
                // endpoint lists its served models and the requested id
                // isn't among them, the cause is almost certainly a name
                // mismatch (e.g. the user entered `bge-m3` but LM Studio
                // serves `text-embedding-bge-m3`). Replace the generic
                // failure with an actionable message naming the available
                // models and the suggested match. Best-effort and only on
                // the failure path, so a passing config is never blocked by
                // an endpoint that doesn't expose `/models`. Derive the
                // endpoint from the payload OR the already-stored
                // `custom:<url>` provider, so a model-only update to an
                // existing custom endpoint still gets the guidance.
                let listed_endpoint = custom_endpoint
                    .as_deref()
                    .or_else(|| effective_provider.strip_prefix("custom:"));
                if let Some(ep) = listed_endpoint {
                    let api_key = resolve_api_key(&config, "custom");
                    tracing::debug!(
                            provider = effective_provider.as_str(),
                            requested = new_model.as_str(),
                            "{LOG_PREFIX} update_settings: probing endpoint /models for served-id guidance"
                        );
                    match fetch_served_model_ids(ep, &api_key).await {
                        Ok(served) => match check_requested_model_served(&new_model, &served) {
                            Some(better) => {
                                tracing::warn!(
                                        provider = effective_provider.as_str(),
                                        requested = new_model.as_str(),
                                        served = served.len(),
                                        "{LOG_PREFIX} update_settings: model not in served list — returning name-mismatch guidance"
                                    );
                                return Ok(model_rejection_outcome(better));
                            }
                            None => {
                                tracing::debug!(
                                        provider = effective_provider.as_str(),
                                        served = served.len(),
                                        "{LOG_PREFIX} update_settings: requested model is served (or list empty) — keeping generic verification error"
                                    );
                            }
                        },
                        Err(e) => {
                            tracing::debug!(
                                provider = effective_provider.as_str(),
                                error = %e,
                                "{LOG_PREFIX} update_settings: /models lookup failed — keeping generic verification error"
                            );
                        }
                    }
                }
                return Ok(probe_rejection_outcome(reject));
            }
            // Passed. Adopt the endpoint's real vector length for every model
            // we probed dimension-agnostically — the user can't be expected to
            // know it, and storing the actual size is what keeps the live embed
            // path's length guard from rejecting future embeds (issue #4056).
            // Models with configurable output widths were probed with the
            // requested size and rejected if the server ignored it.
            let detected_dims = final_probe_dims(&new_model, new_dims, probe_actual_dims);
            if detected_dims != new_dims {
                tracing::info!(
                        provider = effective_provider.as_str(),
                        model = new_model.as_str(),
                        requested = new_dims,
                        detected = detected_dims,
                        "{LOG_PREFIX} update_settings auto-detected custom embedding dimension from probe"
                    );
                new_dims = detected_dims;
                new_sig = format_embedding_signature(&new_provider, &new_model, new_dims);
                sig_changed = new_sig != old_sig;
            }
            tracing::debug!(
                provider = effective_provider.as_str(),
                new_dims,
                "{LOG_PREFIX} update_settings test embed passed — accepting config"
            );
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
        if new_provider == "custom" || new_provider.starts_with("custom:") {
            config.memory.embedding_provider = format!("custom:{ep}");
            config.custom_embeddings = Some(crate::config::schema::CustomEmbeddingsConfig {
                endpoint: ep.clone(),
                model: new_model.clone(),
                dimensions: new_dims,
            });
        }
    }

    config.save().await.map_err(|e| e.to_string())?;

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
