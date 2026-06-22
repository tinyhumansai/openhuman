use serde::Serialize;

use super::super::openai_codex::{
    openai_codex_client_version, openai_codex_user_agent, resolve_openai_codex_routing,
    OpenAiCodexRouting, OPENAI_CODEX_ACCOUNT_HEADER, OPENAI_CODEX_MODEL_HINTS,
    OPENAI_CODEX_ORIGINATOR, OPENAI_CODEX_ORIGINATOR_HEADER,
};
use super::sanitize::sanitize_api_error;

#[derive(Debug, Serialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
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
        .ok_or_else(|| format!("no cloud provider with id or slug '{}' found", provider_id))?;

    let looked_up =
        crate::openhuman::inference::provider::factory::lookup_key_for_slug(&entry.slug, config)
            .unwrap_or_default();
    let api_key = resolve_local_runtime_key(&entry.slug, looked_up, config);

    let routing = resolve_openai_codex_routing(config, &entry.slug, &entry.endpoint, &api_key)
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

#[cfg(test)]
mod tests {
    use super::resolve_local_runtime_key;
    use crate::openhuman::config::Config;

    #[test]
    fn omlx_key_falls_back_to_local_ai_api_key() {
        let mut config = Config::default();
        config.local_ai.api_key = Some("  sk-omlx-list  ".into());
        assert_eq!(
            resolve_local_runtime_key("omlx", String::new(), &config),
            "sk-omlx-list"
        );
    }

    #[test]
    fn looked_up_key_wins_over_local_ai() {
        let mut config = Config::default();
        config.local_ai.api_key = Some("sk-local".into());
        assert_eq!(
            resolve_local_runtime_key("omlx", "from-profiles".into(), &config),
            "from-profiles"
        );
    }

    #[test]
    fn non_omlx_slug_does_not_fall_back() {
        let mut config = Config::default();
        config.local_ai.api_key = Some("sk-local".into());
        assert_eq!(
            resolve_local_runtime_key("ollama", String::new(), &config),
            ""
        );
    }
}
