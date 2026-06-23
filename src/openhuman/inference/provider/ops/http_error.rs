use super::sanitize::sanitize_api_error;
use crate::openhuman::inference::provider::openhuman_backend;

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
pub fn is_budget_exhausted_http_400(status: reqwest::StatusCode, body: &str) -> bool {
    status == reqwest::StatusCode::BAD_REQUEST
        && crate::openhuman::inference::provider::is_budget_exhausted_message(body)
}

/// Whether a custom OpenAI-compatible proxy returned the known generic
/// upstream 400 envelope:
/// `{"error":{"message":"Bad request to upstream provider","type":"upstream_error","status":400}}`.
///
/// This shape is deterministic provider/user-state (endpoint-model mismatch,
/// unsupported schema, provider-side validation) and does not provide
/// actionable signal for OpenHuman Sentry triage.
pub fn is_custom_openai_upstream_bad_request_http_400(
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
pub fn is_provider_access_policy_denied_http_403(status: reqwest::StatusCode, body: &str) -> bool {
    if status != reqwest::StatusCode::FORBIDDEN {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    lower.contains("access_terminated_error")
        || lower.contains("currently only available for coding agents")
}

pub fn log_budget_exhausted_http_400(
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

pub fn log_custom_openai_upstream_bad_request_http_400(
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

/// Whether this provider response carries a managed-backend `errorCode` (#870)
/// that the backend already owns — so the FE must not double-report (F2/F4).
///
/// Gated on `provider == `[`openhuman_backend::PROVIDER_LABEL`]: an `errorCode`
/// is only trustworthy on the **managed backend**. A BYO / direct-provider body
/// that merely contains an `errorCode`-shaped field must NOT be treated as
/// backend-owned (CodeRabbit) — those keep reaching Sentry via the status gate.
///
/// Returns `false` for a backend-flagged **malformed** `BAD_REQUEST`: that one
/// `errorCode` case is a client-built payload the backend couldn't parse, and
/// the FE *does* page for it (F8). Delegates to the single-source decision in
/// [`crate::openhuman::inference::provider::backend_error_code_skips_sentry`]
/// so the provider layer, the higher-layer re-report classifier, and the
/// Sentry `before_send` filter can't drift.
pub fn is_backend_error_code_owned(provider: &str, body: &str) -> bool {
    provider == openhuman_backend::PROVIDER_LABEL
        && crate::openhuman::inference::provider::backend_error_code_skips_sentry(body)
}

pub fn log_backend_error_code_owned(
    operation: &str,
    provider: &str,
    model: Option<&str>,
    status: reqwest::StatusCode,
    body: &str,
) {
    let code = crate::openhuman::inference::provider::extract_backend_error_code_token(body)
        .unwrap_or_default();
    tracing::info!(
        domain = "llm_provider",
        operation = operation,
        provider = provider,
        model = model.unwrap_or(""),
        status = status.as_u16(),
        failure = "non_2xx",
        kind = "backend_error_code",
        error_code = %code,
        "[llm_provider] {operation} backend errorCode={code} ({status}) — backend owns \
         this error, not reporting to Sentry"
    );
}

pub fn log_provider_access_policy_denied_http_403(
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
/// **insufficient-credits** user-state error — the BYO provider account
/// (e.g. OpenRouter) lacks the balance to satisfy the request.
///
/// This is the *residual* case once the request already caps `max_tokens`
/// (so the provider's pre-flight is priced against a realistic output budget
/// rather than the model's full window — see
/// [`crate::openhuman::inference::provider::ChatRequest::max_tokens`]): a 402
/// that still arrives means the user's own third-party account is genuinely
/// out of credit, a billing state OpenHuman has no lever over. Demote from
/// Sentry to an info log rather than page once per retry
/// (TAURI-RUST-C62: 12k events from a single low-balance user).
///
/// Gated on the 402 status **and** a credit/payment phrase so an unrelated
/// 402 is not swallowed. The phrase list is covered by a verbatim-body test
/// so a provider wording drift fails CI instead of silently leaking events.
pub fn is_provider_insufficient_credits_402(status: reqwest::StatusCode, body: &str) -> bool {
    status == reqwest::StatusCode::PAYMENT_REQUIRED && body_indicates_insufficient_credits(body)
}

/// Phrase-level matcher for an insufficient-credits / out-of-balance provider
/// error body. Single source of truth for the credit-phrase set, shared by the
/// emit-site guard [`is_provider_insufficient_credits_402`] (which adds the 402
/// status gate) and the `before_send` defense-in-depth filter
/// [`crate::core::observability::is_insufficient_credits_event`] (which matches
/// the formatted `<provider> API error (402 …): <body>` message so the demotion
/// reaches every compatible-provider HTTP path — `chat_with_system`,
/// `chat_with_history`, the streaming gates, and `api_error` — not just
/// `Provider::chat()`'s `native_chat` cascade). TAURI-RUST-C62.
pub fn body_indicates_insufficient_credits(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("requires more credits")
        || lower.contains("more credits")
        || lower.contains("can only afford")
        || lower.contains("insufficient credit")
        || lower.contains("insufficient balance")
        || lower.contains("insufficient funds")
        || lower.contains("payment required")
}

pub fn log_provider_insufficient_credits_402(
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
        kind = "insufficient_credits",
        "[llm_provider] {operation} provider insufficient-credits 402 — BYO account out of \
         balance (no local lever), not reporting to Sentry"
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
pub fn is_provider_config_rejection_http(
    status: reqwest::StatusCode,
    provider: &str,
    body: &str,
) -> bool {
    // 403 is included for the Ollama Cloud subscription gate:
    // `{"error":"this model requires a subscription, upgrade for access: …"}`.
    // That is deterministic user-state (paid-tier model, free account) — the
    // same class as the 400/404/422 config-rejection shapes above. See
    // TAURI-RUST-4XK. The general `is_backend_auth_failure` polarity guard
    // still fires first (backend 401/403 → SessionExpired), so this branch
    // is only reachable for non-backend providers. The phrase-level polarity
    // guard below (`provider != openhuman_backend::PROVIDER_LABEL`) provides
    // a second layer of defence for the non-OpenAI-compat shapes.
    if !matches!(status.as_u16(), 400 | 403 | 404 | 422) {
        return false;
    }
    if !crate::openhuman::inference::provider::is_provider_config_rejection_message(body) {
        return false;
    }
    // OpenAI-compatible "unknown model" body is user-state regardless of
    // provider — both third-party `custom_openai` upstreams and our own
    // OpenHuman backend now emit it for user-configured model ids that
    // aren't in the registry (TAURI-RUST-2Z1).
    if crate::openhuman::inference::provider::is_openai_compatible_unknown_model_message(body) {
        return true;
    }
    // Remaining config-rejection phrases (DeepSeek `supported api model
    // names are`, Moonshot `invalid temperature`, litellm envelopes, …)
    // are intrinsically scoped to third-party providers — keep the
    // polarity guard so a regression where our own backend emits one of
    // those still reaches Sentry.
    provider != openhuman_backend::PROVIDER_LABEL
}

pub fn log_provider_config_rejection(
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
        // LM Studio / llama.cpp un-evictable-prefix overflow (TAURI-RUST-6V0):
        // `"The number of tokens to keep from the initial prompt is greater
        //   than the context length (n_keep: 10978 >= n_ctx: 8192). Try to
        //   load the model with a larger context length, …"`. The user's local
        // model was loaded with an `n_ctx` smaller than the system/un-evictable
        // prefix; the remediation lives in the user's local server (reload with
        // a larger context), so this is expected user-state, not a product bug.
        "greater than the context length",
    ];
    if CONTEXT_HINTS.iter().any(|hint| lower.contains(hint)) {
        return true;
    }

    // LM Studio / llama.cpp emit the overflow as a paired `n_keep … n_ctx`
    // diagnostic. Require BOTH tokens so the arm stays anchored to that exact
    // shape (TAURI-RUST-6V0) and never broadens to unrelated `n_ctx` logging.
    if lower.contains("n_keep") && lower.contains("n_ctx") {
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

pub fn log_context_window_exceeded(
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
/// auth domain owns recovery, so the predicate is provider-scoped to
/// [`openhuman_backend::PROVIDER_LABEL`]. A `401`/`403` from **other** providers
/// with an auth-key envelope (missing/invalid BYO key) is demoted separately by
/// [`is_byo_provider_auth_failure_http`]; anything else still reaches Sentry.
pub fn is_backend_auth_failure(provider: &str, status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 401 | 403) && provider == openhuman_backend::PROVIDER_LABEL
}

/// Whether a non-backend provider's `401`/`403` carries an OpenAI-style
/// authentication-error body — i.e. a missing or invalid BYO API key.
///
/// This is deterministic **user-config** state (the user pasted a bad or empty
/// key into a custom OpenAI-compatible provider), not a product bug. Sentry has
/// no remediation path, yet retry loops (memory-tree extraction, memory jobs,
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
///   [`openhuman_backend::PROVIDER_LABEL`].
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
    if provider == openhuman_backend::PROVIDER_LABEL {
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
    // OpenAI-style auth envelopes across the BYO providers seen in Sentry:
    // `"type":"authentication_error"` (kiro / Anthropic-style), OpenAI's
    // `"code":"invalid_api_key"` + "Incorrect API key provided", and the
    // bare-message variants Cohere / litellm gateways emit (#3354).
    const AUTH_ERROR_MARKERS: &[&str] = &[
        "authentication_error",
        "invalid_api_key",
        "invalid api key",
        "invalid or missing api key",
        "missing api key",
        "no api key supplied",
        "incorrect api key",
        "invalid authentication",
    ];
    let matched = AUTH_ERROR_MARKERS
        .iter()
        .any(|marker| lower.contains(marker));
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
/// publish [`crate::core::event_bus::DomainEvent::SessionExpired`]: an expired
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
    if provider == openhuman_backend::PROVIDER_LABEL {
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
    let matched = is_openai_oauth_session_expired_message(body);
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

/// Message-level half of [`is_openai_oauth_session_expired_http`]: matches the
/// OpenAI OAuth session-expiry body markers without a status/provider gate.
///
/// The provider HTTP layer demotes its own per-attempt event via the `_http`
/// gate, but the same `anyhow::bail!` string is re-raised at the JSON-RPC
/// boundary (`core::jsonrpc` → `report_error_or_expected` →
/// `core::observability::expected_error_kind`), which has only the message
/// string — no status. This predicate lets that central classifier demote the
/// re-report too, so an RPC-triggered chat/test call does not leak the event
/// the `_http` gate already suppressed (TAURI-RUST-8FQ). Mirrors the
/// `is_provider_config_rejection_message` / `_http` split.
///
/// `token_expired` is OpenAI's OAuth error code; the prose variants cover
/// sanitized/reworded bodies. An API-key rejection never carries these (it
/// emits "incorrect api key" / "invalid_api_key"), and the backend app-session
/// "invalid token" / "please sign in again" wording differs, so this cannot
/// swallow a real misconfig or a backend session-expiry.
pub fn is_openai_oauth_session_expired_message(message: &str) -> bool {
    const OAUTH_EXPIRY_MARKERS: &[&str] = &[
        "token_expired",
        "authentication token is expired",
        "please try signing in again",
    ];
    let lower = message.to_ascii_lowercase();
    OAUTH_EXPIRY_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
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
    // F4/F2: any managed-backend response carrying a stable `errorCode` is
    // backend-owned — it already paged or is expected user-state — so the FE
    // must not double-report. The one exception (malformed `BAD_REQUEST`) is
    // excluded by `is_backend_error_code_owned` and falls through to the
    // status gate below, which reports it (status 400 is non-transient) — F8.
    let is_backend_error_code_owned = is_backend_error_code_owned(provider, &body);
    // Missing/invalid BYO API key on a non-backend provider — user-config
    // state, not a product bug. Demote from Sentry (TAURI-RUST-DHM flood).
    let is_byo_auth_failure = is_byo_provider_auth_failure_http(provider, status, &body);
    // OpenAI ChatGPT/Codex OAuth access token expired with no usable refresh
    // token — user must reconnect OpenAI. Deterministic user-state, demote
    // from Sentry (TAURI-RUST-8FQ flood).
    let is_openai_oauth_session_expired =
        is_openai_oauth_session_expired_http(provider, status, &body);
    // Insufficient-credits 402: the user's own BYO provider account is out of
    // balance — a flat billing fact, not a reservation-window error, so there is
    // NO local max_tokens lever to apply. Demote from Sentry like the per-method
    // compatible-provider arms; the complete classification for a genuinely-
    // unpreventable BYO-balance condition (TAURI-RUST-4QF DeepSeek "Insufficient
    // Balance"). This shared helper backs the two methods that delegate here
    // (chat_via_responses fallback and the non-streaming completion path).
    let is_insufficient_credits_402 = is_provider_insufficient_credits_402(status, &body);

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
    } else if is_backend_error_code_owned {
        log_backend_error_code_owned("api_error", provider, None, status, &body);
    } else if is_byo_auth_failure {
        log_byo_provider_auth_failure("api_error", provider, None, status);
    } else if is_openai_oauth_session_expired {
        log_openai_oauth_session_expired("api_error", provider, None, status);
    } else if is_insufficient_credits_402 {
        log_provider_insufficient_credits_402("api_error", provider, None, status);
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

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    /// Verbatim TAURI-RUST-C62 provider body. The matcher keys on this prose,
    /// so coupling the test to the exact string makes a provider wording drift
    /// fail CI rather than silently leak events back to Sentry.
    const C62_BODY: &str = "myopenrouter API error (402 Payment Required): \
        {\"error\":{\"message\":\"This request requires more credits, or fewer max_tokens. \
        You requested up to 65536 tokens, but can only afford 49732.\"}}";

    #[test]
    fn insufficient_credits_402_matches_verbatim_c62_body() {
        assert!(is_provider_insufficient_credits_402(
            StatusCode::PAYMENT_REQUIRED,
            C62_BODY
        ));
    }

    #[test]
    fn insufficient_credits_402_matches_common_phrasings() {
        for body in [
            "insufficient balance",
            "Insufficient credits to complete this request",
            "insufficient funds on account",
            "you can only afford 100 tokens",
            "402 Payment Required",
        ] {
            assert!(
                is_provider_insufficient_credits_402(StatusCode::PAYMENT_REQUIRED, body),
                "should match: {body:?}"
            );
        }
    }

    #[test]
    fn insufficient_credits_402_ignores_non_402_status() {
        // Same prose but a non-402 status is not this user-state — must stay
        // reportable so a genuine bug elsewhere isn't swallowed.
        assert!(!is_provider_insufficient_credits_402(
            StatusCode::BAD_REQUEST,
            C62_BODY
        ));
        assert!(!is_provider_insufficient_credits_402(
            StatusCode::INTERNAL_SERVER_ERROR,
            C62_BODY
        ));
    }

    #[test]
    fn insufficient_credits_402_ignores_unrelated_402_body() {
        // A 402 without any credit/payment phrase (reserved for other payment
        // semantics) is not swallowed by this guard.
        assert!(!is_provider_insufficient_credits_402(
            StatusCode::PAYMENT_REQUIRED,
            "{\"error\":{\"message\":\"some unrelated condition\"}}"
        ));
    }

    /// Verbatim TAURI-RUST-8FQ Responses-API body. The matcher keys on this
    /// envelope, so coupling the test to the exact string makes a provider
    /// wording drift fail CI rather than silently leak events to Sentry.
    const OAUTH_EXPIRED_8FQ_BODY: &str = "{\"error\":{\"message\":\"Provided \
        authentication token is expired. Please try signing in again.\",\
        \"type\":null,\"code\":\"token_expired\",\"param\":null}}";

    #[test]
    fn openai_oauth_session_expired_matches_verbatim_8fq_body() {
        assert!(is_openai_oauth_session_expired_http(
            "openai",
            StatusCode::UNAUTHORIZED,
            OAUTH_EXPIRED_8FQ_BODY
        ));
    }

    #[test]
    fn openai_oauth_session_expired_matches_marker_variants() {
        for body in [
            "{\"error\":{\"code\":\"token_expired\"}}",
            "Provided authentication token is expired.",
            "Please try signing in again.",
        ] {
            assert!(
                is_openai_oauth_session_expired_http("openai", StatusCode::UNAUTHORIZED, body),
                "should match: {body:?}"
            );
        }
    }

    #[test]
    fn openai_oauth_session_expired_ignores_invalid_api_key_401() {
        // A genuine bad-key rejection must NOT be swallowed here — it is
        // routed by `is_byo_provider_auth_failure_http` instead and stays
        // actionable. The two classifiers must not overlap.
        let bad_key = "{\"error\":{\"code\":\"invalid_api_key\",\
            \"message\":\"Incorrect API key provided.\"}}";
        assert!(!is_openai_oauth_session_expired_http(
            "openai",
            StatusCode::UNAUTHORIZED,
            bad_key
        ));
        assert!(is_byo_provider_auth_failure_http(
            "openai",
            StatusCode::UNAUTHORIZED,
            bad_key
        ));
    }

    #[test]
    fn openai_oauth_session_expired_ignores_non_401_status() {
        // Same prose on a non-401 status is not this user-state — keep it
        // reportable so a genuine bug elsewhere isn't masked.
        assert!(!is_openai_oauth_session_expired_http(
            "openai",
            StatusCode::INTERNAL_SERVER_ERROR,
            OAUTH_EXPIRED_8FQ_BODY
        ));
        assert!(!is_openai_oauth_session_expired_http(
            "openai",
            StatusCode::BAD_REQUEST,
            OAUTH_EXPIRED_8FQ_BODY
        ));
    }

    #[test]
    fn openai_oauth_session_expired_excludes_backend_provider() {
        // The OpenHuman backend owns app-session expiry via
        // `publish_backend_session_expired`; this provider-OAuth gate must not
        // claim a backend 401.
        assert!(!is_openai_oauth_session_expired_http(
            openhuman_backend::PROVIDER_LABEL,
            StatusCode::UNAUTHORIZED,
            OAUTH_EXPIRED_8FQ_BODY
        ));
    }
}
