//! The `list_configured_models` RPC entry point: resolves a provider id to a
//! `cloud_providers` entry (falling back to synthesized local-runtime /
//! managed entries), builds an authenticated `/models` request for it, and
//! parses the response.

use super::*;

/// Resolve the API key used to probe a provider's `/models` endpoint.
///
/// Cloud providers store their key in auth-profiles.json (via `lookup_key_for_slug`),
/// but the OMLX local-runtime chip persists its Bearer key in
/// `config.local_ai.api_key`. When the slug-scoped lookup comes back empty for omlx,
/// fall back to that local-runtime key so the reachability probe still sends
/// `Authorization: Bearer` (otherwise the OMLX server returns 401).
pub(super) fn resolve_local_runtime_key(
    slug: &str,
    looked_up: String,
    config: &crate::config::Config,
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

/// The key and HTTP client a registered catalog listing is handed for `entry`
/// (when the provider id matched a configured entry).
fn catalog_request(
    config: &crate::config::Config,
    entry: Option<&crate::config::schema::cloud_providers::CloudProviderCreds>,
) -> Result<crate::inference::registry::CatalogRequest, String> {
    let service_key = "providers.list_models";
    let slug = entry.map(|e| e.slug.trim()).unwrap_or_default();
    let provider_key = match entry {
        Some(entry) => {
            let looked_up =
                crate::inference::provider::factory::lookup_key_for_slug(&entry.slug, config)
                    .unwrap_or_default();
            resolve_local_runtime_key(&entry.slug, looked_up, config)
        }
        None => String::new(),
    };
    let client = match config
        .cloud_provider_ca_certs
        .get(slug)
        .filter(|pem| !pem.is_empty())
    {
        Some(pem) => {
            crate::util::tls::client_with_ca_bundle_with_timeouts(pem, service_key, 30, 10)?
        }
        None => crate::config::build_runtime_proxy_client_with_timeouts(service_key, 30, 10),
    };
    Ok(crate::inference::registry::CatalogRequest {
        provider_key,
        client,
    })
}

/// The empty listing the managed provider answers with when no hosted catalog
/// is registered.
fn no_managed_catalog() -> crate::core::Outcome<serde_json::Value> {
    log::info!(
        "[providers][list_models] managed catalog unavailable: no catalog provider registered; returning an empty list"
    );
    crate::core::Outcome::new(
        serde_json::json!({ "models": Vec::<ModelInfo>::new() }),
        vec!["no hosted backend; managed catalog is empty".to_string()],
    )
}

pub async fn list_configured_models(
    provider_id: &str,
) -> Result<crate::core::Outcome<serde_json::Value>, String> {
    let config = crate::config::Config::load_or_init()
        .await
        .map_err(|e| e.to_string())?;

    list_configured_models_from_config(provider_id, &config).await
}

pub async fn list_configured_models_from_config(
    provider_id: &str,
    config: &crate::config::Config,
) -> Result<crate::core::Outcome<serde_json::Value>, String> {
    let provider_id = provider_id.trim().to_string();
    if provider_id.is_empty() {
        return Err("provider_id must not be empty".to_string());
    }

    log::debug!("[providers][list_models] provider_id={}", provider_id);

    // A registered catalog (`inference::registry`) answers before any entry
    // resolution; nothing registered => the built-in listing below.
    if let Some(catalog) = crate::inference::registry::catalog(&provider_id) {
        log::debug!(
            "[providers][list_models] provider_id={provider_id} served by a registered catalog"
        );
        let entry = config
            .cloud_providers
            .iter()
            .find(|e| e.id == provider_id || e.slug == provider_id);
        let request = catalog_request(config, entry)?;
        return (catalog.list)(config.clone(), request).await;
    }
    if provider_id == crate::inference::provider::factory::PROVIDER_OPENHUMAN {
        return Ok(no_managed_catalog());
    }

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

    // An entry that authenticates with the managed session is the managed
    // catalog: the registered provider lists it, and with none registered
    // there is nothing to list.
    if entry.auth_style == crate::config::schema::cloud_providers::AuthStyle::OpenhumanJwt {
        return match crate::inference::registry::catalog(
            crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        ) {
            Some(catalog) => {
                let request = catalog_request(config, Some(&entry))?;
                (catalog.list)(config.clone(), request).await
            }
            None => Ok(no_managed_catalog()),
        };
    }

    // Sentry TAURI-RUST-114C/114D: the app stores Claude Code with the cosmetic
    // endpoint `cli://claude-code`. It runs as a local CLI and has no `/models`
    // listing, and reqwest cannot build a request for a non-http(s) URL, so the
    // probe failed with "builder error" on every picker open. Such a provider
    // has no remote catalog: answer with an empty one.
    if !endpoint_hosts_models_listing(&entry.slug, &entry.endpoint) {
        log::debug!(
            "[providers][list_models] slug={} has no http(s) /models listing; returning an empty list",
            entry.slug
        );
        return Ok(crate::core::Outcome::new(
            serde_json::json!({ "models": Vec::<ModelInfo>::new() }),
            vec![format!(
                "provider '{}' has no remote model catalog",
                entry.slug
            )],
        ));
    }

    let looked_up = crate::inference::provider::factory::lookup_key_for_slug(&entry.slug, config)
        .unwrap_or_default();
    let api_key = resolve_local_runtime_key(&entry.slug, looked_up, config);

    let bearer_is_oauth = entry.slug == "openai"
        && crate::inference::provider::factory::openai_bearer_is_oauth(config);
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

    use crate::config::schema::cloud_providers::AuthStyle;
    let service_key = "providers.list_models";
    let client = if let Some(pem) = config
        .cloud_provider_ca_certs
        .get(entry.slug.trim())
        .filter(|pem| !pem.is_empty())
    {
        crate::util::tls::client_with_ca_bundle_with_timeouts(pem, service_key, 30, 10)?
    } else {
        crate::config::build_runtime_proxy_client_with_timeouts(service_key, 30, 10)
    };

    if is_openrouter_provider(&entry.slug, &entry.endpoint) {
        validate_openrouter_api_key(&client, &routing.endpoint, &api_key).await?;
    }

    let mut request = client.get(&models_url);
    if routing.using_oauth {
        request = request
            .header(
                reqwest::header::USER_AGENT,
                openai_codex_user_agent("OpenHuman", env!("CARGO_PKG_VERSION")),
            )
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
        // Handled above: the managed catalog is served by the registered provider.
        AuthStyle::OpenhumanJwt => request,
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
        let truncated = crate::util::truncate_with_ellipsis(&sanitized, 300);
        // TAURI-RUST-8X3: a 404 from the `<base>/models` probe means the
        // configured base URL does not host an OpenAI-compatible `/models`
        // listing (wrong base, a model-only proxy, or a missing `/v1`
        // suffix). Append an actionable hint so the model-dropdown probe
        // surfaces *recovery guidance* inline instead of the bare Go-style
        // `404 page not found`. The `provider returned 404` prefix is kept
        // verbatim so the `is_provider_user_state_message` classifier anchor
        // (which demotes this preventable user-state case out of Sentry)
        // still matches — see `crates/openhuman-core/src/core/observability.rs`.
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
        let snippet = crate::util::truncate_with_ellipsis(&sanitized, 300);
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
    let mut models = parse_models_response(&body).map_err(|error| error.to_string())?;
    if routing.using_oauth {
        merge_openai_codex_model_hints(&mut models);
    }

    log::info!(
        "[providers][list_models] slug={} fetched {} models",
        entry.slug,
        models.len()
    );

    Ok(crate::core::Outcome::new(
        serde_json::json!({ "models": models }),
        vec![format!("fetched {} models", models.len())],
    ))
}
