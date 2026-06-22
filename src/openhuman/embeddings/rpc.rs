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
        match build_embedder(&config, &effective_provider, &new_model, new_dims) {
            Ok(embedder) => {
                // Time-box the probe so a black-hole host can't hang the RPC.
                tracing::debug!(
                    provider = effective_provider.as_str(),
                    "{LOG_PREFIX} update_settings verifying embeddings endpoint with a test embed"
                );
                let probe = tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    embedder.embed(&["connection test"]),
                )
                .await;
                // Normalize the timeout/result into one shape, then apply the
                // pure verification policy (`classify_embed_probe`, unit-tested).
                let outcome = match probe {
                    Ok(Ok(vectors)) => EmbedProbe::Returned(vectors),
                    Ok(Err(e)) => EmbedProbe::Failed(e.to_string()),
                    Err(_elapsed) => EmbedProbe::TimedOut,
                };
                if let Some(reject) = classify_embed_probe(outcome) {
                    tracing::warn!(
                        provider = effective_provider.as_str(),
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
                                    return Ok(better);
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
                    return Ok(reject);
                }
                tracing::debug!(
                    provider = effective_provider.as_str(),
                    "{LOG_PREFIX} update_settings test embed passed — accepting config"
                );
            }
            Err(e) => {
                // Construction failure (unknown slug / bad config) — surface it
                // rather than persisting a config that can never embed.
                return Err(format!("invalid embedding provider configuration: {e}"));
            }
        }
    }

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

/// Build an embedding provider from the live config — the same construction
/// [`embed`] uses, exposed so other domains (e.g. `codegraph`) can obtain a
/// provider for `signature()` + direct embedding without a JSON-RPC round-trip.
pub fn provider_from_config(config: &Config) -> anyhow::Result<Box<dyn super::EmbeddingProvider>> {
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
) -> anyhow::Result<Box<dyn super::EmbeddingProvider>> {
    let api_key = resolve_api_key(config, provider_name);
    let custom_endpoint = provider_name.strip_prefix("custom:").map(|s| s.to_string());
    let provider_slug = if provider_name.starts_with("custom:") {
        "custom"
    } else {
        provider_name
    };
    create_embedding_provider_with_credentials(
        provider_slug,
        model,
        dims,
        &api_key,
        custom_endpoint.as_deref(),
    )
}

/// Normalized result of the setup-time test embed in [`update_settings`].
/// Collapses the `Result<Result<_, _>, Elapsed>` timeout shape into one enum so
/// the verification policy can be expressed (and unit-tested) as a pure
/// function over it.
enum EmbedProbe {
    /// The endpoint returned vectors (may still be empty/zero-dim — checked).
    Returned(Vec<Vec<f32>>),
    /// The embed call returned an error; the string is the provider detail.
    Failed(String),
    /// The probe didn't complete within the time box.
    TimedOut,
}

/// Setup-time embeddings verification policy. Returns `None` when the endpoint
/// is verified (accept + persist the config) or `Some(reject)` — the
/// "not saved" RPC payload — otherwise.
///
/// The endpoint must prove it can embed before we accept it: only a non-empty
/// vector passes; every failure mode (no model loaded, no `/embeddings` route,
/// 5xx/auth/network, timeout, empty vector) rejects the save. We do NOT try to
/// classify-and-suppress the resulting embed flood in code — residual floods
/// (e.g. the user unloads the model after a good save) are handled Sentry-side.
/// The known shapes only get a friendlier remediation message.
fn classify_embed_probe(outcome: EmbedProbe) -> Option<RpcOutcome<serde_json::Value>> {
    let reject = |error: &str, message: &str, summary: &str, detail: Option<&str>| {
        let mut body = serde_json::json!({ "error": error, "message": message });
        if let Some(d) = detail {
            body["detail"] = serde_json::Value::String(d.to_string());
        }
        Some(RpcOutcome::new(body, vec![summary.to_string()]))
    };

    match outcome {
        // Pass only when the endpoint returns a usable vector.
        EmbedProbe::Returned(vectors)
            if vectors.first().map(|v| !v.is_empty()).unwrap_or(false) =>
        {
            None
        }
        // Reachable but produced no usable vector — not a valid embedder.
        EmbedProbe::Returned(_) => reject(
            "EMBEDDINGS_VERIFICATION_FAILED",
            "The embeddings endpoint responded but returned no vector. Choose an \
             embeddings-capable provider or endpoint, then save again.",
            "test embed returned no vectors — not saved",
            None,
        ),
        EmbedProbe::Failed(detail) => {
            let lower = detail.to_ascii_lowercase();
            // Reachable but no model loaded (e.g. LM Studio idle).
            if lower.contains("no models loaded") {
                reject(
                    "EMBEDDINGS_NO_MODEL_LOADED",
                    "Your local embeddings server (e.g. LM Studio) is running but has no \
                     model loaded. Load an embedding model — in LM Studio use the developer \
                     page or the `lms load` command — then save again.",
                    "embeddings server has no model loaded — not saved",
                    Some(&detail),
                )
            } else if crate::core::observability::is_embedding_endpoint_absent(&lower) {
                // Endpoint exposes no embeddings API (404/405).
                reject(
                    "EMBEDDINGS_ENDPOINT_NO_API",
                    "This endpoint has no embeddings API. Choose an embeddings-capable \
                     provider (Managed, Voyage, OpenAI, Cohere, Ollama) or a different \
                     custom endpoint.",
                    "embeddings endpoint has no embeddings API — not saved",
                    Some(&detail),
                )
            } else {
                // Any other failure (5xx, auth, network) — didn't pass verification.
                reject(
                    "EMBEDDINGS_VERIFICATION_FAILED",
                    "Couldn't verify the embeddings endpoint — the test embed failed. Make \
                     sure the endpoint is reachable and serving an embedding model, then \
                     save again.",
                    "embeddings endpoint failed verification — not saved",
                    Some(&detail),
                )
            }
        }
        EmbedProbe::TimedOut => reject(
            "EMBEDDINGS_VERIFICATION_FAILED",
            "Couldn't verify the embeddings endpoint — the test embed timed out. Make sure \
             the endpoint is running and reachable, then save again.",
            "embeddings endpoint timed out during verification — not saved",
            None,
        ),
    }
}

/// GET `{endpoint}/models` (OpenAI-compatible) and return the served model ids.
/// Time-boxed and best-effort — any failure returns `Err` and the caller falls
/// back to the live test-embed probe (issue #3761).
async fn fetch_served_model_ids(endpoint: &str, api_key: &str) -> Result<Vec<String>, String> {
    #[derive(serde::Deserialize)]
    struct ModelEntry {
        id: String,
    }
    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        #[serde(default)]
        data: Vec<ModelEntry>,
    }

    let url = format!("{}/models", endpoint.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let mut req = client.get(&url).timeout(std::time::Duration::from_secs(5));
    if !api_key.trim().is_empty() {
        req = req.bearer_auth(api_key.trim());
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("models request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("models request returned status {}", resp.status()));
    }
    let parsed: ModelsResponse = resp
        .json()
        .await
        .map_err(|e| format!("models parse failed: {e}"))?;
    Ok(parsed.data.into_iter().map(|m| m.id).collect())
}

/// Normalize an embedding model id for tolerant *suggestion* matching:
/// lowercase, drop a leading `text-embedding-`, drop a trailing `:tag`. Used
/// only to suggest the right served name — never to silently rewrite the id.
fn normalize_embed_model_id(name: &str) -> String {
    let lower = name.trim().to_ascii_lowercase();
    let stripped = lower.strip_prefix("text-embedding-").unwrap_or(&lower);
    stripped.split(':').next().unwrap_or(stripped).to_string()
}

/// Decide whether the requested model is acceptable given the endpoint's served
/// list. Returns `Some(reject)` only when the endpoint reports a non-empty list
/// that does NOT contain the requested id — i.e. we have positive evidence the
/// model isn't loaded. An empty/unknown list returns `None` (defer to the live
/// test-embed probe) so we never block on a server that doesn't expose
/// `/models` (issue #3761).
fn check_requested_model_served(
    requested: &str,
    served: &[String],
) -> Option<RpcOutcome<serde_json::Value>> {
    if served.is_empty() || served.iter().any(|m| m == requested) {
        return None;
    }
    Some(reject_model_not_served(requested, served))
}

/// Build the "model not served" rejection: names what the endpoint actually
/// serves and, when a normalized match exists, suggests the exact name to pick
/// (e.g. `bge-m3` → `text-embedding-bge-m3`). Reuses the
/// `EMBEDDINGS_NO_MODEL_LOADED` error code so the existing Embeddings setup
/// dialog surfaces `message` and keeps the config unsaved (issue #3761).
fn reject_model_not_served(requested: &str, served: &[String]) -> RpcOutcome<serde_json::Value> {
    let want = normalize_embed_model_id(requested);
    let suggestion = served
        .iter()
        .find(|m| normalize_embed_model_id(m) == want)
        .cloned();
    let served_list = served.join(", ");
    let message = match suggestion.as_deref() {
        Some(s) => format!(
            "`{requested}` isn't loaded on this embeddings server — but the same model appears to be served as `{s}`. Select `{s}` (the exact name your server reports), then save again. Available models: {served_list}."
        ),
        None => format!(
            "`{requested}` isn't loaded on this embeddings server. Select one of the loaded models (the exact name your server reports), then save again. Available models: {served_list}."
        ),
    };
    let mut body = serde_json::json!({
        "error": "EMBEDDINGS_NO_MODEL_LOADED",
        "message": message,
        "requested_model": requested,
        "available_models": served,
    });
    if let Some(s) = suggestion {
        body["suggested_model"] = serde_json::Value::String(s);
    }
    RpcOutcome::new(
        body,
        vec!["embedding model not served by endpoint — not saved".to_string()],
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// The seam the memory factory depends on (TAURI-RUST-52S fix): the three
    /// `create_memory_with_local_ai` call sites resolve the user's stored BYO
    /// embedding credential via `resolve_api_key` and thread it into the
    /// provider. If this lookup silently returns "" for a configured key —
    /// wrong cred slug, encryption mismatch, profile-store regression — the
    /// memory pipeline reverts to sending an empty bearer and Cohere 401s on
    /// every embed. Lock the round-trip: store under `embeddings:<slug>`, read
    /// it back; an unrelated provider must stay empty (no cross-bleed).
    #[test]
    fn resolve_api_key_returns_stored_embeddings_credential() {
        let tmp = TempDir::new().unwrap();
        let mut config = Config::default();
        config.config_path = tmp.path().join("config.toml");

        // Nothing stored yet → empty (the empty-key guard's "" input).
        assert_eq!(resolve_api_key(&config, "cohere"), "");

        // Store a Cohere embeddings key exactly as `set_api_key` does.
        AuthService::from_config(&config)
            .store_provider_token(
                "embeddings:cohere",
                "default",
                "sk-cohere-test",
                HashMap::new(),
                true,
            )
            .unwrap();

        // Resolve returns it; a provider with no stored key stays empty.
        assert_eq!(resolve_api_key(&config, "cohere"), "sk-cohere-test");
        assert_eq!(resolve_api_key(&config, "voyage"), "");
    }

    /// `custom:<url>` providers must look up under the `embeddings:custom`
    /// slug (the inline URL is not part of the credential key), mirroring the
    /// slug normalization in `embed`/`set_api_key`.
    #[test]
    fn resolve_api_key_normalizes_custom_prefix_to_custom_slug() {
        let tmp = TempDir::new().unwrap();
        let mut config = Config::default();
        config.config_path = tmp.path().join("config.toml");

        AuthService::from_config(&config)
            .store_provider_token(
                "embeddings:custom",
                "default",
                "sk-custom-test",
                HashMap::new(),
                true,
            )
            .unwrap();

        assert_eq!(
            resolve_api_key(&config, "custom:http://localhost:1234"),
            "sk-custom-test"
        );
    }

    #[test]
    fn normalize_embed_model_id_strips_prefix_and_tag() {
        assert_eq!(normalize_embed_model_id("text-embedding-bge-m3"), "bge-m3");
        assert_eq!(normalize_embed_model_id("bge-m3"), "bge-m3");
        assert_eq!(normalize_embed_model_id("bge-m3:latest"), "bge-m3");
        assert_eq!(normalize_embed_model_id("TEXT-EMBEDDING-BGE-M3"), "bge-m3");
        // Exact-after-strip: must not collapse a different model onto bge-m3.
        assert_ne!(normalize_embed_model_id("bge-m3-distill"), "bge-m3");
    }

    #[test]
    fn reject_model_not_served_suggests_normalized_match() {
        // User entered `bge-m3`; LM Studio serves `text-embedding-bge-m3` —
        // the feedback names the exact served id to select (issue #3761).
        let served = vec!["text-embedding-bge-m3".to_string(), "qwen-chat".to_string()];
        let out = reject_model_not_served("bge-m3", &served);
        assert_eq!(out.value["error"], "EMBEDDINGS_NO_MODEL_LOADED");
        assert_eq!(out.value["suggested_model"], "text-embedding-bge-m3");
        let msg = out.value["message"].as_str().unwrap();
        assert!(msg.contains("text-embedding-bge-m3"));
    }

    #[test]
    fn reject_model_not_served_without_match_lists_available() {
        let served = vec!["qwen-chat".to_string(), "llama-3".to_string()];
        let out = reject_model_not_served("bge-m3", &served);
        assert_eq!(out.value["error"], "EMBEDDINGS_NO_MODEL_LOADED");
        assert!(out.value.get("suggested_model").is_none());
        let msg = out.value["message"].as_str().unwrap();
        assert!(msg.contains("qwen-chat") && msg.contains("llama-3"));
    }

    #[test]
    fn check_requested_model_served_decisions() {
        // Served exactly → accept (None).
        assert!(check_requested_model_served(
            "text-embedding-bge-m3",
            &["text-embedding-bge-m3".to_string()],
        )
        .is_none());
        // Empty/unknown list → defer to probe (None), never block.
        assert!(check_requested_model_served("bge-m3", &[]).is_none());
        // Non-empty list without the model → reject with feedback.
        let reject = check_requested_model_served("bge-m3", &["text-embedding-bge-m3".to_string()]);
        assert_eq!(reject.unwrap().value["error"], "EMBEDDINGS_NO_MODEL_LOADED");
    }

    #[tokio::test]
    async fn fetch_served_model_ids_parses_openai_models_list() {
        use axum::{routing::get, Json, Router};
        let app = Router::new().route(
            "/v1/models",
            get(|| async {
                Json(serde_json::json!({
                    "object": "list",
                    "data": [
                        { "id": "text-embedding-bge-m3", "object": "model" },
                        { "id": "qwen-chat", "object": "model" }
                    ]
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let ids = fetch_served_model_ids(&format!("{base}/v1"), "")
            .await
            .expect("models list");
        assert_eq!(ids, vec!["text-embedding-bge-m3", "qwen-chat"]);
    }

    /// Helper: pull the `error` code out of a reject payload.
    fn reject_code(outcome: EmbedProbe) -> Option<String> {
        classify_embed_probe(outcome).map(|rpc| {
            rpc.value
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        })
    }

    /// A usable vector is the ONLY thing that passes the setup-time gate — the
    /// config is then accepted and persisted.
    #[test]
    fn classify_embed_probe_accepts_only_usable_vector() {
        assert!(
            classify_embed_probe(EmbedProbe::Returned(vec![vec![0.1, 0.2, 0.3]])).is_none(),
            "a non-empty vector must verify the endpoint"
        );
    }

    /// Reachable but empty/zero-dim response is a failed verification, not a
    /// valid embedder — never persist it.
    #[test]
    fn classify_embed_probe_rejects_empty_vectors() {
        assert_eq!(
            reject_code(EmbedProbe::Returned(vec![])).as_deref(),
            Some("EMBEDDINGS_VERIFICATION_FAILED")
        );
        assert_eq!(
            reject_code(EmbedProbe::Returned(vec![vec![]])).as_deref(),
            Some("EMBEDDINGS_VERIFICATION_FAILED")
        );
    }

    /// LM Studio idle ("No models loaded") must reject the save with the
    /// one-step remediation code so the doomed config is never persisted — the
    /// fix is verifying at setup, not suppressing the later flood.
    #[test]
    fn classify_embed_probe_rejects_no_model_loaded() {
        let body = r#"Embedding API error (400 Bad Request): {"error":"No models loaded. Please load a model in the developer page or use the 'lms load' command."}"#;
        let rpc = classify_embed_probe(EmbedProbe::Failed(body.to_string())).unwrap();
        assert_eq!(
            rpc.value.get("error").and_then(|v| v.as_str()),
            Some("EMBEDDINGS_NO_MODEL_LOADED")
        );
        // The raw provider detail is preserved for the UI.
        assert_eq!(rpc.value.get("detail").and_then(|v| v.as_str()), Some(body));
    }

    /// A 404/405 (no `/embeddings` route) keeps its dedicated code.
    #[test]
    fn classify_embed_probe_rejects_endpoint_absent() {
        assert_eq!(
            reject_code(EmbedProbe::Failed(
                "Embedding API error (404 Not Found): no route".into()
            ))
            .as_deref(),
            Some("EMBEDDINGS_ENDPOINT_NO_API")
        );
    }

    /// Any other failure (5xx/auth/network) and timeouts both reject — the
    /// endpoint didn't prove it can embed, so we don't accept it.
    #[test]
    fn classify_embed_probe_rejects_other_failures_and_timeout() {
        assert_eq!(
            reject_code(EmbedProbe::Failed(
                "Embedding API error (500 Internal Server Error): boom".into()
            ))
            .as_deref(),
            Some("EMBEDDINGS_VERIFICATION_FAILED")
        );
        assert_eq!(
            reject_code(EmbedProbe::Failed("connection refused".into())).as_deref(),
            Some("EMBEDDINGS_VERIFICATION_FAILED")
        );
        assert_eq!(
            reject_code(EmbedProbe::TimedOut).as_deref(),
            Some("EMBEDDINGS_VERIFICATION_FAILED")
        );
    }
}
