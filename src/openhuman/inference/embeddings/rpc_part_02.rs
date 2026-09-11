
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

    tracing::debug!(
        provider = provider_tag,
        model,
        dims,
        "{LOG_PREFIX} test_connection starting"
    );

    let result = if let Some(endpoint) = custom_endpoint.as_deref() {
        probe_custom_embeddings(endpoint, &api_key, model).await
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
            .embed(&["connection test"])
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
    create_embedding_provider_with_config(
        config,
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
            // The probe detail is the raw endpoint response body. It can carry the
            // API key (OpenAI's 401 echoes `Incorrect API key provided: sk-…`), and
            // the frontend appends `detail` to the surfaced message — so redact any
            // key/bearer material before it ever leaves the core, for both the UI
            // and logs (#5116). The clean classified `message` is the primary text;
            // the sanitized detail only adds a self-diagnosis hint.
            body["detail"] = serde_json::Value::String(redact_secrets(d));
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
            // The endpoint IS reachable and correctly shaped (POST /v1/embeddings
            // with the user's model + key — verified conformant by the mock-endpoint
            // regression test). The failures below are all *distinct causes*; issue
            // #5017 was that they collapsed into one generic "test embed failed"
            // message, so a user whose endpoint works for chat couldn't tell that
            // (e.g.) their chosen model isn't an embeddings model, their key was
            // rejected, or the host was unreachable. Order matters: check the
            // specific shapes before the generic fallback.
            if lower.contains("no models loaded") {
                // Reachable but no model loaded (e.g. LM Studio idle).
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
            } else if is_embedding_dimension_mismatch(&lower) {
                // Endpoint embedded fine but returned a different vector length than
                // the (Matryoshka) size we requested — a `text-embedding-3-*` model
                // name pointed at a host that ignores the `dimensions` param.
                reject(
                    "EMBEDDINGS_DIMENSION_MISMATCH",
                    "The endpoint returned a vector with a different length than the \
                     dimensions you entered. Set dimensions to match the model's native \
                     output, then save again.",
                    "embeddings endpoint returned mismatched dimensions — not saved",
                    Some(&detail),
                )
            } else if is_embedding_model_incompatible(&lower) {
                // Reachable, authenticated embeddings API that rejected the model —
                // the user pasted a chat/reasoning model (e.g. `gpt-5-mini`) into the
                // embeddings model field. This is the #5017 reporter's exact case:
                // the same model works for chat but is not an embeddings model.
                reject(
                    "EMBEDDINGS_MODEL_INCOMPATIBLE",
                    "That model isn't an embeddings model on this endpoint. A chat model \
                     (the one that works in Chat settings) can't produce embeddings — \
                     enter an embeddings model id (e.g. text-embedding-3-small, bge-m3), \
                     then save again.",
                    "embeddings model is not an embeddings model — not saved",
                    Some(&detail),
                )
            } else if embed_error_mentions_status(&lower, 401)
                || embed_error_mentions_status(&lower, 403)
            {
                // Auth failure — key missing/wrong/lacking embeddings scope. The
                // embeddings key is stored separately from the chat BYOK key, so
                // "works for chat" does not imply the embeddings key is set.
                reject(
                    "EMBEDDINGS_AUTH_FAILED",
                    "The endpoint rejected the API key (401/403). Enter a valid key for \
                     this endpoint — note the embeddings key is stored separately from the \
                     Chat provider key — then save again.",
                    "embeddings endpoint rejected the API key — not saved",
                    Some(&detail),
                )
            } else if is_embedding_endpoint_unreachable(&lower) {
                // Transport-level failure — DNS, refused connection, TLS. The base
                // URL is wrong or the host is down.
                reject(
                    "EMBEDDINGS_ENDPOINT_UNREACHABLE",
                    "Couldn't reach the embeddings endpoint (network/DNS/connection \
                     error). Check the base URL and that the host is reachable, then save \
                     again.",
                    "embeddings endpoint unreachable — not saved",
                    Some(&detail),
                )
            } else {
                // Any other failure (5xx, unclassified) — didn't pass verification.
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
            "EMBEDDINGS_ENDPOINT_UNREACHABLE",
            "Couldn't verify the embeddings endpoint — the test embed timed out. Make sure \
             the endpoint is running and reachable, then save again.",
            "embeddings endpoint timed out during verification — not saved",
            None,
        ),
    }
}

/// Whether a lowercased embed-error detail names the given HTTP status, tolerant
/// of the wire shapes the embeddings stack emits:
///   `openai embeddings returned HTTP 401 Unauthorized: …` (tinyagents adapter)
///   `Embedding API error (401 Unauthorized): …`           (parenthesized host shape)
///   `Embedding API error 401 Unauthorized: …`             (bare-status host shape)
/// The bare-status `Embedding API error {code}` form is the one the observability
/// classifier in `src/core/observability.rs` covers; without it, setup-time
/// verification for those hosts fell through to the generic failure code (#5017).
fn embed_error_mentions_status(lower: &str, code: u16) -> bool {
    let code = code.to_string();
    lower.contains(&format!("http {code}"))
        || lower.contains(&format!("({code}"))
        || lower.contains(&format!("embedding api error {code}"))
}

/// A reachable, authenticated embeddings API that **rejected the model id** — the
/// user pointed the embeddings model field at a chat/reasoning model.
///
/// Two tiers of phrasing:
///
/// - **Strong, status-independent phrasings** unambiguously name a model that
///   can't embed. OpenAI returns *HTTP 403* "You are not allowed to generate
///   embeddings from this model" when a chat model (e.g. `gpt-4o-mini`) is used
///   as the embeddings model — a MODEL problem, not an auth problem. Because
///   `classify_embed_probe` checks this **before** the 401/403 auth branch, that
///   403 must be caught here or it falls through and misreports "enter a valid
///   key" (issue #5116). None of these phrases appear in a genuine auth rejection
///   (`Incorrect API key provided …`), so matching them ahead of auth is safe.
/// - **Weak phrasings** (a stray "does not exist" / odd model-name format) are
///   only unambiguous alongside a 400/422 bad-request, so a genuine 5xx or an
///   oversized-input 400 still falls through to the generic failure (issue #5017).
fn is_embedding_model_incompatible(lower: &str) -> bool {
    let strong_model_rejection = lower.contains("not allowed to generate embeddings")
        || lower.contains("does not support embeddings")
        || lower.contains("not an embedding model")
        || lower.contains("is not an embedding")
        || lower.contains("not supported for embeddings")
        || (lower.contains("unsupported") && lower.contains("embedding"));
    if strong_model_rejection {
        return true;
    }
    let bad_request =
        embed_error_mentions_status(lower, 400) || embed_error_mentions_status(lower, 422);
    bad_request
        && (lower.contains("does not exist") || lower.contains("unexpected model name format"))
}

/// Strip API-key / bearer-token material from any text before it reaches the UI
/// or logs. Matches OpenAI-style keys (`sk-…`, including the modern `sk-proj-…`
/// form with embedded hyphens/underscores) and `Bearer <token>` headers, and
/// replaces each **whole** match — the replacements deliberately contain no `sk-`
/// substring, so not even a key *prefix* can surface (#5116).
fn redact_secrets(input: &str) -> String {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static SK_KEY_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\bsk-[A-Za-z0-9_-]+").unwrap());
    static BEARER_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]+").unwrap());
    let redacted = SK_KEY_RE.replace_all(input, "[redacted-key]");
    BEARER_RE
        .replace_all(&redacted, "Bearer [redacted]")
        .into_owned()
}

/// The post-response length guard fired: the endpoint embedded but returned a
/// vector whose length differs from the requested (Matryoshka) `dimensions`.
/// Canonical shape from the tinyagents adapter:
/// `openai embed dimension mismatch: expected 1024, got 3072`.
fn is_embedding_dimension_mismatch(lower: &str) -> bool {
    lower.contains("dimension mismatch")
}

/// A transport-level failure (DNS, refused connection, TLS, connect timeout) —
/// the endpoint was never reached, so the base URL is wrong or the host is down.
/// The tinyagents adapter wraps these as
/// `openai embeddings request to <url> failed: <reqwest error>`.
fn is_embedding_endpoint_unreachable(lower: &str) -> bool {
    lower.contains("request to") && lower.contains("failed")
        || lower.contains("connection refused")
        || lower.contains("error sending request")
        || lower.contains("error trying to connect")
        || lower.contains("dns error")
        || lower.contains("failed to lookup address")
        || lower.contains("tcp connect error")
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
