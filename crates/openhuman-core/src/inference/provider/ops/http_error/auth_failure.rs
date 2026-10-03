//! Auth-failure classification: backend app-session expiry, BYO API-key
//! rejection, and OpenAI OAuth (ChatGPT/Codex) session expiry.

use super::*;

/// Whether a provider non-2xx response is the OpenHuman **backend** rejecting
/// the app session JWT (`401`/`403`). This is expected user-session state
/// (token expired / revoked / rotated server-side), not a product bug — the
/// auth domain owns recovery, so the predicate is provider-scoped to
/// [`openhuman_backend_model::PROVIDER_LABEL`]. A `401`/`403` from **other** providers
/// with an auth-key envelope (missing/invalid BYO key) is demoted separately by
/// [`is_byo_provider_auth_failure_http`]; anything else still reaches Sentry.
pub fn is_backend_auth_failure(provider: &str, status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 401 | 403) && provider == openhuman_backend_model::PROVIDER_LABEL
}

/// Whether a non-backend provider's `401`/`403` carries an OpenAI-style
/// authentication-error body — i.e. a missing or invalid BYO API key.
///
/// This is deterministic **user-config** state (the user pasted a bad or empty
/// key into a custom OpenAI-compatible provider), not a product bug. Sentry has
/// no remediation path, yet retry loops (background jobs,
/// cron) hammer the known-bad credential and flood Sentry with thousands of
/// identical events from a single user — TAURI-RUST-DHM (5,636 events from a
/// `kiro` custom provider with no key), the same class as the Cohere
/// "no api key supplied" flood (#3354) and the backend session-expiry flood
/// (#2786 / [`is_backend_auth_failure`]).
///
/// Provider-scoped and body-shape-anchored, mirroring the sibling rules:
/// - The OpenHuman **backend** keeps its [`is_backend_auth_failure`] →
///   [`publish_backend_session_expired`] branch (a backend `401`/`403` is
///   app-session expiry, not a BYO key), so this predicate excludes
///   [`openhuman_backend_model::PROVIDER_LABEL`].
/// - A `401`/`403` whose body does **not** look like an auth-key envelope
///   (e.g. a gateway returning `401` on quota / geo-block) still reaches Sentry
///   — the gate keys on the body, not the bare status.
pub fn is_byo_provider_auth_failure_http(
    provider: &str,
    status: reqwest::StatusCode,
    body: &str,
) -> bool {
    if !matches!(status.as_u16(), 401 | 403) {
        tracing::debug!(
            domain = "llm_provider",
            operation = "http_error_classifier",
            provider = provider,
            status = status.as_u16(),
            matched = false,
            reason = "byo_provider_auth_failure_probe:non_auth_status",
            "[llm_provider] BYO auth-failure classifier skipped — status is not 401/403"
        );
        return false;
    }
    if provider == openhuman_backend_model::PROVIDER_LABEL {
        tracing::debug!(
            domain = "llm_provider",
            operation = "http_error_classifier",
            provider = provider,
            status = status.as_u16(),
            matched = false,
            reason = "byo_provider_auth_failure_probe:backend_excluded",
            "[llm_provider] BYO auth-failure classifier skipped — backend owns session-expiry recovery"
        );
        return false;
    }
    let lower = body.to_ascii_lowercase();
    // The OpenAI-style auth-envelope marker set lives in `tinyinference-llm`
    // (`body_indicates_auth_key_error`).
    let matched = tinyinference_llm::failure::body_indicates_auth_key_error(body)
        // OpenRouter's wording for a key that resolves to no account
        // (revoked / deleted user): `401 {"error":{"message":"User not
        // found.","code":401}}`. Same invalid-BYO-key user-state as the
        // markers above — OpenHuman has no lever to make the user's
        // third-party account exist. Kept OpenRouter-gated (not a global
        // marker): `"user not found"` is generic prose another provider
        // could emit for an unrelated 401/403, and demoting that would
        // suppress a real error and show the wrong remediation. Without this
        // anchor the 401 leaks to Sentry once per memory-summarization retry
        // (TAURI-RUST-4RC: ~9k events / 6 users). A verbatim-body test
        // couples it to this payload so a wording drift fails CI instead of
        // silently leaking.
        || (provider == "openrouter" && lower.contains("user not found"));
    // Body content is intentionally omitted from the log — it can carry the
    // raw (sanitized-or-not) provider payload; only the match outcome is logged.
    tracing::debug!(
        domain = "llm_provider",
        operation = "http_error_classifier",
        provider = provider,
        status = status.as_u16(),
        matched,
        reason = "byo_provider_auth_failure_probe",
        "[llm_provider] evaluated BYO auth-failure classifier"
    );
    matched
}

pub fn log_byo_provider_auth_failure(
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
        reason = "byo_provider_auth_failure",
        "[llm_provider] {operation} BYO provider auth failure ({status}) — \
         user API key missing/invalid, not reporting to Sentry"
    );

    // Demoting from Sentry hides the failure from us, so it must not also be
    // invisible to the user — the failing path is often a silent background
    // loop (memory summarization) that just degrades to regex-only. Record the
    // rejection into the process registry that backs the AI-settings
    // provider-error notice, and on the *first* record of this episode publish
    // a one-shot notification. The 401 repeats per retry (~9k events for
    // TAURI-RUST-4RC), so the registry latch is what keeps this from
    // re-flooding the notification center the way the raw error flooded Sentry.
    let status_code = status.as_u16();
    if crate::inference::auth_error_registry::record(provider, status_code) {
        crate::core::bus::BUS.publish(crate::core::events::DomainEvent::ProviderApiKeyRejected {
            provider: provider.to_string(),
            message: crate::inference::auth_error_registry::auth_error_message(
                provider,
                status_code,
            ),
        });
    }
}

/// Whether a `401` is the OpenAI **OAuth** (ChatGPT-subscription / Codex)
/// access token having expired — distinct from a misconfigured BYO API key.
///
/// The ChatGPT/Codex OAuth Responses endpoint returns
/// `{"error":{"code":"token_expired","message":"Provided authentication token
/// is expired. Please try signing in again."}}` once the OAuth access token
/// lapses. The valid-`refresh_token` case already self-heals at credential
/// resolution time (`openai_oauth::lookup_openai_oauth_credentials` refreshes
/// proactively within a 2-min skew, and the chat provider is rebuilt per
/// request), so the residual events that reach this 401 are ones where the
/// refresh token is **absent or revoked** — the user must reconnect OpenAI.
/// That is deterministic user-state, not a server bug, and reporting it spams
/// Sentry (TAURI-RUST-8FQ: 97,938 events / 31 users).
///
/// Keyed on the OAuth-expiry body markers, which an API-key rejection never
/// emits (those say "incorrect api key" — caught by
/// [`is_byo_provider_auth_failure_http`] instead). The OpenHuman **backend**
/// provider is excluded — its `401`/`403` is app-session expiry handled by
/// [`publish_backend_session_expired`]. Unlike that path, this does **not**
/// publish [`crate::core::events::DomainEvent::SessionExpired`]: an expired
/// *provider* OAuth token must not tear down the OpenHuman app session.
pub fn is_openai_oauth_session_expired_http(
    provider: &str,
    status: reqwest::StatusCode,
    body: &str,
) -> bool {
    if status.as_u16() != 401 {
        tracing::debug!(
            domain = "llm_provider",
            operation = "http_error_classifier",
            provider = provider,
            status = status.as_u16(),
            matched = false,
            reason = "openai_oauth_session_expired_probe:non_401",
            "[llm_provider] OpenAI OAuth session-expiry classifier skipped — status is not 401"
        );
        return false;
    }
    if provider == openhuman_backend_model::PROVIDER_LABEL {
        tracing::debug!(
            domain = "llm_provider",
            operation = "http_error_classifier",
            provider = provider,
            status = status.as_u16(),
            matched = false,
            reason = "openai_oauth_session_expired_probe:backend_excluded",
            "[llm_provider] OpenAI OAuth session-expiry classifier skipped — backend owns app-session expiry"
        );
        return false;
    }
    let matched = tinyinference_providers::is_openai_oauth_session_expired_message(body);
    tracing::debug!(
        domain = "llm_provider",
        operation = "http_error_classifier",
        provider = provider,
        status = status.as_u16(),
        matched,
        reason = "openai_oauth_session_expired_probe",
        "[llm_provider] evaluated OpenAI OAuth session-expiry classifier"
    );
    matched
}

/// Demote an OpenAI OAuth session-expiry `401` to an info log (user-state,
/// not a server bug) instead of reporting it to Sentry. The message tells the
/// user to reconnect OpenAI, which is the only recovery once the refresh token
/// is gone. See [`is_openai_oauth_session_expired_http`].
pub fn log_openai_oauth_session_expired(
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
        reason = "openai_oauth_session_expired",
        "[llm_provider] {operation} OpenAI OAuth session expired ({status}) — \
         ChatGPT/Codex token lapsed without a usable refresh token; user must \
         reconnect OpenAI, not reporting to Sentry"
    );
}

/// Handle a backend session-expiry auth failure: publish a
/// [`crate::core::events::DomainEvent::SessionExpired`] so the credentials
/// subscriber clears the session and flips the scheduler-gate signed-out
/// override (halting downstream LLM work — see OPENHUMAN-TAURI-1T), and skip
/// the Sentry report. Mirrors the `is_auth_failure && is_backend` arm in
/// [`super::dispatch::api_error`], factored out for adapter error paths that
/// already consumed the response body and cannot delegate to `api_error`.
///
/// `message` is the already-formatted `"{provider} API error ({status}): …"`
/// string; it embeds the sanitized body, but the prefix and caller-controlled
/// provider name aren't scrubbed, so re-run [`sanitize_api_error`] on the final
/// string before it reaches the SessionExpired subscriber's logs.
pub fn publish_backend_session_expired(
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
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::SessionExpired {
        source: "llm_provider.openhuman_backend".to_string(),
        reason: sanitize_api_error(message),
    });
}
