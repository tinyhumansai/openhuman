use super::sanitize::sanitize_api_error;
use crate::openhuman::inference::provider::openhuman_backend_model;

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

/// Whether a provider non-2xx response is a local inference server that is
/// running but has **no model loaded** (e.g. LM Studio idle): a 400 carrying
/// `No models loaded. Please load a model …`.
///
/// This is pure local user-state — nothing OpenHuman sent is malformed, there
/// is no product bug and no local lever beyond the user loading a model — so it
/// should be demoted from Sentry to an info log rather than paging on every
/// retry (TAURI-RUST-DMQ: 5,469 events from a single idle LM Studio server).
/// The embeddings path already special-cases this exact string
/// (`embeddings/rpc.rs`, PR #3688 / TAURI-RUST-4P4); this is the chat sibling.
pub fn is_local_provider_no_model_loaded(status: reqwest::StatusCode, body: &str) -> bool {
    status == reqwest::StatusCode::BAD_REQUEST
        && body.to_ascii_lowercase().contains("no models loaded")
}

/// Actionable user-facing guidance for a local inference server with no model
/// loaded, mirroring the embeddings verification message
/// (`embeddings/rpc.rs`). Returned in place of the raw provider body so the
/// surfaced error tells the user how to fix it.
pub fn local_provider_no_model_loaded_user_message() -> String {
    "Your local inference server (e.g. LM Studio) is running but has no model loaded. \
     Load a model — in LM Studio use the developer page or the `lms load` command — \
     then try again."
        .to_string()
}

pub fn log_local_provider_no_model_loaded(
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
        reason = "local_provider_no_model_loaded",
        "[llm_provider] {operation} local inference server has no model loaded — \
         user must load a model, not reporting to Sentry"
    );
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
/// Gated on `provider == `[`openhuman_backend_model::PROVIDER_LABEL`]: an `errorCode`
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
    provider == openhuman_backend_model::PROVIDER_LABEL
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

/// Whether a provider non-2xx response is a deterministic **monthly-quota /
/// usage-limit exhausted** user-state error — the user's third-party plan has
/// spent its allotment for the period and no request will succeed until it
/// resets (a billing/plan state OpenHuman has no lever over).
///
/// Distinct from [`is_provider_insufficient_credits_402`] in two ways:
/// 1. The signal is a *usage-quota cap* ("you have reached the limit",
///    `MONTHLY_REQUEST_COUNT`), not an account balance.
/// 2. The upstream proxy may wrap its own 402 inside a **500** envelope, e.g.
///    Kiro IDE: `kiro API error (500 Internal Server Error): {"error":\
///    {"message":"HTTP 402 from Kiro IDE: {\"reason\":\"MONTHLY_REQUEST_COUNT\"}"…}}`.
///    So this is **status-agnostic** — matched against the body like
///    [`is_context_window_exceeded_message`] — because gating on a 402
///    transport status (as the credits matcher does) would let the 500-wrapped
///    flood straight through to [`should_report_provider_http_failure`]
///    (TAURI-RUST-C9A: 9k events from a single quota-capped user, retried per
///    memory-extraction attempt).
///
/// Keyed on quota-specific wording only, so a generic 500 outage (or a 429
/// rate-limit, which has its own transient handling) is not swallowed. Covered
/// by a verbatim-body test so a provider wording drift fails CI.
pub fn is_provider_quota_exhausted(body: &str) -> bool {
    body_indicates_quota_exhausted(body)
}

/// Phrase-level matcher for a provider monthly-quota / usage-limit exhausted
/// body. Single source of truth for the quota-phrase set, shared by the
/// emit-site guard [`is_provider_quota_exhausted`] and the `before_send`
/// defense-in-depth filter
/// [`crate::core::observability::is_quota_exhausted_event`] (which matches the
/// formatted `<provider> API error (…): <body>` message so the demotion reaches
/// every compatible-provider HTTP path, not just `Provider::chat()`'s
/// `native_chat` cascade). TAURI-RUST-C9A.
pub fn body_indicates_quota_exhausted(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("monthly_request_count")
        || lower.contains("monthly request")
        || lower.contains("monthly limit")
        || lower.contains("monthly quota")
        || lower.contains("quota exceeded")
        || lower.contains("usage limit exceeded")
        // Codex/ChatGPT OAuth `/responses` plan-cap body (TAURI-RUST-AFE):
        // `usage_limit_reached` / "The usage limit has been reached" — a plan
        // quota with no "monthly"/"quota" co-marker, so the phrases above miss
        // it. Both are quota-specific enough to match on their own (the loop
        // retries until `resets_at`, flooding from a single capped Plus user).
        || lower.contains("usage_limit_reached")
        || lower.contains("usage limit has been reached")
        // "reached the limit" alone is ambiguous (rate-limit, token-limit), so
        // require a quota/plan/request/monthly co-marker to keep the blast
        // radius on plan-quota exhaustion only.
        || (lower.contains("reached the limit")
            && (lower.contains("request")
                || lower.contains("quota")
                || lower.contains("monthly")
                || lower.contains("plan")))
}

pub fn log_provider_quota_exhausted(
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
        kind = "quota_exhausted",
        "[llm_provider] {operation} provider monthly-quota exhausted — third-party plan limit \
         reached (no local lever), not reporting to Sentry"
    );
}

/// Stable anchor phrase for the actionable Ollama-Cloud-500 user message, shared
/// by [`ollama_cloud_internal_500_user_message`] (which builds it) and
/// [`is_ollama_cloud_internal_500_message`] (which matches the re-raised string
/// at the RPC/agent boundary), so the two cannot drift.
const OLLAMA_CLOUD_INTERNAL_500_USER_PREFIX: &str = "Ollama cloud is temporarily unavailable";

/// Whether a provider non-2xx response is an Ollama **Cloud** hosted-inference
/// internal error: `500` + a `{"error":"Internal Server Error (ref: <uuid>)"}`
/// body.
///
/// ollama.com's hosted `*:cloud` models (minimax-m3 / qwen3.5 / gpt-oss …)
/// intermittently `500` with this opaque, server-generated envelope. The `ref:`
/// is a fresh UUID per event, the failure is non-deterministic, and the request
/// that 500s is byte-identical to the one that succeeds when the cloud backend
/// is healthy — so there is **no client lever** (nothing to validate,
/// reshape, or reconfigure). The reliable-provider layer already retries and
/// falls back across providers/models, so each per-attempt 500 is pure noise:
/// TAURI-RUST-5MV, 3,062 events from 5 users in a single window. Demote from
/// Sentry to an info log while the error still propagates so retry/fallback runs
/// unchanged.
///
/// Anchored on the `internal server error (ref:` body shape, which is specific
/// to ollama.com's hosted envelope — a **local** Ollama daemon 500 (a genuine
/// model crash / OOM worth paging) does not carry a `ref:` UUID, so it still
/// reaches Sentry. The phrase is covered by a verbatim-body test so a provider
/// wording drift fails CI instead of silently leaking events.
pub fn is_ollama_cloud_internal_500(
    provider: &str,
    status: reqwest::StatusCode,
    body: &str,
) -> bool {
    provider == "ollama"
        && status == reqwest::StatusCode::INTERNAL_SERVER_ERROR
        && body
            .to_ascii_lowercase()
            .contains("internal server error (ref:")
}

/// Message-level half of [`is_ollama_cloud_internal_500`]: matches the actionable
/// user message re-raised at the RPC/agent boundary
/// (`core::observability::expected_error_kind`), so the higher-layer re-report is
/// demoted too instead of leaking the event the emit-site already suppressed (the
/// `domain=agent` half of TAURI-RUST-5MV). Mirrors the
/// `is_provider_insufficient_credits_402` / `body_indicates_insufficient_credits`
/// split. Keyed on the [`OLLAMA_CLOUD_INTERNAL_500_USER_PREFIX`] anchor, which we
/// own, so it cannot collide with an unrelated provider body.
pub fn is_ollama_cloud_internal_500_message(message: &str) -> bool {
    let needle = OLLAMA_CLOUD_INTERNAL_500_USER_PREFIX.to_ascii_lowercase();
    message.to_ascii_lowercase().contains(needle.as_str())
}

/// Build the actionable user-facing message for an Ollama-Cloud hosted-inference
/// 500, replacing the opaque `Internal Server Error (ref: <uuid>)` body (which
/// carries no signal the user can act on) with retry/switch guidance. The model
/// is included when known (native/streaming chat); the `api_error` path has no
/// model in scope and omits it.
pub fn ollama_cloud_internal_500_user_message(
    model: Option<&str>,
    status: reqwest::StatusCode,
) -> String {
    let code = status.as_u16();
    match model {
        Some(model) => format!(
            "{OLLAMA_CLOUD_INTERNAL_500_USER_PREFIX} for model `{model}` (Ollama returned HTTP \
             {code}); the hosted model failed on Ollama's side — retry shortly or switch models."
        ),
        None => format!(
            "{OLLAMA_CLOUD_INTERNAL_500_USER_PREFIX} (Ollama returned HTTP {code}); the hosted \
             model failed on Ollama's side — retry shortly or switch models."
        ),
    }
}

pub fn log_ollama_cloud_internal_500(
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
        kind = "ollama_cloud_internal_500",
        "[llm_provider] {operation} Ollama Cloud hosted-inference 500 — provider-internal \
         (no client lever), not reporting to Sentry"
    );
}

/// Whether a provider non-2xx response is an **external content-moderation
/// rejection** — the user routes their provider (here: ollama) through a
/// third-party safety/moderation proxy that refuses the prompt with a `400`
/// and a verdict envelope, e.g.
/// `{"error":"Message rejected by Ombudsman","score":80}` (TAURI-RUST-ECR,
/// 4,517 events from a single looping triage-agent machine).
///
/// This is `genuinely-unpreventable` from OpenHuman's side: the request is
/// well-formed, the rejection comes from an external guard we neither own nor
/// configure, and there is no client lever to reshape the request into one the
/// proxy will accept. The triage agent re-issues the same prompt every turn, so
/// the raw 400 floods `report_error` (400 ∉ the transient set in
/// [`should_report_provider_http_failure`]). Demote to an info log while the
/// error still propagates so retry/fallback runs unchanged — the same
/// classify-and-backpressure answer as the 5MV / A3T / 8S3 precedent.
///
/// Anchored on the moderation-verdict shape — the rejection wording
/// (`message rejected` / `ombudsman`) or the `"score"` verdict field — none of
/// which OpenHuman's own backend or a normal provider 400 (malformed request,
/// schema error) emits, so a genuine bug still reaches Sentry. Covered by a
/// verbatim-body test so a proxy wording drift fails CI instead of silently
/// leaking events.
pub fn is_provider_moderation_rejection_http_400(status: reqwest::StatusCode, body: &str) -> bool {
    if status != reqwest::StatusCode::BAD_REQUEST {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    lower.contains("message rejected")
        || lower.contains("ombudsman")
        // The moderation proxy returns its confidence as a `"score"` JSON field
        // alongside the verdict; anchor on the quoted key shape (not a bare
        // `score`) so an unrelated 400 mentioning the word in prose isn't
        // swallowed.
        || lower.contains("\"score\"")
}

pub fn log_provider_moderation_rejection(
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
        kind = "external_moderation_rejection",
        "[llm_provider] {operation} external content-moderation rejection ({status}) — request \
         refused by a third-party moderation proxy (no client lever), not reporting to Sentry"
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
    // guard below (`provider != openhuman_backend_model::PROVIDER_LABEL`) provides
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
    provider != openhuman_backend_model::PROVIDER_LABEL
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
