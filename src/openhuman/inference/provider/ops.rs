use super::*;

use serde::Serialize;
use std::path::PathBuf;

use super::openai_codex::{
    openai_codex_client_version, openai_codex_user_agent, resolve_openai_codex_routing,
    OpenAiCodexRouting, OPENAI_CODEX_ACCOUNT_HEADER, OPENAI_CODEX_MODEL_HINTS,
    OPENAI_CODEX_ORIGINATOR, OPENAI_CODEX_ORIGINATOR_HEADER,
};

const MAX_API_ERROR_CHARS: usize = 200;

/// Fixed id for the single inference backend (OpenHuman API).
pub const INFERENCE_BACKEND_ID: &str = "openhuman";

#[derive(Debug, Clone)]
pub struct ProviderRuntimeOptions {
    pub auth_profile_override: Option<String>,
    pub openhuman_dir: Option<PathBuf>,
    pub secrets_encrypt: bool,
    pub reasoning_enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
}

pub async fn list_configured_models(
    provider_id: &str,
) -> Result<crate::rpc::RpcOutcome<serde_json::Value>, String> {
    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| e.to_string())?;

    list_configured_models_from_config(provider_id, &config).await
}

async fn list_configured_models_from_config(
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
        .ok_or_else(|| format!("no cloud provider with id or slug '{}' found", provider_id))?;

    let api_key =
        crate::openhuman::inference::provider::factory::lookup_key_for_slug(&entry.slug, config)
            .unwrap_or_default();
    let api_key = api_key.trim().to_string();

    let routing = resolve_openai_codex_routing(config, &entry.slug, &entry.endpoint, &api_key)
        .unwrap_or_else(|err| {
            log::warn!(
                "[providers][list_models] openai codex routing unavailable; continuing with configured endpoint: {err}"
            );
            OpenAiCodexRouting::standard(&entry.endpoint)
        });

    let mut models_url = format!("{}/models", routing.endpoint);
    if routing.using_oauth {
        models_url =
            append_query_param(&models_url, "client_version", openai_codex_client_version());
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
    if is_openrouter_provider(&entry) {
        validate_openrouter_api_key(&client, &routing.endpoint, &api_key).await?;
    }

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
            if !api_key.is_empty() {
                request.header("Authorization", format!("Bearer {}", api_key))
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
        // A 404 from the /models endpoint means the provider does not support model
        // listing — this is expected for many OpenAI-compatible providers (e.g. DeepSeek,
        // Moonshot, Kimi, custom proxies). Return an empty model list so the caller can
        // proceed normally instead of surfacing a spurious error / Sentry event.
        // (Sentry issue TAURI-RUST-1Z — 819 events from this path alone.)
        if status == reqwest::StatusCode::NOT_FOUND {
            log::debug!(
                "[providers][list_models] slug={} returned 404 — provider does not support /models listing; returning empty list",
                entry.slug
            );
            return Ok(crate::rpc::RpcOutcome::new(
                serde_json::json!({ "models": serde_json::Value::Array(vec![]), "unsupported": true }),
                vec!["provider does not support model listing (404)".to_string()],
            ));
        }

        let body = response.text().await.unwrap_or_default();
        let sanitized = sanitize_api_error(&body);
        let truncated = crate::openhuman::util::truncate_with_ellipsis(&sanitized, 300);
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
///    `{"object":"error","data":{…}}`, `{"data":null}`, or similar
///    non-array. The error names the actual JSON type so triage knows what
///    the provider sent.
/// 3. **Non-object top-level body** — provider returned a bare array,
///    string, etc. Caught explicitly so the parser doesn't silently
///    drop into the missing-data arm with a `<non-object>` keys list.
///
/// Per-entry parsing ignores entries that don't have a usable string id/slug
/// (lax on purpose — many OpenAI-compatible servers include malformed rows for
/// capabilities they don't fully implement).
fn parse_models_response(body: &serde_json::Value) -> Result<Vec<ModelInfo>, String> {
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

/// Synthesize a transient [`CloudProviderCreds`] entry for the well-known
/// local-runtime slugs (`ollama`, `lmstudio`) so [`list_configured_models`]
/// can probe their OpenAI-compatible `/v1/models` endpoint even when the
/// user has not registered a matching `cloud_providers` row.
///
/// Background: the AI settings panel registers an `ollama` `cloud_providers`
/// entry when the user configures Ollama (see comment on
/// [`crate::openhuman::config::schema::cloud_providers::is_slug_reserved`]),
/// but in practice some users hit
/// `inference_list_models("ollama")` without that entry — config drift,
/// flush-vs-probe race, or upgrade from a build that only persisted
/// `config.local_ai.base_url`. Sentry TAURI-RUST-28Z captures this:
/// 24 events / 7d, all `domain=rpc, method=openhuman.inference_list_models,
/// operation=invoke_method`. Without this fallback, the dropdown surfaces
/// the bare `"no cloud provider with id or slug 'ollama' found"` error
/// (also visible in the Sentry breadcrumb) instead of returning models.
///
/// Returns `None` for any slug that is not a recognized local-runtime
/// alias — callers continue down the normal "no cloud provider" error
/// path for `openai` / `anthropic` / opaque ids / typos.
fn synthesize_local_runtime_entry(
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

fn merge_openai_codex_model_hints(models: &mut Vec<ModelInfo>) {
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
            });
        }
    }
}

fn is_openrouter_provider(
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

fn append_query_param(url: &str, key: &str, value: &str) -> String {
    if let Ok(mut parsed) = reqwest::Url::parse(url) {
        parsed.query_pairs_mut().append_pair(key, value);
        return parsed.to_string();
    }

    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}{key}={value}")
}

fn model_items_from_body(body: &serde_json::Value) -> Option<Vec<serde_json::Value>> {
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
    Some(ModelInfo {
        id,
        owned_by,
        context_window,
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

impl Default for ProviderRuntimeOptions {
    fn default() -> Self {
        Self {
            auth_profile_override: None,
            openhuman_dir: None,
            secrets_encrypt: true,
            reasoning_enabled: None,
        }
    }
}

fn is_secret_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':')
}

fn token_end(input: &str, from: usize) -> usize {
    let mut end = from;
    for (i, c) in input[from..].char_indices() {
        if is_secret_char(c) {
            end = from + i + c.len_utf8();
        } else {
            break;
        }
    }
    end
}

/// Scrub known secret-like token prefixes from provider error strings.
pub fn scrub_secret_patterns(input: &str) -> String {
    const PREFIXES: [&str; 7] = [
        "sk-",
        "xoxb-",
        "xoxp-",
        "ghp_",
        "gho_",
        "ghu_",
        "github_pat_",
    ];

    let mut scrubbed = input.to_string();

    for prefix in PREFIXES {
        let mut search_from = 0;
        loop {
            let Some(rel) = scrubbed[search_from..].find(prefix) else {
                break;
            };

            let start = search_from + rel;
            let content_start = start + prefix.len();
            let end = token_end(&scrubbed, content_start);

            if end == content_start {
                search_from = content_start;
                continue;
            }

            scrubbed.replace_range(start..end, "[REDACTED]");
            search_from = start + "[REDACTED]".len();
        }
    }

    scrubbed
}

/// Sanitize API error text by scrubbing secrets and truncating length.
pub fn sanitize_api_error(input: &str) -> String {
    let scrubbed = scrub_secret_patterns(input);
    crate::openhuman::util::truncate_with_ellipsis(&scrubbed, MAX_API_ERROR_CHARS)
}

const TRANSPORT_ERROR_MAX_CHARS: usize = 1200;

/// Full `source()` chain for connection / TLS failures (scrubbed, longer than API body snippets).
pub fn format_error_chain(err: &dyn std::error::Error) -> String {
    let mut parts: Vec<String> = vec![err.to_string()];
    let mut src = std::error::Error::source(err);
    while let Some(e) = src {
        parts.push(e.to_string());
        src = std::error::Error::source(e);
    }
    let joined = parts.join(" | ");
    let scrubbed = scrub_secret_patterns(&joined);
    crate::openhuman::util::truncate_with_suffix(&scrubbed, TRANSPORT_ERROR_MAX_CHARS, "…")
}

/// Cause chain from [`anyhow::Error`] (e.g. responses fallback), scrubbed and length-limited.
pub fn format_anyhow_chain(err: &anyhow::Error) -> String {
    let joined = err
        .chain()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join(" | ");
    let scrubbed = scrub_secret_patterns(&joined);
    crate::openhuman::util::truncate_with_suffix(&scrubbed, TRANSPORT_ERROR_MAX_CHARS, "…")
}

/// Whether a non-2xx provider response is worth reporting to Sentry.
///
/// Transient upstream statuses — 429 Too Many Requests, 408 Request Timeout,
/// and 502/503/504 gateway-layer failures — are caller-side throttling or
/// upstream-capacity signals. The reliable-provider layer already retries
/// with backoff and falls back across providers/models, and the aggregate
/// "all providers exhausted" event still fires if every attempt fails.
/// Reporting each individual transient failure floods Sentry (see
/// OPENHUMAN-TAURI-6Y / 2E / 84 / T: thousands of events/day per user from
/// a single upstream rate-limit / outage window). Callers should still
/// propagate the error so retry and fallback logic runs unchanged; this
/// only gates the per-attempt Sentry report.
pub fn should_report_provider_http_failure(status: reqwest::StatusCode) -> bool {
    !crate::core::observability::TRANSIENT_PROVIDER_HTTP_STATUSES.contains(&status.as_u16())
}

/// Whether a provider non-2xx response is a deterministic budget-exhausted
/// user-state error that should be demoted from Sentry to an info log.
pub(super) fn is_budget_exhausted_http_400(status: reqwest::StatusCode, body: &str) -> bool {
    status == reqwest::StatusCode::BAD_REQUEST && super::is_budget_exhausted_message(body)
}

/// Whether a custom OpenAI-compatible proxy returned the known generic
/// upstream 400 envelope:
/// `{"error":{"message":"Bad request to upstream provider","type":"upstream_error","status":400}}`.
///
/// This shape is deterministic provider/user-state (endpoint-model mismatch,
/// unsupported schema, provider-side validation) and does not provide
/// actionable signal for OpenHuman Sentry triage.
pub(super) fn is_custom_openai_upstream_bad_request_http_400(
    provider: &str,
    status: reqwest::StatusCode,
    body: &str,
) -> bool {
    if provider != "custom_openai" || status != reqwest::StatusCode::BAD_REQUEST {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    lower.contains("bad request to upstream provider") && lower.contains("upstream_error")
}

/// Whether a provider non-2xx response is a deterministic provider-policy
/// denial (not a product bug) that should be demoted from Sentry.
///
/// Canonical example: Kimi's coding endpoint rejects non-agent clients with
/// HTTP 403 + `access_terminated_error` and a message like:
/// "currently only available for Coding Agents …".
pub(super) fn is_provider_access_policy_denied_http_403(
    status: reqwest::StatusCode,
    body: &str,
) -> bool {
    if status != reqwest::StatusCode::FORBIDDEN {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    lower.contains("access_terminated_error")
        || lower.contains("currently only available for coding agents")
}

pub(super) fn log_budget_exhausted_http_400(
    operation: &str,
    provider: &str,
    model: Option<&str>,
    status: reqwest::StatusCode,
) {
    tracing::info!(
        domain = "llm_provider",
        operation = operation,
        provider = provider,
        model = model.unwrap_or(""),
        status = status.as_u16(),
        failure = "non_2xx",
        kind = "budget",
        "[llm_provider] {operation} budget-exhausted 400 — not reporting to Sentry"
    );
}

pub(super) fn log_custom_openai_upstream_bad_request_http_400(
    operation: &str,
    provider: &str,
    model: Option<&str>,
    status: reqwest::StatusCode,
) {
    tracing::info!(
        domain = "llm_provider",
        operation = operation,
        provider = provider,
        model = model.unwrap_or(""),
        status = status.as_u16(),
        failure = "non_2xx",
        kind = "provider_user_state",
        reason = "custom_openai_upstream_bad_request",
        "[llm_provider] {operation} custom_openai upstream 400 — not reporting to Sentry"
    );
}

pub(super) fn log_provider_access_policy_denied_http_403(
    operation: &str,
    provider: &str,
    model: Option<&str>,
    status: reqwest::StatusCode,
) {
    tracing::info!(
        domain = "llm_provider",
        operation = operation,
        provider = provider,
        model = model.unwrap_or(""),
        status = status.as_u16(),
        failure = "non_2xx",
        kind = "provider_access_policy",
        "[llm_provider] {operation} provider access-policy 403 — not reporting to Sentry"
    );
}

/// Whether a provider non-2xx response is a deterministic
/// **configuration-rejection** user-state error (unknown model id,
/// abstract tier leaked to a custom provider, model-specific temperature
/// constraint) that should be demoted from Sentry to an info log.
///
/// Provider-aware (inverted polarity vs. the 401/403 backend rule): for
/// most config-rejection phrases the same body from the OpenHuman
/// **backend** stays Sentry-actionable — that would mean we sent our own
/// backend a bad request (a regression, e.g. #2079). Restricted to the
/// observed shapes (400 invalid-param / unknown-model, 404
/// model-does-not-exist, 422 unprocessable); 408/429 are transient and
/// handled separately.
///
/// **Exception: OpenAI-compatible "unknown model"** (`Model 'X' is not
/// available. Use GET /openai/v1/models …`). The OpenHuman backend now
/// emits this exact body for user-configured unknown model ids, so it is
/// user-state regardless of provider — the polarity guard is dropped for
/// this specific shape (TAURI-RUST-2Z1). See
/// [`super::is_openai_compatible_unknown_model_message`].
pub(super) fn is_provider_config_rejection_http(
    status: reqwest::StatusCode,
    provider: &str,
    body: &str,
) -> bool {
    if !matches!(status.as_u16(), 400 | 404 | 422) {
        return false;
    }
    if !super::is_provider_config_rejection_message(body) {
        return false;
    }
    // OpenAI-compatible "unknown model" body is user-state regardless of
    // provider — both third-party `custom_openai` upstreams and our own
    // OpenHuman backend now emit it for user-configured model ids that
    // aren't in the registry (TAURI-RUST-2Z1).
    if super::is_openai_compatible_unknown_model_message(body) {
        return true;
    }
    // Remaining config-rejection phrases (DeepSeek `supported api model
    // names are`, Moonshot `invalid temperature`, litellm envelopes, …)
    // are intrinsically scoped to third-party providers — keep the
    // polarity guard so a regression where our own backend emits one of
    // those still reaches Sentry.
    provider != openhuman_backend::PROVIDER_LABEL
}

pub(super) fn log_provider_config_rejection(
    operation: &str,
    provider: &str,
    model: Option<&str>,
    status: reqwest::StatusCode,
) {
    tracing::info!(
        domain = "llm_provider",
        operation = operation,
        provider = provider,
        model = model.unwrap_or(""),
        status = status.as_u16(),
        failure = "non_2xx",
        kind = "provider_config_rejection",
        "[llm_provider] {operation} provider config-rejection ({status}) — \
         user model/param configuration, not reporting to Sentry"
    );
}

/// Whether a provider error body indicates the request exceeded the model's
/// context window (the conversation/prompt is too long for the configured
/// model). This is a deterministic user-state / usage condition — the
/// remediation is "start a new chat, trim the conversation, or pick a
/// larger-context model" — not a product bug. Sentry has no signal to act
/// on.
///
/// Single source of truth for the context-overflow phrasing, shared by:
/// - [`super::reliable`]'s non-retryable classifier (retrying the same
///   oversized request can't help),
/// - the [`api_error`] Sentry-suppression cascade (below), and
/// - the `core::observability` `ContextWindowExceeded` classifier (which
///   catches the higher-layer re-report under `domain=agent` /
///   `web_channel`).
///
/// Status-agnostic on purpose: providers disagree on the HTTP code for this
/// condition — OpenAI / most emit `400 context_length_exceeded`, but some
/// custom / self-hosted gateways mis-report it as `500` (Sentry
/// TAURI-RUST-501: `"custom API error (500 …): Context size has been
/// exceeded."`). Matching on the body keeps all of them in one bucket.
///
/// Anchoring is deliberately two-tier because this matcher now also feeds
/// `core::observability::expected_error_kind` (Sentry suppression) and the
/// `reliable` non-retryable decision, so an over-broad match would both
/// hide a real error from Sentry *and* wrongly mark a retryable error as
/// permanent:
///
/// - **Length/context phrases** ([`CONTEXT_HINTS`]) are unambiguous —
///   "context window", "context length", "prompt is too long" only describe
///   request-size overflow — so they match alone.
/// - **Token-count phrases** ([`TOKEN_HINTS`]) collide with per-minute token
///   *rate* limits ("rate limit reached … too many tokens per min"), which
///   are transient 429s that MUST stay retryable and keep reaching Sentry.
///   They only count as context-overflow when no rate-limit marker is
///   present.
pub fn is_context_window_exceeded_message(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();

    // Unambiguous request-size / context phrases — match on their own.
    const CONTEXT_HINTS: &[&str] = &[
        "exceeds the context window",
        "context window of this model",
        "maximum context length",
        "context length exceeded",
        "context size has been exceeded",
        "prompt is too long",
        "input is too long",
    ];
    if CONTEXT_HINTS.iter().any(|hint| lower.contains(hint)) {
        return true;
    }

    // Token-count phrases are ambiguous with token-per-minute RATE limits.
    // Treat them as context-overflow only when the body carries no
    // rate-limit marker — otherwise a transient TPM 429 would be silenced
    // from Sentry and (via `reliable`) wrongly classified as non-retryable.
    const TOKEN_HINTS: &[&str] = &["too many tokens", "token limit exceeded"];
    if TOKEN_HINTS.iter().any(|hint| lower.contains(hint)) {
        const RATE_LIMIT_MARKERS: &[&str] = &[
            "per minute",
            "per min",
            "rate limit",
            "rate_limit",
            "tpm",
            "requests per",
            "retry after",
            "try again in",
        ];
        return !RATE_LIMIT_MARKERS
            .iter()
            .any(|marker| lower.contains(marker));
    }

    false
}

pub(super) fn log_context_window_exceeded(
    operation: &str,
    provider: &str,
    model: Option<&str>,
    status: reqwest::StatusCode,
) {
    tracing::warn!(
        domain = "llm_provider",
        operation = operation,
        provider = provider,
        model = model.unwrap_or(""),
        status = status.as_u16(),
        failure = "non_2xx",
        kind = "context_window_exceeded",
        "[llm_provider] {operation} context-window exceeded ({status}) — \
         request too long for the model, not reporting to Sentry"
    );
}

/// Whether a provider non-2xx response is the OpenHuman **backend** rejecting
/// the app session JWT (`401`/`403`). This is expected user-session state
/// (token expired / revoked / rotated server-side), not a product bug — the
/// auth domain owns recovery. `401`/`403` from **other** providers (OpenAI,
/// Anthropic, …) mean a misconfigured BYO API key and stay Sentry-actionable,
/// so the predicate is provider-scoped to [`openhuman_backend::PROVIDER_LABEL`].
pub(super) fn is_backend_auth_failure(provider: &str, status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 401 | 403) && provider == openhuman_backend::PROVIDER_LABEL
}

/// Handle a backend session-expiry auth failure: publish a
/// [`crate::core::event_bus::DomainEvent::SessionExpired`] so the credentials
/// subscriber clears the session and flips the scheduler-gate signed-out
/// override (halting downstream LLM work — see OPENHUMAN-TAURI-1T), and skip
/// the Sentry report. Mirrors the `is_auth_failure && is_backend` arm in
/// [`api_error`], factored out for the hand-rolled provider HTTP-error chains
/// in [`super::compatible::OpenAiCompatibleProvider`] which consume the
/// response body inline and so can't delegate to `api_error`. The
/// `chat_completions` chain lacked this branch and reported the backend
/// `401 Invalid token` to Sentry — that drift was TAURI-RUST-N.
///
/// `message` is the already-formatted `"{provider} API error ({status}): …"`
/// string; it embeds the sanitized body, but the prefix and caller-controlled
/// provider name aren't scrubbed, so re-run [`sanitize_api_error`] on the final
/// string before it reaches the SessionExpired subscriber's logs.
pub(super) fn publish_backend_session_expired(
    operation: &str,
    provider: &str,
    status: reqwest::StatusCode,
    message: &str,
) {
    tracing::warn!(
        domain = "llm_provider",
        operation = operation,
        provider = provider,
        status = status.as_u16(),
        "[llm_provider] backend auth failure ({status}) — publishing SessionExpired"
    );
    crate::core::event_bus::publish_global(crate::core::event_bus::DomainEvent::SessionExpired {
        source: "llm_provider.openhuman_backend".to_string(),
        reason: sanitize_api_error(message),
    });
}

/// Build a sanitized provider error from a failed HTTP response.
///
/// Reports the failure to Sentry with `provider` and `status` tags so
/// upstream LLM errors are visible in observability without every call-site
/// having to remember to log — except for:
///
/// - **Transient statuses** (429 — see [`should_report_provider_http_failure`]).
///   These get retried by the reliable-provider layer and don't deserve a
///   per-attempt Sentry event.
/// - **401/403 from the OpenHuman backend provider** — the user's app session
///   expired. That is expected user-state, not a server bug, and reporting it
///   spams Sentry (OPENHUMAN-TAURI-1T: 5,414 events from a single user whose
///   cron loops kept firing post-expiry). Instead we publish a
///   [`crate::core::event_bus::DomainEvent::SessionExpired`] so the credentials
///   subscriber clears the session and flips the scheduler-gate signed-out
///   override, halting downstream LLM work. 401/403 from **other** providers
///   (OpenAI, Anthropic, …) still go to Sentry — those mean a misconfigured
///   API key, which is actionable.
/// - **Provider config-rejection** (4xx unknown-model / abstract-tier /
///   model-specific temperature) from a **non-backend** provider — the
///   user pointed a custom provider at a model/param it doesn't accept.
///   Deterministic user-config state, surfaced in the UI; demoted to an
///   info log (#2079 / #2076 / #2202). See
///   [`is_provider_config_rejection_http`].
pub async fn api_error(provider: &str, response: reqwest::Response) -> anyhow::Error {
    let status = response.status();
    let status_str = status.as_u16().to_string();
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| "<failed to read provider error body>".to_string());
    let sanitized = sanitize_api_error(&body);
    let message = format!("{provider} API error ({status}): {sanitized}");

    let is_auth_failure = matches!(status.as_u16(), 401 | 403);
    let is_backend = provider == openhuman_backend::PROVIDER_LABEL;
    let is_budget_exhausted_user_state = is_budget_exhausted_http_400(status, &body);
    let is_custom_openai_upstream_bad_request =
        is_custom_openai_upstream_bad_request_http_400(provider, status, &body);
    let is_provider_access_policy_denied = is_provider_access_policy_denied_http_403(status, &body);
    let is_provider_config_rejection = is_provider_config_rejection_http(status, provider, &body);
    // Context-overflow is status-agnostic: match the body directly (some
    // custom gateways mis-report it as 500 — TAURI-RUST-501 — so a status
    // gate would let those through to `should_report_provider_http_failure`).
    let is_context_window_exceeded = is_context_window_exceeded_message(&body);

    if is_auth_failure && is_backend {
        // Single source of truth for backend session-expiry handling (warn +
        // SessionExpired publish + final-string sanitize) — shared with the
        // hand-rolled `chat_completions` chain in `compatible.rs`.
        publish_backend_session_expired("api_error", provider, status, &message);
    } else if is_budget_exhausted_user_state {
        log_budget_exhausted_http_400("api_error", provider, None, status);
    } else if is_custom_openai_upstream_bad_request {
        log_custom_openai_upstream_bad_request_http_400("api_error", provider, None, status);
    } else if is_provider_access_policy_denied {
        log_provider_access_policy_denied_http_403("api_error", provider, None, status);
    } else if is_provider_config_rejection {
        log_provider_config_rejection("api_error", provider, None, status);
    } else if is_context_window_exceeded {
        log_context_window_exceeded("api_error", provider, None, status);
    } else if should_report_provider_http_failure(status) {
        // Defense-in-depth: some backends (e.g. OpenHuman) wrap an upstream
        // provider 429 as an HTTP 500 with a rate-limit phrase in the body
        // (`"429 rate limit exceeded"`, `"upstream rate limit exceeded"`).
        // `should_report_provider_http_failure(500)` would otherwise let this
        // through to Sentry — suppress it here before the report fires so the
        // noise stays off Sentry (OPENHUMAN-TAURI-S: ~6 984 events).
        // The `expected_error_kind` classifier in `report_error_or_expected`
        // catches the same shape at re-report sites (agent / web_channel).
        let lower_body = body.to_ascii_lowercase();
        let is_rate_limit_body =
            crate::core::observability::is_upstream_rate_limit_message(&lower_body);
        if is_rate_limit_body {
            tracing::warn!(
                domain = "llm_provider",
                operation = "api_error",
                provider = provider,
                status = status_str.as_str(),
                "[llm_provider] api_error: skipping Sentry report — rate-limit body in \
                 non-429 response ({status})"
            );
        } else {
            crate::core::observability::report_error(
                message.as_str(),
                "llm_provider",
                "api_error",
                &[
                    ("provider", provider),
                    ("status", status_str.as_str()),
                    ("failure", "non_2xx"),
                ],
            );
        }
    }
    anyhow::anyhow!(message)
}

/// Create the inference provider.
///
/// - `inference_url`: optional custom OpenAI-compatible LLM endpoint
///   (`config.inference_url`). When set together with `api_key`, inference
///   talks directly to this URL — keeping product-backend traffic
///   (auth/billing/voice) on `backend_url` where it belongs.
/// - `backend_url`: the OpenHuman product backend URL (`config.api_url`).
///   Used by the fallback [`openhuman_backend::OpenHumanBackendProvider`]
///   which routes inference to `{backend}/openai/v1/...` with the app
///   session JWT.
/// - `api_key`: the API key for the custom inference endpoint. Ignored on
///   the OpenHuman fallback path (the backend uses a session JWT, not a
///   user-supplied key).
pub fn create_backend_inference_provider(
    inference_url: Option<&str>,
    backend_url: Option<&str>,
    api_key: Option<&str>,
    options: &ProviderRuntimeOptions,
) -> anyhow::Result<Box<dyn Provider>> {
    if let (Some(url), Some(key)) = (inference_url, api_key) {
        log::info!(
            "[providers] inference target = custom_openai @ {} (api_key bytes={})",
            url,
            key.len()
        );
        Ok(Box::new(
            crate::openhuman::inference::provider::compatible::OpenAiCompatibleProvider::new_no_responses_fallback(
                "custom_openai",
                url,
                Some(key),
                crate::openhuman::inference::provider::compatible::AuthStyle::Bearer,
            ),
        ))
    } else {
        if api_key.is_some() && inference_url.is_none() {
            log::warn!(
                "[providers] api_key provided without inference_url — key will be ignored, using OpenHuman backend"
            );
        }
        log::info!(
            "[providers] inference target = openhuman_backend (backend_url={}, inference_url_set={}, api_key_set={})",
            backend_url.unwrap_or("<default>"),
            inference_url.is_some(),
            api_key.is_some()
        );
        Ok(Box::new(openhuman_backend::OpenHumanBackendProvider::new(
            backend_url,
            options,
        )))
    }
}

/// Create provider chain with retry and fallback behavior.
pub fn create_resilient_provider(
    inference_url: Option<&str>,
    backend_url: Option<&str>,
    api_key: Option<&str>,
    reliability: &crate::openhuman::config::ReliabilityConfig,
) -> anyhow::Result<Box<dyn Provider>> {
    create_resilient_provider_with_options(
        inference_url,
        backend_url,
        api_key,
        reliability,
        &ProviderRuntimeOptions::default(),
    )
}

/// Create provider chain with retry/fallback behavior and auth runtime options.
pub fn create_resilient_provider_with_options(
    inference_url: Option<&str>,
    backend_url: Option<&str>,
    api_key: Option<&str>,
    reliability: &crate::openhuman::config::ReliabilityConfig,
    options: &ProviderRuntimeOptions,
) -> anyhow::Result<Box<dyn Provider>> {
    if !reliability.fallback_providers.is_empty() {
        tracing::warn!(
            "reliability.fallback_providers is ignored; inference uses only the OpenHuman backend"
        );
    }

    let primary_provider =
        create_backend_inference_provider(inference_url, backend_url, api_key, options)?;
    let providers: Vec<(String, Box<dyn Provider>)> =
        vec![(INFERENCE_BACKEND_ID.to_string(), primary_provider)];

    let reliable = reliable::ReliableProvider::new(
        providers,
        reliability.provider_retries,
        reliability.provider_backoff_ms,
    )
    .with_model_fallbacks(reliability.model_fallbacks.clone());

    Ok(Box::new(reliable))
}

/// Create a RouterProvider if model routes are configured, otherwise return a resilient provider.
pub fn create_routed_provider(
    inference_url: Option<&str>,
    backend_url: Option<&str>,
    api_key: Option<&str>,
    reliability: &crate::openhuman::config::ReliabilityConfig,
    model_routes: &[crate::openhuman::config::ModelRouteConfig],
    default_model: &str,
) -> anyhow::Result<Box<dyn Provider>> {
    create_routed_provider_with_options(
        inference_url,
        backend_url,
        api_key,
        reliability,
        model_routes,
        default_model,
        &ProviderRuntimeOptions::default(),
    )
}

pub fn create_routed_provider_with_options(
    inference_url: Option<&str>,
    backend_url: Option<&str>,
    api_key: Option<&str>,
    reliability: &crate::openhuman::config::ReliabilityConfig,
    model_routes: &[crate::openhuman::config::ModelRouteConfig],
    default_model: &str,
    options: &ProviderRuntimeOptions,
) -> anyhow::Result<Box<dyn Provider>> {
    if model_routes.is_empty() {
        return create_resilient_provider_with_options(
            inference_url,
            backend_url,
            api_key,
            reliability,
            options,
        );
    }

    let backend = create_backend_inference_provider(inference_url, backend_url, api_key, options)?;
    let providers: Vec<(String, Box<dyn Provider>)> =
        vec![(INFERENCE_BACKEND_ID.to_string(), backend)];

    let routes: Vec<(String, router::Route)> = model_routes
        .iter()
        .map(|r| {
            (
                r.hint.clone(),
                router::Route {
                    provider_name: INFERENCE_BACKEND_ID.to_string(),
                    model: r.model.clone(),
                    context_window:
                        crate::openhuman::inference::model_context::context_window_for_model(
                            &r.model,
                        ),
                },
            )
        })
        .collect();

    Ok(Box::new(router::RouterProvider::new(
        providers,
        routes,
        default_model.to_string(),
    )))
}

/// Create a provider with intelligent local/remote routing.
///
/// When `config.local_ai.runtime_enabled` is `true` and Ollama is reachable,
/// lightweight and medium tasks (e.g. `hint:reaction`, `hint:summarize`) are
/// served by the local model. Heavy tasks (`hint:reasoning`, `hint:agentic`,
/// `hint:coding`) always go to the remote backend. A health-gated fallback
/// transparently promotes failed local calls to the remote backend.
///
/// Telemetry for every routing decision is emitted at `INFO` level under the
/// `"routing"` tracing target.
pub fn create_intelligent_routing_provider(
    inference_url: Option<&str>,
    backend_url: Option<&str>,
    api_key: Option<&str>,
    config: &crate::openhuman::config::Config,
    options: &ProviderRuntimeOptions,
) -> anyhow::Result<Box<dyn Provider>> {
    let raw_backend =
        create_backend_inference_provider(inference_url, backend_url, api_key, options)?;
    // Wrap the raw backend in ReliableProvider so transient 502/503/504 errors
    // are retried before propagating to the agent turn. Without this, a single
    // 502 from the backend bypasses the retry layer entirely and surfaces as a
    // fatal `run_single` failure.
    log::debug!(
        "[providers] initialising reliable wrapper: retries={} backoff_ms={} fallbacks={}",
        config.reliability.provider_retries,
        config.reliability.provider_backoff_ms,
        config.reliability.model_fallbacks.len()
    );
    let reliable_backend: Box<dyn Provider> = Box::new(
        reliable::ReliableProvider::new(
            vec![(INFERENCE_BACKEND_ID.to_string(), raw_backend)],
            config.reliability.provider_retries,
            config.reliability.provider_backoff_ms,
        )
        .with_model_fallbacks(config.reliability.model_fallbacks.clone()),
    );
    let default_model = config
        .default_model
        .as_deref()
        .unwrap_or(crate::openhuman::config::DEFAULT_MODEL);

    // When the user has configured `model_routes` (custom provider via
    // BackendProviderPanel), wrap the reliable remote in a RouterProvider so
    // abstract tier names like `reasoning-v1` get translated to the configured
    // provider-specific model id (e.g. `gpt-5.5`) BEFORE the request leaves
    // the host. Without this step the abstract tier name would reach
    // `custom_openai` and 404. The OpenHuman backend can dispatch tier names
    // natively, so we skip the wrap when routes are empty.
    log::info!(
        "[providers] intelligent routing: model_routes_count={} default_model={} inference_url_set={}",
        config.model_routes.len(),
        default_model,
        inference_url.is_some()
    );
    let remote: Box<dyn Provider> = if config.model_routes.is_empty() {
        reliable_backend
    } else {
        let providers: Vec<(String, Box<dyn Provider>)> =
            vec![(INFERENCE_BACKEND_ID.to_string(), reliable_backend)];
        let routes: Vec<(String, router::Route)> = config
            .model_routes
            .iter()
            .map(|r| {
                (
                    r.hint.clone(),
                    router::Route {
                        provider_name: INFERENCE_BACKEND_ID.to_string(),
                        model: r.model.clone(),
                        context_window:
                            crate::openhuman::inference::model_context::context_window_for_model(
                                &r.model,
                            ),
                    },
                )
            })
            .collect();
        Box::new(router::RouterProvider::new(
            providers,
            routes,
            default_model.to_string(),
        ))
    };

    let provider = crate::openhuman::routing::new_provider(
        remote,
        &config.local_ai,
        default_model,
        &config.temperature_unsupported_models,
    );
    Ok(Box::new(provider))
}

/// Information about a supported provider for display purposes.
pub struct ProviderInfo {
    pub name: &'static str,
    pub display_name: &'static str,
    pub aliases: &'static [&'static str],
    pub local: bool,
}

/// Return known providers for display (single backend path).
pub fn list_providers() -> Vec<ProviderInfo> {
    vec![ProviderInfo {
        name: INFERENCE_BACKEND_ID,
        display_name: "OpenHuman (backend)",
        aliases: &["backend", "openhuman-backend"],
        local: false,
    }]
}

// Legacy provider alias stubs (integrations / config); remote providers were removed.
pub fn is_glm_alias(_name: &str) -> bool {
    false
}
pub fn is_zai_alias(_name: &str) -> bool {
    false
}
pub fn is_minimax_alias(_name: &str) -> bool {
    false
}
pub fn is_moonshot_alias(_name: &str) -> bool {
    false
}
pub fn is_qianfan_alias(_name: &str) -> bool {
    false
}
pub fn is_qwen_alias(_name: &str) -> bool {
    false
}
pub fn is_qwen_oauth_alias(_name: &str) -> bool {
    false
}
pub fn canonical_china_provider_name(_name: &str) -> Option<&'static str> {
    let _ = _name;
    None
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
