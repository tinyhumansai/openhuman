
/// Whether a provider error body is a **permanent per-request rate-cap
/// rejection**: the provider refused because a *single* request's token count
/// exceeds the account's tokens-per-minute (TPM) budget, so no amount of
/// retrying or spacing can ever let it through on the current tier.
///
/// Distinct from a *transient* TPM `429` ("rate limit reached … try again in
/// 2s" — a burst that [`is_context_window_exceeded_message`] and the `reliable`
/// retry classifier deliberately keep retryable), from a monthly-plan quota
/// ([`body_indicates_quota_exhausted`]), and from context-window overflow
/// ([`is_context_window_exceeded_message`], a model-size limit not a rate cap).
/// Here the request is larger than the per-minute limit outright, so it is
/// permanently non-viable until the user picks a higher-tier model/provider —
/// OpenHuman has no lever to raise a third-party account's TPM tier.
///
/// Canonical wire shape (groq `on_demand` free tier, Sentry TAURI-RUST-HXF):
/// `groq API error (413 Payload Too Large): {"error":{"message":"Request too
/// large for model `openai/gpt-oss-120b` in organization `org_…` service tier
/// `on_demand` on tokens per minute (TPM): Limit 8000, Requested 42084 …"}}`.
///
/// Anchored on BOTH the permanence marker `"request too large"` (a single
/// request over the cap, not a burst) AND a per-minute-tokens marker
/// (`"tokens per minute"` / `"(tpm)"`), so a transient "rate limit reached,
/// retry in Ns" burst — which lacks "request too large" — is NOT swallowed and
/// stays retryable + Sentry-visible. Status-agnostic (groq uses `413`; a
/// gateway could wrap it) and covered by a verbatim-body test so a provider
/// wording drift fails CI. Single source of truth shared by
/// [`crate::core::observability::is_provider_user_state_message`] (Sentry
/// demotion of the `domain=agent` re-report).
pub fn is_provider_rate_cap_exceeded_message(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("request too large")
        && (lower.contains("tokens per minute") || lower.contains("(tpm)"))
}

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
        .any(|marker| lower.contains(marker))
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
    if crate::openhuman::inference::auth_error_registry::record(provider, status_code) {
        crate::core::bus::BUS.publish(crate::core::events::DomainEvent::ProviderApiKeyRejected {
            provider: provider.to_string(),
            message: crate::openhuman::inference::auth_error_registry::auth_error_message(
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
/// [`crate::core::events::DomainEvent::SessionExpired`] so the credentials
/// subscriber clears the session and flips the scheduler-gate signed-out
/// override (halting downstream LLM work — see OPENHUMAN-TAURI-1T), and skip
/// the Sentry report. Mirrors the `is_auth_failure && is_backend` arm in
/// [`api_error`], factored out for adapter error paths that already consumed
/// the response body and cannot delegate to `api_error`.
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
///   [`crate::core::events::DomainEvent::SessionExpired`] so the credentials
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
    let is_backend = provider == openhuman_backend_model::PROVIDER_LABEL;
    let is_budget_exhausted_user_state = is_budget_exhausted_http_400(status, &body);
    // Local inference server (LM Studio etc.) running with no model loaded —
    // pure local user-state, nothing we sent is malformed. Demote and replace
    // the body with actionable "load a model" guidance (TAURI-RUST-DMQ, mirrors
    // the embeddings #3688 special-case).
    let is_local_provider_no_model_loaded = is_local_provider_no_model_loaded(status, &body);
    let is_custom_openai_upstream_bad_request =
        is_custom_openai_upstream_bad_request_http_400(provider, status, &body);
    let is_provider_access_policy_denied = is_provider_access_policy_denied_http_403(status, &body);
    let is_provider_config_rejection = is_provider_config_rejection_http(status, provider, &body);
    // Context-overflow is status-agnostic: match the body directly (some
    // custom gateways mis-report it as 500 — TAURI-RUST-501 — so a status
    // gate would let those through to `should_report_provider_http_failure`).
    let is_context_window_exceeded = is_context_window_exceeded_message(&body);
    // Monthly-quota exhaustion is likewise status-agnostic: the Kiro IDE proxy
    // wraps its 402 inside a 500 envelope (TAURI-RUST-C9A), so match the body
    // directly rather than gating on a 402 status (which the credits matcher
    // below does). The user's third-party plan quota is spent — no local lever.
    let is_quota_exhausted = is_provider_quota_exhausted(&body);
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
    // Ollama Cloud hosted-inference 500 (`Internal Server Error (ref: <uuid>)`):
    // provider-internal, non-deterministic, no client lever. Demote from Sentry
    // and replace the opaque ref body with actionable guidance (TAURI-RUST-5MV).
    let is_ollama_cloud_internal_500 = is_ollama_cloud_internal_500(provider, status, &body);
    // External content-moderation proxy ("Ombudsman") refused the prompt with a
    // 400 + verdict — well-formed request, external safety guard, no client
    // lever. Demote from Sentry like the native_chat ladder (TAURI-RUST-ECR).
    let is_moderation_rejection = is_provider_moderation_rejection_http_400(status, &body);

    if is_auth_failure && is_backend {
        // Single source of truth for backend session-expiry handling (warn +
        // SessionExpired publish + final-string sanitize) — shared with the
        // hand-rolled `chat_completions` chain in `compatible.rs`.
        publish_backend_session_expired("api_error", provider, status, &message);
    } else if is_budget_exhausted_user_state {
        log_budget_exhausted_http_400("api_error", provider, None, status);
    } else if is_local_provider_no_model_loaded {
        log_local_provider_no_model_loaded("api_error", provider, None, status);
    } else if is_custom_openai_upstream_bad_request {
        log_custom_openai_upstream_bad_request_http_400("api_error", provider, None, status);
    } else if is_provider_access_policy_denied {
        log_provider_access_policy_denied_http_403("api_error", provider, None, status);
    } else if is_provider_config_rejection {
        log_provider_config_rejection("api_error", provider, None, status);
    } else if is_context_window_exceeded {
        log_context_window_exceeded("api_error", provider, None, status);
    } else if is_quota_exhausted {
        log_provider_quota_exhausted("api_error", provider, None, status);
    } else if is_backend_error_code_owned {
        log_backend_error_code_owned("api_error", provider, None, status, &body);
    } else if is_byo_auth_failure {
        log_byo_provider_auth_failure("api_error", provider, None, status);
    } else if is_openai_oauth_session_expired {
        log_openai_oauth_session_expired("api_error", provider, None, status);
    } else if is_insufficient_credits_402 {
        log_provider_insufficient_credits_402("api_error", provider, None, status);
    } else if is_ollama_cloud_internal_500 {
        log_ollama_cloud_internal_500("api_error", provider, None, status);
    } else if is_moderation_rejection {
        log_provider_moderation_rejection("api_error", provider, None, status);
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
    // Replace the opaque `Internal Server Error (ref: <uuid>)` body with
    // actionable guidance; the prefix anchors the higher-layer re-report
    // demotion (`is_ollama_cloud_internal_500_message`).
    if is_ollama_cloud_internal_500 {
        return anyhow::anyhow!(ollama_cloud_internal_500_user_message(None, status));
    }
    // Replace the raw `No models loaded` body with actionable guidance so the
    // surfaced chat error tells the user how to recover (TAURI-RUST-DMQ).
    if is_local_provider_no_model_loaded {
        return anyhow::anyhow!(local_provider_no_model_loaded_user_message());
    }
    anyhow::anyhow!(message)
}
