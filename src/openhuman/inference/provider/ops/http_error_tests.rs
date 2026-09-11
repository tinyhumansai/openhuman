use super::*;
use reqwest::StatusCode;

/// Verbatim TAURI-RUST-DMQ LM Studio body — the local server is running but
/// has no model loaded. The matcher keys on this prose, so coupling the test
/// to the exact string makes a wording drift fail CI rather than silently
/// leak events back to Sentry.
const DMQ_BODY: &str = "lm_studio API error (400 Bad Request): {\"error\":\
    {\"message\":\"No models loaded. Please load a model in the developer page \
    or via `lms load`.\",\"type\":\"invalid_request_error\",\"param\":\"model\"}}";

#[test]
fn local_provider_no_model_loaded_matches_verbatim_dmq_body() {
    assert!(is_local_provider_no_model_loaded(
        StatusCode::BAD_REQUEST,
        DMQ_BODY
    ));
    // Status-gated: the same prose on a non-400 status must not match (the
    // 400 is the local-idle signal; other statuses are different failures).
    assert!(!is_local_provider_no_model_loaded(
        StatusCode::INTERNAL_SERVER_ERROR,
        DMQ_BODY
    ));
    // A generic 400 (malformed request) must stay reportable.
    assert!(!is_local_provider_no_model_loaded(
        StatusCode::BAD_REQUEST,
        "{\"error\":{\"message\":\"invalid 'temperature': must be <= 2\"}}"
    ));
    // The surfaced guidance is actionable (tells the user to load a model).
    assert!(local_provider_no_model_loaded_user_message()
        .to_ascii_lowercase()
        .contains("load a model"));
}

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

/// Verbatim TAURI-RUST-C9A provider body — the Kiro IDE proxy wraps its own
/// 402 monthly-quota refusal inside a 500 envelope. The matcher keys on this
/// prose, so coupling the test to the exact string makes a provider wording
/// drift fail CI rather than silently leak events back to Sentry.
const C9A_BODY: &str = "kiro API error (500 Internal Server Error): \
    {\"error\":{\"message\":\"HTTP 402 from Kiro IDE: {\\\"message\\\":\\\"You have \
    reached the limit.\\\",\\\"reason\\\":\\\"MONTHLY_REQUEST_COUNT\\\"}\",\
    \"type\":\"server_error\"}}";

/// Verbatim TAURI-RUST-AFE Responses-API body — the Codex/ChatGPT OAuth
/// `/responses` endpoint refuses with `usage_limit_reached` once the Plus
/// plan cap is hit. It carries no "monthly"/"quota" co-marker, so the C9A
/// phrase set missed it; couple the test to the exact string so a wording
/// drift fails CI rather than silently leaking events back to Sentry.
const AFE_BODY: &str = "openai Responses API error: {\"error\":{\"type\":\
    \"usage_limit_reached\",\"message\":\"The usage limit has been reached\",\
    \"plan_type\":\"plus\",\"resets_at\":1750000000}}";

#[test]
fn quota_exhausted_matches_verbatim_c9a_body() {
    // Status-agnostic: the verbatim 500-wrapped body must match even though
    // the transport status is 500, not 402.
    assert!(is_provider_quota_exhausted(C9A_BODY));
    assert!(body_indicates_quota_exhausted(C9A_BODY));
}

#[test]
fn rate_cap_exceeded_matches_verbatim_hxf_body_but_not_transient_or_context() {
    // TAURI-RUST-HXF: verbatim groq `on_demand` free-tier 413 — a single
    // request over the per-minute token cap. Status-agnostic; anchored on
    // BOTH "request too large" (single-request permanence) and a
    // tokens-per-minute marker.
    assert!(is_provider_rate_cap_exceeded_message(
        "groq API error (413 Payload Too Large): {\"error\":{\"message\":\"Request too large \
         for model `openai/gpt-oss-120b` in organization `org_x` service tier `on_demand` on \
         tokens per minute (TPM): Limit 8000, Requested 42084.\",\"code\":\"rate_limit_exceeded\"}}"
    ));
    // Transient burst ("try again in Ns") lacks "request too large" → stays
    // retryable + Sentry-visible.
    assert!(!is_provider_rate_cap_exceeded_message(
        "groq API error (429 Too Many Requests): Rate limit reached. Please try again in 2.5s."
    ));
    // Context-window overflow is a different bucket (model size, not a rate
    // cap) — no tokens-per-minute marker.
    assert!(!is_provider_rate_cap_exceeded_message(
        "openai API error (400): This model's maximum context length is 8192 tokens"
    ));
    // A bare 413 with no TPM marker must not match.
    assert!(!is_provider_rate_cap_exceeded_message(
        "openai API error (413 Payload Too Large): request entity too large"
    ));
}

#[test]
fn quota_exhausted_matches_verbatim_afe_body() {
    // Coverage gap closed (TAURI-RUST-AFE): the Responses `usage_limit_reached`
    // body must demote through the same #4076 quota machinery even though it
    // lacks a "monthly"/"quota" co-marker.
    assert!(is_provider_quota_exhausted(AFE_BODY));
    assert!(body_indicates_quota_exhausted(AFE_BODY));
    // Bare phrasings (no surrounding envelope) must also match.
    assert!(body_indicates_quota_exhausted("usage_limit_reached"));
    assert!(body_indicates_quota_exhausted(
        "The usage limit has been reached"
    ));
}

#[test]
fn quota_exhausted_matches_common_phrasings() {
    for body in [
        "{\"reason\":\"MONTHLY_REQUEST_COUNT\"}",
        "You have reached the limit on your monthly requests",
        "monthly request quota reached",
        "monthly limit reached",
        "plan quota exceeded",
        "usage limit exceeded for this period",
    ] {
        assert!(is_provider_quota_exhausted(body), "should match: {body:?}");
    }
}

#[test]
fn quota_exhausted_ignores_unrelated_500_and_rate_limit() {
    // A generic 500 outage and a 429 rate-limit are NOT plan-quota
    // exhaustion and must stay reportable / retryable respectively — the
    // quota guard must not swallow them.
    for body in [
        "kiro API error (500 Internal Server Error): {\"error\":\
         {\"message\":\"upstream connection reset\",\"type\":\"server_error\"}}",
        "rate_limit_exceeded: too many requests, retry after 12s",
        "429 Too Many Requests",
        "context length exceeded: reduce the number of tokens",
    ] {
        assert!(
            !is_provider_quota_exhausted(body),
            "should NOT match: {body:?}"
        );
    }
}

#[test]
fn quota_and_credits_matchers_do_not_overlap_on_c9a() {
    // The 402-gated credits matcher must keep ignoring the 500-wrapped
    // quota body (it is status-anchored) — the quota matcher is the one
    // that catches it. Proves the locked-in
    // `insufficient_credits_402_ignores_non_402_status` invariant holds and
    // the two classifiers cover distinct shapes.
    assert!(!is_provider_insufficient_credits_402(
        StatusCode::INTERNAL_SERVER_ERROR,
        C9A_BODY
    ));
    assert!(is_provider_quota_exhausted(C9A_BODY));
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

/// Verbatim TAURI-RUST-5MV provider body. The matcher keys on the
/// `Internal Server Error (ref:` envelope, so coupling the test to the exact
/// wire shape makes an Ollama-Cloud wording drift fail CI rather than
/// silently leak events back to Sentry.
const OLLAMA_CLOUD_500_BODY: &str =
    "{\"error\":\"Internal Server Error (ref: df512dcb-d915-493b-8f2d-e8d3dfa640c1)\"}";

#[test]
fn ollama_cloud_internal_500_matches_verbatim_5mv_body() {
    assert!(is_ollama_cloud_internal_500(
        "ollama",
        StatusCode::INTERNAL_SERVER_ERROR,
        OLLAMA_CLOUD_500_BODY
    ));
}

#[test]
fn ollama_cloud_internal_500_ignores_non_500_status() {
    // Same body on a non-500 status is not this provider-internal flood —
    // keep it reportable so a genuine bug elsewhere isn't masked.
    assert!(!is_ollama_cloud_internal_500(
        "ollama",
        StatusCode::BAD_REQUEST,
        OLLAMA_CLOUD_500_BODY
    ));
    assert!(!is_ollama_cloud_internal_500(
        "ollama",
        StatusCode::SERVICE_UNAVAILABLE,
        OLLAMA_CLOUD_500_BODY
    ));
}

#[test]
fn ollama_cloud_internal_500_ignores_other_providers() {
    // A 500 with the same envelope from a non-ollama provider stays
    // reportable — this gate is scoped to ollama.com hosted inference.
    assert!(!is_ollama_cloud_internal_500(
        "openai",
        StatusCode::INTERNAL_SERVER_ERROR,
        OLLAMA_CLOUD_500_BODY
    ));
}

#[test]
fn ollama_cloud_internal_500_ignores_local_ollama_500_without_ref() {
    // A local Ollama daemon 500 (genuine model crash / OOM, worth paging)
    // does not carry the `ref:` UUID, so it must NOT be swallowed.
    assert!(!is_ollama_cloud_internal_500(
        "ollama",
        StatusCode::INTERNAL_SERVER_ERROR,
        "{\"error\":\"llama runner process has terminated: exit status 0xc0000409\"}"
    ));
}

#[test]
fn ollama_cloud_internal_500_user_message_is_matched_by_message_matcher() {
    // Couple the prose builder to the re-report matcher so the
    // `expected_error_kind` / before_send demotion can't drift from the
    // string the emit sites actually raise.
    let with_model = ollama_cloud_internal_500_user_message(
        Some("minimax-m3:cloud"),
        StatusCode::INTERNAL_SERVER_ERROR,
    );
    assert!(with_model.contains("minimax-m3:cloud"));
    assert!(!with_model.contains("ref:"));
    assert!(is_ollama_cloud_internal_500_message(&with_model));

    let without_model =
        ollama_cloud_internal_500_user_message(None, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(is_ollama_cloud_internal_500_message(&without_model));
}

#[test]
fn log_ollama_cloud_internal_500_smoke() {
    // The helper only emits a demotion info log; calling it covers that path.
    log_ollama_cloud_internal_500(
        "native_chat",
        "ollama",
        Some("minimax-m3:cloud"),
        StatusCode::INTERNAL_SERVER_ERROR,
    );
}

/// Verbatim TAURI-RUST-ECR provider body — an external content-moderation
/// proxy ("Ombudsman") refuses the triage agent's prompt with a 400 +
/// verdict envelope. The matcher keys on this prose / verdict field, so
/// coupling the test to the exact wire shape makes a proxy wording drift
/// fail CI rather than silently leak the 4,517-event flood back to Sentry.
const ECR_OMBUDSMAN_BODY: &str = "{\"error\":\"Message rejected by Ombudsman\",\"score\":80}";

#[test]
fn moderation_rejection_matches_verbatim_ecr_body() {
    // The 400 Ombudsman refusal must be demoted (classified), not reported.
    assert!(is_provider_moderation_rejection_http_400(
        StatusCode::BAD_REQUEST,
        ECR_OMBUDSMAN_BODY
    ));
}

#[test]
fn moderation_rejection_matches_marker_variants() {
    for body in [
        "{\"error\":\"Message rejected by Ombudsman\",\"score\":80}",
        "{\"error\":\"message rejected by moderation\"}",
        "{\"verdict\":\"blocked\",\"score\":0.97}",
        "Request blocked by Ombudsman policy",
    ] {
        assert!(
            is_provider_moderation_rejection_http_400(StatusCode::BAD_REQUEST, body),
            "should match: {body:?}"
        );
    }
}

#[test]
fn moderation_rejection_ignores_non_400_status() {
    // Same body on a non-400 status is not this external-guard refusal —
    // keep it reportable so a genuine bug elsewhere isn't masked.
    assert!(!is_provider_moderation_rejection_http_400(
        StatusCode::INTERNAL_SERVER_ERROR,
        ECR_OMBUDSMAN_BODY
    ));
    assert!(!is_provider_moderation_rejection_http_400(
        StatusCode::FORBIDDEN,
        ECR_OMBUDSMAN_BODY
    ));
}

#[test]
fn moderation_rejection_ignores_unrelated_400() {
    // A genuine malformed-request 400 (no moderation verdict / score field)
    // must keep reaching Sentry — the gate must not swallow it.
    for body in [
        "{\"error\":{\"message\":\"invalid request: missing field `messages`\"}}",
        "{\"error\":\"Bad Request\"}",
        "{\"error\":{\"message\":\"unknown model 'gemma3:1b'\"}}",
    ] {
        assert!(
            !is_provider_moderation_rejection_http_400(StatusCode::BAD_REQUEST, body),
            "should NOT match: {body:?}"
        );
    }
}

#[test]
fn log_provider_moderation_rejection_smoke() {
    // The helper only emits a demotion info log; calling it covers that path.
    log_provider_moderation_rejection(
        "native_chat",
        "ollama",
        Some("gemma3:1b-it-qat"),
        StatusCode::BAD_REQUEST,
    );
}

#[test]
fn openai_oauth_session_expired_excludes_backend_provider() {
    // The OpenHuman backend owns app-session expiry via
    // `publish_backend_session_expired`; this provider-OAuth gate must not
    // claim a backend 401.
    assert!(!is_openai_oauth_session_expired_http(
        openhuman_backend_model::PROVIDER_LABEL,
        StatusCode::UNAUTHORIZED,
        OAUTH_EXPIRED_8FQ_BODY
    ));
}

/// Verbatim TAURI-RUST-4RC OpenRouter body. The matcher keys on the
/// `"user not found"` prose, so coupling the test to the exact payload
/// makes a wording drift fail CI rather than silently leak the 401 flood
/// (~9k events / 6 users) back to Sentry.
const OPENROUTER_USER_NOT_FOUND_4RC_BODY: &str =
    "{\"error\":{\"message\":\"User not found.\",\"code\":401}}";

#[test]
fn byo_auth_failure_matches_openrouter_user_not_found_401() {
    assert!(is_byo_provider_auth_failure_http(
        "openrouter",
        StatusCode::UNAUTHORIZED,
        OPENROUTER_USER_NOT_FOUND_4RC_BODY
    ));
}

#[test]
fn byo_auth_failure_user_not_found_ignores_non_auth_status() {
    // Same prose on a non-401/403 status is not this user-state — keep it
    // reportable so an unrelated "user not found" elsewhere isn't masked.
    assert!(!is_byo_provider_auth_failure_http(
        "openrouter",
        StatusCode::NOT_FOUND,
        OPENROUTER_USER_NOT_FOUND_4RC_BODY
    ));
    assert!(!is_byo_provider_auth_failure_http(
        "openrouter",
        StatusCode::INTERNAL_SERVER_ERROR,
        OPENROUTER_USER_NOT_FOUND_4RC_BODY
    ));
}

#[test]
fn byo_auth_failure_user_not_found_excludes_backend_provider() {
    // A backend 401 is app-session expiry (handled by
    // `publish_backend_session_expired`), never a BYO key — even if the
    // body happens to carry the same prose.
    assert!(!is_byo_provider_auth_failure_http(
        openhuman_backend_model::PROVIDER_LABEL,
        StatusCode::UNAUTHORIZED,
        OPENROUTER_USER_NOT_FOUND_4RC_BODY
    ));
}

#[test]
fn byo_auth_failure_user_not_found_is_openrouter_gated() {
    // `"user not found"` is OpenRouter-specific prose, NOT a global auth
    // marker. A different BYO provider returning a 401 whose body happens
    // to contain that phrase must keep its original (reported) error path
    // — demoting it would suppress a real failure and surface the wrong
    // "update your key" remediation. Only OpenRouter's wording is anchored.
    assert!(!is_byo_provider_auth_failure_http(
        "anthropic",
        StatusCode::UNAUTHORIZED,
        OPENROUTER_USER_NOT_FOUND_4RC_BODY
    ));
    // The canonical auth markers still match regardless of provider.
    assert!(is_byo_provider_auth_failure_http(
        "anthropic",
        StatusCode::UNAUTHORIZED,
        "{\"error\":{\"type\":\"authentication_error\"}}"
    ));
}
