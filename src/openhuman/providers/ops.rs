use super::*;

use std::path::PathBuf;

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

    if is_auth_failure && is_backend {
        tracing::warn!(
            domain = "llm_provider",
            operation = "api_error",
            provider = provider,
            status = status_str.as_str(),
            "[llm_provider] backend auth failure ({status}) — publishing SessionExpired"
        );
        // `message` already embeds the sanitized body via
        // `sanitize_api_error(&body)`, but the leading `{provider} API
        // error ({status})` prefix and any caller-controlled provider
        // name aren't scrubbed — re-run sanitize on the final string so
        // the SessionExpired subscriber's logs never persist secrets.
        crate::core::event_bus::publish_global(
            crate::core::event_bus::DomainEvent::SessionExpired {
                source: "llm_provider.openhuman_backend".to_string(),
                reason: sanitize_api_error(&message),
            },
        );
    } else if is_budget_exhausted_user_state {
        log_budget_exhausted_http_400("api_error", provider, None, status);
    } else if should_report_provider_http_failure(status) {
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
            crate::openhuman::providers::compatible::OpenAiCompatibleProvider::new(
                "custom_openai",
                url,
                Some(key),
                crate::openhuman::providers::compatible::AuthStyle::Bearer,
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

    let provider = crate::openhuman::routing::new_provider(remote, &config.local_ai, default_model);
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
mod tests {
    use super::*;

    #[test]
    fn factory_backend() {
        assert!(create_backend_inference_provider(
            None,
            None,
            None,
            &ProviderRuntimeOptions::default()
        )
        .is_ok());
    }

    #[test]
    fn skips_sentry_report_for_transient_upstream_statuses() {
        // Transient statuses — 429 rate-limit, 408 client timeout, and 502/503/504
        // gateway-layer failures — are retried by reliable.rs. The aggregate
        // "all providers exhausted" event still fires for genuine outages.
        // Reporting each attempt individually floods Sentry (OPENHUMAN-TAURI-2E
        // ~1393 events, 84 ~1050 events, T ~871 events).
        for transient in [
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            reqwest::StatusCode::REQUEST_TIMEOUT,
            reqwest::StatusCode::BAD_GATEWAY,
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            reqwest::StatusCode::GATEWAY_TIMEOUT,
        ] {
            assert!(
                !should_report_provider_http_failure(transient),
                "transient status {transient} must not trigger per-attempt Sentry report"
            );
        }
        // Auth + permanent server faults remain reportable — those are
        // misconfiguration or genuine bugs, not transient capacity issues.
        for reportable in [
            reqwest::StatusCode::UNAUTHORIZED,
            reqwest::StatusCode::FORBIDDEN,
            reqwest::StatusCode::BAD_REQUEST,
            reqwest::StatusCode::NOT_FOUND,
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            assert!(
                should_report_provider_http_failure(reportable),
                "status {reportable} must still report to Sentry"
            );
        }
    }

    // Confirm the budget-exhausted suppression predicate is scoped correctly.
    // These tests exercise the real production function, not a duplicate.
    mod budget_exhausted_suppression {
        use super::*;

        const BUDGET_BODY: &str = "Insufficient budget";
        const UNRELATED_BODY: &str = "Invalid request: model not found";

        #[test]
        fn budget_exhausted_400_is_suppressed() {
            assert!(is_budget_exhausted_http_400(
                reqwest::StatusCode::BAD_REQUEST,
                BUDGET_BODY,
            ));
        }

        #[test]
        fn budget_exhausted_400_is_case_insensitive() {
            assert!(is_budget_exhausted_http_400(
                reqwest::StatusCode::BAD_REQUEST,
                "budget exceeded — ADD credits to continue",
            ));
        }

        #[test]
        fn budget_exhausted_500_is_not_suppressed() {
            // A 500 is a server bug, not expected user-state — keep reporting.
            assert!(!is_budget_exhausted_http_400(
                reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                BUDGET_BODY,
            ));
        }

        #[test]
        fn budget_exhausted_400_unrelated_body_is_not_suppressed() {
            assert!(!is_budget_exhausted_http_400(
                reqwest::StatusCode::BAD_REQUEST,
                UNRELATED_BODY,
            ));
        }

        #[test]
        fn budget_exhausted_402_is_not_suppressed() {
            assert!(!is_budget_exhausted_http_400(
                reqwest::StatusCode::PAYMENT_REQUIRED,
                BUDGET_BODY,
            ));
        }

        #[test]
        fn budget_exhausted_empty_body_is_not_suppressed() {
            assert!(!is_budget_exhausted_http_400(
                reqwest::StatusCode::BAD_REQUEST,
                "",
            ));
        }
    }

    #[test]
    fn test_sanitize_api_error_utf8() {
        let input = "🦀".repeat(MAX_API_ERROR_CHARS + 10);
        let sanitized = sanitize_api_error(&input);
        assert!(sanitized.ends_with("..."));
        // Should truncate at MAX_API_ERROR_CHARS crabs
        let crabs_count = sanitized.chars().filter(|c| *c == '🦀').count();
        assert_eq!(crabs_count, MAX_API_ERROR_CHARS);
    }
}
