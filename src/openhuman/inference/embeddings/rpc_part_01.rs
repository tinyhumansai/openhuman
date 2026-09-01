use std::collections::HashMap;

use crate::openhuman::config::Config;
use crate::openhuman::security::credentials::AuthService;
use crate::rpc::RpcOutcome;

use super::catalog;
use super::factory::{create_embedding_provider_with_config, model_supports_dimensions};

const LOG_PREFIX: &str = "[embeddings::rpc]";

/// Slug naming the embedder ingestion will actually use, resolved host-side
/// from the `Config` fields the resolution ladder reads.
///
/// Mirrors `tinymemory_core::tree::score::embed::effective_embedder_slug` so
/// `get_settings` no longer calls `tinymemory_core::` directly (#5560).
///
/// `MemoryScoring::embedder_slug()` is not used here for two reasons:
/// (1) `get_settings` is a synchronous config-reading RPC handler and cannot
/// await an async bus call; (2) this function answers "what slug will ingestion
/// use?" — a config-derived prediction that must work even when the module is
/// not loaded. The bus call would give the same answer when the module is
/// running, but would fail gracefully when it is not, offering no benefit over
/// reading the config directly. Keep both implementations in sync whenever the
/// engine's resolution ladder changes.
///
/// Resolution order (matches the engine factory's ladder):
/// 1. Explicit Ollama override — `memory_tree.embedding_endpoint` +
///    `memory_tree.embedding_model` both `Some` and non-empty → `"ollama"`.
/// 2. Deliberate opt-out — `embeddings_provider` trimmed equals `"none"` → `"none"`.
/// 3. Local Ollama via unified workload setting — `workload_local_model("embeddings")`
///    is `Some` → `"ollama"`.
/// 4. User OpenAI-compatible endpoint — `memory.embedding_provider` is
///    `"openai"`, `"custom"`, or starts with `"custom:"` → `"custom"`.
/// 5. Managed cloud session — `auth-profiles.json` exists next to the config
///    file → `"cloud"`.
/// 6. Nothing usable → `"unconfigured"`.
fn effective_embedder_slug_from_config(config: &Config) -> &'static str {
    // 1. Explicit Ollama override.
    if let (Some(ep), Some(model)) = (
        config.memory_tree.embedding_endpoint.as_deref(),
        config.memory_tree.embedding_model.as_deref(),
    ) {
        if !ep.trim().is_empty() && !model.trim().is_empty() {
            return "ollama";
        }
    }
    // 2. Deliberate opt-out.
    if config
        .embeddings_provider
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| s == "none")
    {
        return "none";
    }
    // 3. Local Ollama via unified workload setting.
    if config.workload_local_model("embeddings").is_some() {
        return "ollama";
    }
    // 4. User OpenAI-compatible endpoint.
    let picker = config.memory.embedding_provider.trim();
    if picker == "openai" || picker == "custom" || picker.starts_with("custom:") {
        return "custom";
    }
    // 5. Managed cloud session.
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

/// Send one OpenAI-compatible embedding request without requesting or
/// validating a vector width. This is intentionally separate from the live
/// provider: a setup probe must discover a custom endpoint's native width,
/// while a live provider must enforce the width persisted after that probe.
async fn probe_custom_embeddings(
    endpoint: &str,
    api_key: &str,
    model: &str,
) -> Result<Vec<Vec<f32>>, String> {
    let base = endpoint.trim_end_matches('/');
    let url = if base.ends_with("/embeddings") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{base}/embeddings")
    } else {
        format!("{base}/v1/embeddings")
    };
    let mut request = reqwest::Client::new()
        .post(&url)
        .json(&serde_json::json!({ "model": model, "input": ["connection test"] }));
    if !api_key.trim().is_empty() {
        request = request.bearer_auth(api_key.trim());
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("custom embeddings request to {url} failed: {e}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("custom embeddings response read failed: {e}"))?;
    if !status.is_success() {
        return Err(format!("custom embeddings returned HTTP {status}: {body}"));
    }
    let data = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| format!("custom embeddings response was not JSON: {e}"))?
        .get("data")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .ok_or_else(|| "custom embeddings response missing data array".to_string())?;
    data.into_iter()
        .map(|item| {
            item.get("embedding")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| "custom embeddings response missing embedding array".to_string())?
                .iter()
                .map(|value| value.as_f64().map(|value| value as f32).ok_or_else(|| "custom embeddings response contains a non-numeric vector".to_string()))
                .collect()
        })
        .collect()
}

/// Dimension to persist after a successful Custom verification probe.
///
/// For a `text-embedding-3-*` model the endpoint honoured the requested size,
/// so keep the user's `configured` value (Matryoshka). For every other model we
/// probed dimension-agnostically, so adopt the endpoint's actual returned
/// length (`actual`) — the user can't be expected to know it, and storing the
/// real size is what lets the live embed path's length guard pass afterwards.
/// Falls back to `configured` if the probe somehow reported a zero-length
/// vector (defensive — `classify_embed_probe` already rejects empty vectors).
fn final_probe_dims(model: &str, configured: usize, actual: usize) -> usize {
    if model_supports_dimensions(model) || actual == 0 {
        configured
    } else {
        actual
    }
}

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

    // The embedder ingestion will *actually* use. `provider` above is the
    // per-section setting the picker writes; it is NOT authoritative for how
    // embeddings are funded, because the Local AI "Memory embeddings" toggle and
    // the `memory_tree.embedding_endpoint` override both route to local Ollama
    // without rewriting it. Additive field — callers that only need the picker
    // value are unaffected; callers asking "does this bill the managed budget?"
    // must read this one (#5402).
    let effective_provider = effective_embedder_slug_from_config(config);

    let payload = serde_json::json!({
        "provider": provider,
        "effective_provider": effective_provider,
        "model": model,
        "dimensions": dimensions,
        "rate_limit_per_min": rate_limit,
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
    use crate::openhuman::inference::embeddings::format_embedding_signature;

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
    // `new_dims`/`new_sig`/`dims_changed` are recomputed after the Custom
    // verification probe auto-detects the endpoint's real vector length
    // (issue #4056), so they must be mutable.
    let mut new_dims = dimensions.unwrap_or(config.memory.embedding_dimensions);
    let mut new_sig = format_embedding_signature(&new_provider, &new_model, new_dims);

    let old_dims = config.memory.embedding_dimensions;
    let mut dims_changed = new_dims != old_dims;
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
            probe_custom_embeddings(endpoint, &api_key, &new_model),
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
                    let reject_code = reject
                        .value
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("EMBEDDINGS_VERIFICATION_FAILED");
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
                // Passed. Adopt the endpoint's real vector length for every model
                // we probed dimension-agnostically — the user can't be expected to
                // know it, and storing the actual size is what keeps the live embed
                // path's length guard from rejecting future embeds (issue #4056).
                // `text-embedding-3-*` keeps the requested size (server honoured it).
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
                    dims_changed = new_dims != old_dims;
                    sig_changed = new_sig != old_sig;
                }
                tracing::debug!(
                    provider = effective_provider.as_str(),
                    new_dims,
                    "{LOG_PREFIX} update_settings test embed passed — accepting config"
                );
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
        }
    }

    config.save().await.map_err(|e| e.to_string())?;

    if sig_changed {
        crate::openhuman::memory::ops::maintenance::reembed_best_effort(
            &config,
            "embedding settings",
        )
        .await;
    }

    // #5324: this is the exact screen the "embedding budget reached" alert
    // deep-links to, so a provider/endpoint save here is the user completing
    // the remediation. Un-park the jobs that failed under the old
    // (budget-exhausted / misconfigured) provider so memory resumes growing
    // without the user also having to find "Retry failed" in Memory Tree
    // settings.
    //
    // Gated on an actual provider/endpoint/signature touch — NOT unconditional:
    // a save that only nudges `rate_limit_per_min` does not remediate the
    // embedder, so it must leave terminally-failed jobs parked. `provider`
    // covers re-selecting the *same* provider after fixing the account behind
    // it (a legitimate remediation even when the signature is unchanged).
    let is_embedding_remediation = sig_changed || provider.is_some() || custom_endpoint.is_some();
    // #5324: the settings save has already succeeded. A failed un-park must not
    // fail the RPC, but it must be surfaced (not reported as `0`) so a queue
    // that stayed parked isn't presented as remediated.
    let requeue_result = if is_embedding_remediation {
        crate::openhuman::memory::ops::maintenance::retry_failed(&config).await
    } else {
        Ok(0)
    };
    let requeued_count = *requeue_result.as_ref().unwrap_or(&0);
    let requeue_error = requeue_result.as_ref().err().cloned();
    let requeued_note = match &requeue_error {
        None => requeued_count.to_string(),
        Some(e) => format!("error ({e})"),
    };

    tracing::info!(
        provider = config.memory.embedding_provider.as_str(),
        model = config.memory.embedding_model.as_str(),
        dimensions = config.memory.embedding_dimensions,
        sig_changed,
        requeued = requeued_count,
        requeue_error = requeue_error.as_deref().unwrap_or(""),
        "{LOG_PREFIX} update_settings applied"
    );

    let payload = serde_json::json!({
        "provider": config.memory.embedding_provider,
        "model": config.memory.embedding_model,
        "dimensions": config.memory.embedding_dimensions,
        "signature_changed": sig_changed,
        "new_signature": new_sig,
        "requeued_failed_jobs": requeued_count,
        "requeue_error": requeue_error,
    });

    Ok(RpcOutcome::new(
        payload,
        vec![format!(
            "embeddings settings updated (sig_changed={sig_changed} requeued_failed={requeued_note})"
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

    // #5324: supplying a BYO key does NOT change the embedding signature, so
    // `ensure_reembed_backfill` has nothing to enqueue — but it is precisely
    // the action that unblocks jobs parked on `budget_exhausted` /
    // `auth_missing`. Requeue them here or they stay dead until the user
    // separately discovers the "Retry failed" button. A store failure is
    // surfaced (not reported as `0`) so the key-stored response can't imply the
    // parked queue was recovered when it wasn't.
    let requeue_result = crate::openhuman::memory::ops::maintenance::retry_failed(config).await;
    let requeued_count = *requeue_result.as_ref().unwrap_or(&0);
    let requeue_error = requeue_result.as_ref().err().cloned();
    let requeued_note = match &requeue_error {
        None => requeued_count.to_string(),
        Some(e) => format!("error ({e})"),
    };

    tracing::info!(
        provider = provider_slug,
        requeued = requeued_count,
        requeue_error = requeue_error.as_deref().unwrap_or(""),
        "{LOG_PREFIX} set_api_key stored"
    );

    Ok(RpcOutcome::new(
        serde_json::json!({ "stored": true, "provider": provider_slug, "requeued_failed_jobs": requeued_count, "requeue_error": requeue_error }),
        vec![format!(
            "embedding API key stored for {provider_slug} (requeued_failed={requeued_note})"
        )],
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

    let embedder = create_embedding_provider_with_config(
        config,
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
