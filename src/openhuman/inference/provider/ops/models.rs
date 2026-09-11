use serde::Serialize;

use super::super::openai_codex::{
    openai_codex_client_version, openai_codex_user_agent, resolve_openai_codex_routing,
    OpenAiCodexRouting, OPENAI_CODEX_ACCOUNT_HEADER, OPENAI_CODEX_MODEL_HINTS,
    OPENAI_CODEX_ORIGINATOR, OPENAI_CODEX_ORIGINATOR_HEADER,
};
use super::sanitize::sanitize_api_error;

include!("models_part_01.rs");

#[derive(Debug, Serialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// Human-readable name when the listing supplies one (the managed
    /// `?catalog=` listing does; a bare OpenAI-compatible `/models` does not).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Charged price in USD per 1M tokens, when the listing publishes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_per_1m: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_per_1m: Option<f64>,
}

/// Resolve the API key used to probe a provider's `/models` endpoint.
///
/// Cloud providers store their key in auth-profiles.json (via `lookup_key_for_slug`),
/// but the OMLX local-runtime chip persists its Bearer key in
/// `config.local_ai.api_key`. When the slug-scoped lookup comes back empty for omlx,
/// fall back to that local-runtime key so the reachability probe still sends
/// `Authorization: Bearer` (otherwise the OMLX server returns 401).
fn resolve_local_runtime_key(
    slug: &str,
    looked_up: String,
    config: &crate::openhuman::config::Config,
) -> String {
    let looked_up = looked_up.trim().to_string();
    if !looked_up.is_empty() {
        return looked_up;
    }
    if slug == "omlx" {
        return config
            .local_ai
            .api_key
            .clone()
            .unwrap_or_default()
            .trim()
            .to_string();
    }
    looked_up
}

pub async fn list_configured_models(
    provider_id: &str,
) -> Result<crate::rpc::RpcOutcome<serde_json::Value>, String> {
    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| e.to_string())?;

    list_configured_models_from_config(provider_id, &config).await
}

pub async fn list_configured_models_from_config(
    provider_id: &str,
    config: &crate::openhuman::config::Config,
) -> Result<crate::rpc::RpcOutcome<serde_json::Value>, String> {
    let provider_id = provider_id.trim().to_string();
    if provider_id.is_empty() {
        return Err("provider_id must not be empty".to_string());
    }

    log::debug!("[providers][list_models] provider_id={}", provider_id);

    // Explicit `cloud_providers` entry wins (e.g. a user-pointed remote
    // ollama box at https://ollama.example.com/v1). Falling back to the
    // local-runtime synthesis below only happens when no entry matches.
    let entry = config
        .cloud_providers
        .iter()
        .find(|e| e.id == provider_id || e.slug == provider_id)
        .cloned()
        .or_else(|| synthesize_local_runtime_entry(&provider_id, config))
        .or_else(|| synthesize_managed_entry(&provider_id))
        .ok_or_else(|| format!("no cloud provider with id or slug '{}' found", provider_id))?;

    let looked_up =
        crate::openhuman::inference::provider::factory::lookup_key_for_slug(&entry.slug, config)
            .unwrap_or_default();
    let api_key = resolve_local_runtime_key(&entry.slug, looked_up, config);

    let bearer_is_oauth = entry.slug == "openai"
        && crate::openhuman::inference::provider::factory::openai_bearer_is_oauth(config);
    let routing =
        resolve_openai_codex_routing(config, &entry.slug, &entry.endpoint, &api_key, bearer_is_oauth)
            .unwrap_or_else(|err| {
            log::warn!(
                "[providers][list_models] openai codex routing unavailable; continuing with configured endpoint: {err}"
            );
            OpenAiCodexRouting::standard(&entry.endpoint)
        });

    let mut models_url = format!("{}/models", routing.endpoint);
    let codex_client_version = routing.using_oauth.then(openai_codex_client_version);
    if let Some(client_version) = codex_client_version.as_deref() {
        models_url = append_query_param(&models_url, "client_version", client_version);
    }

    log::debug!(
        "[providers][list_models] fetching url={} slug={} codex_oauth={} account_id_header={}",
        models_url,
        entry.slug,
        routing.using_oauth,
        routing.account_id.is_some()
    );

    let client = crate::openhuman::config::build_runtime_proxy_client_with_timeouts(
        "providers.list_models",
        30,
        10,
    );

    use crate::openhuman::config::schema::cloud_providers::AuthStyle;

    // Managed backend (`openhuman`) needs a different URL *and* a different
    // credential than every BYOK provider above, so neither `entry.endpoint`
    // nor `lookup_key_for_slug` is usable here:
    //
    //   * the seeded entry's endpoint is the *chat* base
    //     (`https://api.openhuman.ai/v1`), so `{endpoint}/models` would probe
    //     the wrong host entirely — the hosted API is resolved by
    //     `effective_backend_api_url`, which also ignores an `api_url`
    //     override pointing at a local/third-party inference host.
    //   * the session JWT lives in the `app-session` auth profile, not
    //     `provider:openhuman`, so `lookup_key_for_slug` returns "" and the
    //     request would go out unauthenticated (401).
    //
    // `?catalog=openrouter` is required: without it the backend returns only
    // the curated tier list (chat-v1, reasoning-v1, ...), which is deliberately
    // byte-identical to the legacy payload. The catalog listing is gated
    // server-side by OPENROUTER_PASSTHROUGH_ENABLED and returns an empty set
    // when the passthrough is off, so this degrades to "no models" rather than
    // an error on a backend that has not enabled it.
    let mut managed_token = String::new();
    if entry.auth_style == AuthStyle::OpenhumanJwt {
        // Do NOT propagate a missing session: a self-hosted entry may carry a
        // provider-scoped key instead, and the auth arm below documents that
        // fallback. Only fail when neither credential exists, so the error the
        // caller sees names the real problem.
        //
        // Classify the session directly rather than going through
        // `require_live_session_token`, which flattens "signed out" and "could
        // not read the credential store" into one opaque Err. A lock timeout or
        // filesystem error is a recoverable fault the picker should surface —
        // swallowing it into a successful empty catalog hides it (review, #6206).
        // A store error still propagates; only a genuinely absent/expired
        // session degrades to an empty list.
        use crate::openhuman::security::credentials::session_support::{
            classify_session_token, load_app_session_profile, publish_local_session_expiry,
            SessionTokenCheck,
        };
        let profile = load_app_session_profile(config)?;
        match classify_session_token(profile.as_ref(), chrono::Utc::now()) {
            SessionTokenCheck::Live(token) => managed_token = token,
            // Signed out is not a provider failure. The managed catalog has
            // nothing to offer until there is a session, and managed stays
            // selectable on its automatic routing — so return an empty list
            // rather than surfacing "could not load models".
            check if api_key.is_empty() => {
                let reason = match check {
                    SessionTokenCheck::Expired => {
                        // Still announce the expiry: `require_live_session_token`
                        // did this for us before, and without it an expired token
                        // stays in the store with nothing prompting a re-auth.
                        publish_local_session_expiry("list_configured_models");
                        "session expired"
                    }
                    _ => "no session",
                };
                log::info!(
                    "[providers][list_models] managed catalog unavailable — {reason}; returning an empty list"
                );
                return Ok(crate::rpc::RpcOutcome::new(
                    serde_json::json!({ "models": Vec::<ModelInfo>::new() }),
                    vec![format!("{reason}; managed catalog is empty")],
                ));
            }
            check => {
                if matches!(check, SessionTokenCheck::Expired) {
                    publish_local_session_expiry("list_configured_models");
                }
                log::debug!(
                    "[providers][list_models] no live session; falling back to the provider-scoped key"
                );
            }
        }
        let base = crate::api::config::effective_backend_api_url(&config.api_url);
        models_url = append_query_param(
            &crate::api::config::api_url(&base, "/openai/v1/models"),
            "catalog",
            "openrouter",
        );
        log::debug!(
            "[providers][list_models] managed catalog url={}",
            models_url
        );
    }

    if is_openrouter_provider(&entry) {
        validate_openrouter_api_key(&client, &routing.endpoint, &api_key).await?;
    }

    // Whether the app session token actually made it onto the wire. It is not
    // the same question as "is `managed_token` non-empty": the credential-safety
    // guard below can decline to attach it, and a 401 on an unauthenticated
    // request must not be read as a stale session (review, #6206).
    let mut managed_session_attached = false;
    let mut request = client.get(&models_url);
    if routing.using_oauth {
        request = request
            .header(reqwest::header::USER_AGENT, openai_codex_user_agent())
            .header(OPENAI_CODEX_ORIGINATOR_HEADER, OPENAI_CODEX_ORIGINATOR);
    }

    request = match entry.auth_style {
        AuthStyle::Bearer => {
            if !api_key.is_empty() {
                let mut r = request.header("Authorization", format!("Bearer {}", api_key));
                if let Some(account_id) = routing.account_id.as_deref() {
                    r = r.header(OPENAI_CODEX_ACCOUNT_HEADER, account_id);
                }
                r
            } else {
                request
            }
        }
        AuthStyle::Anthropic => {
            let mut r = request.header("anthropic-version", "2023-06-01");
            if !api_key.is_empty() {
                r = r.header("x-api-key", &api_key);
            }
            r
        }
        AuthStyle::OpenhumanJwt => {
            // Prefer the live session JWT resolved above; fall back to a
            // provider-scoped key so a self-hosted entry that stores one still
            // authenticates.
            let token = if !managed_token.is_empty() {
                managed_token.as_str()
            } else {
                api_key.as_str()
            };
            managed_session_attached = managed_session_attaches(&managed_token, &models_url);
            // Never put a bearer credential on the wire in clear text. `https`
            // or a loopback host only — loopback stays allowed so a local
            // backend (BACKEND_URL=http://127.0.0.1:...) still works in dev.
            if !token.is_empty() && url_is_credential_safe(&models_url) {
                // Managed traffic is attributed per embedding product
                // (OpenCompany / Medulla / desktop); the generic provider client
                // does not carry it, so attach it explicitly.
                let (name, value) = crate::api::product::product_identity_header();
                request
                    .header("Authorization", format!("Bearer {}", token))
                    .header(name, value)
            } else if !token.is_empty() {
                log::warn!(
                    "[providers][list_models] refusing to send a bearer token to a non-https, non-loopback URL"
                );
                request
            } else {
                request
            }
        }
        AuthStyle::None => request,
    };

    let response = request
        .send()
        .await
        .map_err(|e| format!("[providers][list_models] HTTP request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        // A 401 from the MANAGED catalog means the caller is not signed in —
        // the stored session was rejected server-side even though its local
        // `exp` was still valid, so `require_live_session_token` handed us a
        // token the backend no longer honours. That is a signed-out state, not
        // a provider failure, and rendering it as "could not load models" put a
        // red error under managed for a user whose only problem is a stale
        // session. Managed stays selectable on its automatic routing, so return
        // an empty catalog and let the app's normal auth surfaces prompt for
        // re-authentication.
        //
        // Scoped to the managed provider on purpose: for a BYOK provider a 401
        // IS the actionable error (a wrong or revoked API key), and hiding it
        // would strand the user with a silently empty dropdown.
        if managed_401_means_signed_out(status.as_u16(), entry.auth_style, managed_session_attached)
        {
            log::info!(
                "[providers][list_models] managed catalog unavailable — backend rejected the session token (401); returning an empty list"
            );
            return Ok(crate::rpc::RpcOutcome::new(
                serde_json::json!({ "models": Vec::<ModelInfo>::new() }),
                vec!["session not accepted; managed catalog is empty".to_string()],
            ));
        }
        let body = response.text().await.unwrap_or_default();
        let sanitized = sanitize_api_error(&body);
        let truncated = crate::openhuman::util::truncate_with_ellipsis(&sanitized, 300);
        // TAURI-RUST-8X3: a 404 from the `<base>/models` probe means the
        // configured base URL does not host an OpenAI-compatible `/models`
        // listing (wrong base, a model-only proxy, or a missing `/v1`
        // suffix). Append an actionable hint so the model-dropdown probe
        // surfaces *recovery guidance* inline instead of the bare Go-style
        // `404 page not found`. The `provider returned 404` prefix is kept
        // verbatim so the `is_provider_user_state_message` classifier anchor
        // (which demotes this preventable user-state case out of Sentry)
        // still matches — see `src/core/observability.rs`.
        if status.as_u16() == 404 {
            return Err(format!(
                "provider returned 404: {} — the configured base URL does not expose a `/models` endpoint; check the provider's base URL (it usually ends in `/v1`)",
                truncated
            ));
        }
        return Err(format!(
            "provider returned {}: {}",
            status.as_u16(),
            truncated
        ));
    }

    // TAURI-RUST-12: `response.json()` discards the body when decoding fails,
    // so Sentry just sees `error decoding response body` with no clue what the
    // server actually sent. In practice the offending body is HTML from a
    // captive portal / corporate proxy login page, an upstream load-balancer
    // 502 served as HTML with a `200 OK`, or a JSON parser tripping on a
    // wrong-path endpoint. Read the body as text first, then parse, and
    // surface a sanitized + truncated snippet so the failure is diagnosable
    // from the error string alone.
    let raw_body = response.text().await.map_err(|e| {
        format!(
            "[providers][list_models] failed to read response body: {}",
            e
        )
    })?;
    let body: serde_json::Value = serde_json::from_str(&raw_body).map_err(|e| {
        let sanitized = sanitize_api_error(&raw_body);
        let snippet = crate::openhuman::util::truncate_with_ellipsis(&sanitized, 300);
        format!(
            "[providers][list_models] failed to parse JSON: {} (body: {})",
            e, snippet
        )
    })?;

    // OpenAI-compatible servers occasionally return HTTP 200 with an error
    // payload instead of a 4xx (LM Studio does this for unknown paths like
    // `/v11/models` — body `{"error":"Unexpected endpoint or method..."}`).
    // Treat any top-level `error` field as a failure so the AI-panel probe
    // doesn't silently accept a typo'd endpoint.
    if let Some(err_field) = body.get("error") {
        let msg = err_field
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| {
                err_field
                    .get("message")
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| err_field.to_string());
        let sanitized = sanitize_api_error(&msg);
        return Err(format!("provider returned error payload: {}", sanitized));
    }

    // Parse the OpenAI-compatible `/models` envelope into typed model
    // entries. See `parse_models_response` for the distinct error shapes
    // returned for "missing field" vs "field present but wrong type"
    // (TAURI-RUST-4Y). The ChatGPT Codex backend uses a sibling `models`
    // array keyed by `slug`, so that shape is accepted here too.
    let mut models = parse_models_response(&body)?;
    if routing.using_oauth {
        merge_openai_codex_model_hints(&mut models);
    }

    log::info!(
        "[providers][list_models] slug={} fetched {} models",
        entry.slug,
        models.len()
    );

    Ok(crate::rpc::RpcOutcome::new(
        serde_json::json!({ "models": models }),
        vec![format!("fetched {} models", models.len())],
    ))
}

/// Parse the OpenAI-compatible `/models` response envelope, or the ChatGPT
/// Codex backend's sibling `models` envelope, into typed [`ModelInfo`] entries.
///
/// Returns distinct errors for the three failure modes the wild has
/// produced in `inference_list_models` Sentry events:
///
/// 1. **Missing `data`/`models` field** — endpoint isn't `/models`-compatible
///    (user typo'd the base URL, pointed at a vector-DB host, etc.).
/// 2. **`data`/`models` field present but wrong type** — provider returned
///    `{"object":"error","data":{…}}` or similar non-array. The error names
///    the actual JSON type so triage knows what the provider sent.
/// 3. **Non-object top-level body** — provider returned a bare array,
///    string, etc. Caught explicitly so the parser doesn't silently
///    drop into the missing-data arm with a `<non-object>` keys list.
///
/// A **null** `data`/`models` field on a **success envelope** is NOT an
/// error — Ollama's OpenAI-compatible `/v1/models` null-encodes the catalog
/// (`{"object":"list","data":null}`) when no models are pulled, so it is
/// treated as an empty model list (TAURI-RUST-874 / TAURI-RUST-875). The
/// null-as-empty short-circuit is gated on `object` being absent or `"list"`:
/// a null `data` on an error envelope (`{"object":"error","data":null}`)
/// instead falls through to failure mode 2 so the provider error still
/// surfaces in the UI and Sentry.
///
/// Per-entry parsing ignores entries that don't have a usable string id/slug
/// (lax on purpose — many OpenAI-compatible servers include malformed rows for
/// capabilities they don't fully implement).
pub fn parse_models_response(body: &serde_json::Value) -> Result<Vec<ModelInfo>, String> {
    let obj = body.as_object().ok_or_else(|| {
        format!(
            "provider response is not a JSON object — endpoint is not OpenAI-compatible (got {} at top level)",
            json_value_kind(body)
        )
    })?;

    let (field_name, data_value) = obj
        .get("data")
        .map(|value| ("data", value))
        .or_else(|| obj.get("models").map(|value| ("models", value)))
        .ok_or_else(|| {
        let keys = obj.keys().cloned().collect::<Vec<_>>().join(", ");
        format!(
                "provider response missing `data` or `models` field — endpoint is not OpenAI-compatible (got keys: {})",
            keys
        )
    })?;

    // A null `data`/`models` field is a valid empty catalog ONLY on a success
    // envelope: Ollama's OpenAI-compatible `/v1/models` returns
    // `{"object":"list","data":null}` (object="list", the success marker) when
    // no models are pulled. Treat that as an empty model list so a healthy-but-
    // empty local runtime doesn't manufacture a hard error (TAURI-RUST-874 /
    // TAURI-RUST-875).
    //
    // An error body such as `{"object":"error","data":null}` ALSO null-encodes
    // `data`; swallowing it as an empty catalog would hide provider/endpoint
    // failures from the UI and Sentry. So gate on a success envelope: short-
    // circuit only when `object` is absent or "list". Any other `object` value
    // (e.g. "error") with null `data` falls through to the descriptive error
    // below, which surfaces the `object` value for triage. Non-array kinds
    // (object/string/number/bool) likewise fall through.
    let is_success_envelope = obj
        .get("object")
        .and_then(|value| value.as_str())
        .is_none_or(|object| object.eq_ignore_ascii_case("list"));

    if data_value.is_null() && is_success_envelope {
        log::info!(
            "[providers][list_models] `{field_name}` is null on a success envelope — provider returned an empty catalog (no models)"
        );
        return Ok(Vec::new());
    }

    let data = data_value.as_array().ok_or_else(|| {
        // Include the sibling `object` field if present — OpenAI-shaped
        // servers set it to `"list"` on success and `"error"` (or omit)
        // on failure, so its value is the fastest triage signal for
        // future Sentry events on the wrong-type arm.
        let object_field = obj
            .get("object")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "<absent>".to_string());
        format!(
            "provider response has `{}` field but it is {}, expected array — endpoint may be returning an error envelope (\"object\" = {})",
            field_name,
            json_value_kind(data_value),
            object_field,
        )
    })?;

    Ok(data
        .iter()
        .filter_map(model_info_from_catalog_item)
        .collect())
}

/// Name the JSON value kind for use in `parse_models_response` error
/// messages. Mirrors `serde_json::Value::*` variants exactly so test
/// assertions on the rendered token (`object`/`string`/`null`/…) stay
/// in lock-step with the matcher.
fn json_value_kind(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

pub fn synthesize_local_runtime_entry(
    slug: &str,
    config: &crate::openhuman::config::Config,
) -> Option<crate::openhuman::config::schema::cloud_providers::CloudProviderCreds> {
    use crate::openhuman::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};

    let endpoint = match slug {
        // Ollama's OpenAI-compatible surface at `<base>/v1/models` returns
        // the same `{"data": [...]}` shape the existing parser handles, so
        // we route through that rather than the native `/api/tags`.
        "ollama" => {
            let base = crate::openhuman::inference::local::ollama_base_url_from_config(config);
            format!("{}/v1", base.trim_end_matches('/'))
        }
        // `lm_studio_base_url` already ends in `/v1`.
        "lmstudio" => crate::openhuman::inference::local::lm_studio::lm_studio_base_url(config),
        _ => return None,
    };

    Some(CloudProviderCreds {
        id: format!("synthetic_local_{slug}"),
        slug: slug.to_string(),
        label: slug.to_string(),
        endpoint,
        // Local runtimes accept unauthenticated requests on loopback.
        // The probe at `<endpoint>/models` runs without an Authorization
        // header — `lookup_key_for_slug` may still return a key, but
        // `AuthStyle::None` ignores it (see auth-style match below).
        auth_style: AuthStyle::None,
        legacy_type: None,
        default_model: None,
    })
}

pub fn merge_openai_codex_model_hints(models: &mut Vec<ModelInfo>) {
    let mut seen = models
        .iter()
        .map(|model| model.id.to_ascii_lowercase())
        .collect::<std::collections::HashSet<_>>();

    for id in OPENAI_CODEX_MODEL_HINTS {
        if seen.insert(id.to_ascii_lowercase()) {
            models.push(ModelInfo {
                id: (*id).to_string(),
                owned_by: Some("openai-codex".to_string()),
                context_window: None,
                display_name: None,
                input_per_1m: None,
                output_per_1m: None,
            });
        }
    }
}

pub fn is_openrouter_provider(
    entry: &crate::openhuman::config::schema::cloud_providers::CloudProviderCreds,
) -> bool {
    if entry.slug.eq_ignore_ascii_case("openrouter") {
        return true;
    }

    reqwest::Url::parse(&entry.endpoint)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        .is_some_and(|host| host == "openrouter.ai" || host.ends_with(".openrouter.ai"))
}

pub fn append_query_param(url: &str, key: &str, value: &str) -> String {
    if let Ok(mut parsed) = reqwest::Url::parse(url) {
        parsed.query_pairs_mut().append_pair(key, value);
        return parsed.to_string();
    }

    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}{key}={value}")
}

#[allow(dead_code)]
pub fn model_items_from_body(body: &serde_json::Value) -> Option<Vec<serde_json::Value>> {
    body.get("data")
        .and_then(|d| d.as_array())
        .or_else(|| body.get("models").and_then(|d| d.as_array()))
        .cloned()
}

fn model_info_from_catalog_item(item: &serde_json::Value) -> Option<ModelInfo> {
    if let Some(id) = item.as_str().map(str::trim).filter(|id| !id.is_empty()) {
        return Some(ModelInfo {
            id: id.to_string(),
            owned_by: None,
            context_window: None,
            display_name: None,
            input_per_1m: None,
            output_per_1m: None,
        });
    }

    let id = item
        .get("id")
        .or_else(|| item.get("slug"))
        .or_else(|| item.get("name"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())?
        .to_string();
    let owned_by = item
        .get("owned_by")
        .or_else(|| item.get("owned_by_organization"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let context_window = item
        .get("context_length")
        .or_else(|| item.get("context_window"))
        .or_else(|| item.get("max_context_window"))
        .and_then(|v| v.as_u64());
    let display_name = item
        .get("display_name")
        .or_else(|| item.get("name"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    // `pricing` is already the CHARGED price per 1M tokens on the managed
    // catalog listing, so it can be surfaced verbatim — no margin math here.
    let pricing = item.get("pricing");
    let price = |key: &str| -> Option<f64> {
        pricing
            .and_then(|p| p.get(key))
            .and_then(|v| v.as_f64())
            .filter(|n| n.is_finite() && *n >= 0.0)
    };
    Some(ModelInfo {
        id,
        owned_by,
        context_window,
        display_name,
        input_per_1m: price("inputPer1M"),
        output_per_1m: price("outputPer1M"),
    })
}

async fn validate_openrouter_api_key(
    client: &reqwest::Client,
    base: &str,
    api_key: &str,
) -> Result<(), String> {
    if api_key.is_empty() {
        return Err("OpenRouter API key is required before enabling the provider".to_string());
    }

    let key_url = format!("{}/key", base);
    log::debug!("[providers][list_models] validating OpenRouter API key");
    let response = client
        .get(&key_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await
        .map_err(|e| format!("[providers][list_models] OpenRouter key validation failed: {e}"))?;

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let sanitized = sanitize_api_error(&text);
        let truncated = crate::openhuman::util::truncate_with_ellipsis(&sanitized, 300);
        log::debug!(
            "[providers][list_models] OpenRouter key validation failed status={} body={}",
            status.as_u16(),
            truncated
        );
        return Err(format!(
            "OpenRouter key validation returned {}: {}",
            status.as_u16(),
            truncated
        ));
    }

    if let Ok(body) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(err_field) = body.get("error") {
            let msg = err_field
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| {
                    err_field
                        .get("message")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| err_field.to_string());
            let sanitized = sanitize_api_error(&msg);
            log::debug!(
                "[providers][list_models] OpenRouter key validation returned error payload={}",
                sanitized
            );
            return Err(format!(
                "OpenRouter key validation returned error payload: {}",
                sanitized
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
#[path = "models_tests.rs"]
mod tests;
