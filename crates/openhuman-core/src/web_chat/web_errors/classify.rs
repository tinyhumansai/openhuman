//! The chat-error classification ladder: turning a flattened inference error
//! string into a [`ClassifiedError`] envelope the frontend can render a
//! recovery UI from.

use super::backend_error_code::classify_by_backend_error_code;
use super::budget::{is_action_budget_exhausted, is_inference_budget_exceeded_error};
use super::retry::{is_non_retryable_rate_limit_text, retry_after_hint};
use super::timeout::is_turn_timeout_error;
use crate::inference::failure_copy::{failure_copy, FailureClass};
use tinyinference_llm::failure::{
    extract_provider_error_detail, extract_provider_name, is_auth_error_text,
    is_codex_token_expired_text, is_connection_dropped_text, is_context_length_text,
    is_empty_provider_response_text, is_fallback_chain_exhausted, is_malformed_tool_history_text,
    is_model_unavailable_text, is_payment_required_text, is_provider_request_rejected_text,
    is_rate_limit_text, is_server_error_text, is_timeout_text, is_transient_unavailability_text,
    is_vision_unsupported_text, parse_retry_after_secs, with_provider_detail,
};

/// Structured chat-error envelope produced by [`classify_inference_error`].
///
/// Carries the typed metadata the frontend needs to render a recovery UI
/// (retry-after countdown, retry button, fallback CTA) without having to
/// regex the human-readable `message`. Issue #2606.
///
/// `error_type` and `message` preserve the wire shape PR #2371 established
/// — existing FE handlers that read those fields keep working. The new
/// fields are additive and `Option`-typed where the value isn't always
/// known at the classifier layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClassifiedError {
    /// Stable token: `rate_limited`, `action_budget_exceeded`,
    /// `max_iterations`, `turn_timeout`, `timeout`, `auth_error`,
    /// `session_expired`, `budget_exhausted`, `provider_error`,
    /// `context_overflow`, `model_unavailable`, `payload_too_large`,
    /// `provider_request_rejected`, `capability_unsupported`,
    /// `chat_template_rejected`, `empty_response`, `network`, `inference`.
    pub(crate) error_type: &'static str,
    /// User-facing copy (already includes provider detail block and the
    /// retry-after countdown sentence when available).
    pub(crate) message: String,
    /// Where the limit originated. One of:
    /// - `"provider"`         — upstream LLM provider 429 / rate limit
    /// - `"openhuman_budget"` — local SecurityPolicy per-hour action cap
    /// - `"agent_loop"`       — agent ran out of tool iterations
    /// - `"openhuman_billing"` — OpenHuman credit/quota exhaustion
    /// - `"transport"`        — network / DNS / TLS / timeout
    /// - `"config"`           — auth, model, context, generic
    pub(crate) source: &'static str,
    /// Can the user retry the same prompt in the same thread? `false` for
    /// non-retryable business 429s, auth failures, model_unavailable,
    /// context_overflow, and OpenHuman billing exhaustion.
    pub(crate) retryable: bool,
    /// Milliseconds the upstream asked us to wait. Surfaced verbatim from
    /// `Retry-After:` / `retry_after:` headers when present; `None` when
    /// the upstream didn't supply one OR the error class doesn't have a
    /// concept of retry-after (auth, config, etc.).
    pub(crate) retry_after_ms: Option<u64>,
    /// Provider name extracted from the leading
    /// `"<provider> API error (...)"` envelope emitted by
    /// `inference::provider::ops::api_error`. `None` for non-provider
    /// errors (OpenHuman budget cap, agent loop) and for transport
    /// failures that don't carry an identifiable provider prefix.
    pub(crate) provider: Option<String>,
    /// `Some(false)` once the reliable-provider chain has exhausted every
    /// configured `model_fallbacks` entry (the aggregate "All
    /// providers/models failed" branch). `None` means the classifier
    /// can't tell from the error string alone — the FE should treat it
    /// as "unknown, don't promise a fallback".
    pub(crate) fallback_available: Option<bool>,
    /// Stable i18n key of the table row (`chat_error.<class>`), sent as
    /// `copy_key` so the frontend can render `message` in the user's locale.
    pub(crate) copy_key: &'static str,
    /// Values the translated copy needs, sent as `copy_params`:
    /// `retry_after_secs` (the countdown sentence), `provider`, and `detail`
    /// (the sanitized provider error quoted under the copy). `None` when the
    /// row has none.
    pub(crate) copy_params: Option<serde_json::Value>,
}

/// Build the `copy_params` object; `None` when every value is absent.
pub(super) fn copy_params(
    provider: Option<&str>,
    retry_after_secs: Option<u64>,
    detail: Option<String>,
) -> Option<serde_json::Value> {
    let mut params = serde_json::Map::new();
    if let Some(secs) = retry_after_secs {
        params.insert("retry_after_secs".to_string(), secs.into());
    }
    if let Some(provider) = provider {
        params.insert("provider".to_string(), provider.into());
    }
    if let Some(detail) = detail {
        params.insert("detail".to_string(), detail.into());
    }
    (!params.is_empty()).then_some(serde_json::Value::Object(params))
}

/// Build the envelope for `class` from the copy table. `message` is the row's
/// copy with whatever suffix the arm adds (provider detail, retry-after hint).
pub(super) fn classified(
    class: FailureClass,
    message: String,
    provider: Option<String>,
    fallback_available: Option<bool>,
) -> ClassifiedError {
    let row = failure_copy(class);
    ClassifiedError {
        error_type: row.error_type,
        message,
        source: row.source,
        retryable: row.retryable,
        retry_after_ms: None,
        copy_params: copy_params(provider.as_deref(), None, None),
        provider,
        fallback_available,
        copy_key: row.key,
    }
}

/// [`classified`] with the row's copy and the provider detail block appended.
fn classified_with_detail(
    class: FailureClass,
    err: &str,
    provider: Option<String>,
    fallback_available: Option<bool>,
) -> ClassifiedError {
    let message = with_provider_detail(failure_copy(class).copy, err);
    ClassifiedError {
        copy_params: copy_params(
            provider.as_deref(),
            None,
            extract_provider_error_detail(err),
        ),
        ..classified(class, message, provider, fallback_available)
    }
}

/// [`classified`] with the row's copy verbatim.
pub(super) fn classified_plain(
    class: FailureClass,
    provider: Option<String>,
    fallback_available: Option<bool>,
) -> ClassifiedError {
    classified(
        class,
        failure_copy(class).copy.to_string(),
        provider,
        fallback_available,
    )
}

/// Whether `err` says nothing except that the offline local session cannot
/// reach managed inference (#6932).
///
/// A fallback aggregate concatenates every attempt, so the sentinel being
/// present does not mean it is the whole story: one leg can be this refusal
/// while another is a BYO provider failing on its own key. Only an error whose
/// every attempt is the refusal is explained by the local profile.
fn only_local_session_refused(err: &str) -> bool {
    const AGGREGATE: &str = "All providers/models failed";
    let sentinel =
        crate::security::credentials::session_support::LOCAL_SESSION_MANAGED_INFERENCE_UNAVAILABLE;
    if !err.contains(sentinel) {
        return false;
    }
    match err.split_once(AGGREGATE) {
        // Not a chain aggregate: the refusal is the only failure there is.
        None => true,
        Some((_, attempts)) => attempts
            .split(';')
            .map(str::trim)
            .filter(|attempt| !attempt.is_empty())
            .all(|attempt| attempt.contains(sentinel)),
    }
}

pub(crate) fn classify_inference_error(err: &str) -> ClassifiedError {
    use FailureClass as C;
    let lower = err.to_lowercase();
    let provider = extract_provider_name(err);
    let fallback_available = if is_fallback_chain_exhausted(err) {
        Some(false)
    } else {
        None
    };

    // F2: when the managed backend stamped a stable `errorCode` on the body,
    // trust it and branch on it FIRST — ignoring the substring heuristics
    // below, which are tuned for the BYO / direct-provider path (no
    // `errorCode`). Only a recognised code short-circuits; an absent or
    // unrecognised code falls through to the substring ladder unchanged, so
    // the BYO "check your API key / model settings" copy stays intact.
    if let Some(classified) =
        classify_by_backend_error_code(err, provider.clone(), fallback_available)
    {
        log::debug!(
            "[chat-error][classify] error_type={} source={} retryable={} provider={:?} (via errorCode)",
            classified.error_type,
            classified.source,
            classified.retryable,
            classified.provider,
        );
        return classified;
    }

    // Order matters: the SecurityPolicy hourly cap and the agent-loop
    // max-iterations error both surface as strings that contain "rate limit" /
    // "iteration", so they MUST be checked before the generic provider-429
    // branch — otherwise users see a confusing "your AI provider is
    // rate-limiting you" message for limits OpenHuman itself enforced (#2364).
    let classified = if only_local_session_refused(err) {
        // #6932: `resolve_bearer` refuses the offline local session before the
        // request, so the backend `401 "Invalid token"` that
        // `is_session_expired_message` claims — and that told a locally
        // signed-in user their session had expired — never comes back.
        //
        // Leads the ladder, because the aggregate wrapper's own words defeat
        // the arms below: "All providers/models failed" supplies "models" and
        // the sentinel supplies "unavailable", which is exactly
        // `is_model_unavailable_text`, so a refusal reaching the lower arms
        // gets told to check its model settings. The gate is what keeps that
        // lead honest — a chain that refused managed and then failed a BYO
        // provider on its own key is NOT claimed here and reports that key
        // instead.
        classified_plain(
            C::LocalSessionManagedUnavailable,
            provider,
            fallback_available,
        )
    } else if is_codex_token_expired_text(&err.to_ascii_lowercase()) {
        // Codex OAuth refresh failed (#5869): the user must reconnect Codex in
        // Settings → Integrations, NOT sign into OpenHuman. Checked before
        // `is_session_expired_message` because the sentinel contains
        // "authentication token is expired", which the broader "session
        // expired" test would also match. Requires "codex" so a generic
        // provider `token_expired` is not misread as a Codex failure.
        classified_plain(
            C::CodexSessionExpired,
            Some("openai_codex".to_string()),
            None,
        )
    } else if crate::core::observability::is_session_expired_message(err) {
        // The OpenHuman app-session JWT expired. There is NO client-side
        // refresh — recovery is an interactive re-auth — so non-retryable, and
        // it must route to sign-in. Checked after the Codex arm; the
        // `auth_error` arm below can't claim the backend's `401 "Invalid
        // token"` envelope (it contains "401") and mislead managed-backend
        // users with "check your API key". A BYO provider's own 401 still
        // falls through to `auth_error`. Provider name is irrelevant to a
        // sign-in prompt.
        classified_plain(C::SessionExpired, None, None)
    } else if is_action_budget_exhausted(&lower) {
        // OpenHuman's own cap (#2364): the window decays gradually so the same
        // thread CAN recover, but the exact wait is unpredictable. The limit is
        // not from a provider, so any provider name in the chain is dropped.
        classified_with_detail(C::ActionBudget, err, None, None)
    } else if tinyagents_harness::agent_error::is_max_iterations_error(err) {
        classified_with_detail(C::MaxIterations, err, provider, None)
    } else if is_turn_timeout_error(err) {
        // The web turn driver's wall-clock backstop fired (#4746): a wedged main
        // agent or a delegated sub-agent that never returned. Anchored next to
        // max_iterations — both are deterministic agent-loop outcomes that must
        // not be shadowed by the broad provider-429 / 5xx arms below. No
        // provider detail: the marker string carries no provider body.
        classified_plain(C::TurnTimeout, None, None)
    } else if is_empty_provider_response_text(&lower) {
        // `AgentError::EmptyProviderResponse`, flattened at the native-bus
        // boundary (Sentry TAURI-RUST-4JW, the largest source of the
        // #3092 / #3119 cluster). An empty successful completion is not a
        // billing verdict — reasoning-only replies can spend output tokens and
        // show nothing — so no credit guidance and no provider detail.
        classified_plain(C::EmptyResponse, None, None)
    } else if tinyinference_llm::providers::openai::is_chat_template_rejection_message(err) {
        // #5291: a local runtime rendered the request through the model's OWN
        // Jinja chat template and the template raised. Placed above the broad
        // substring ladder because both renderings of this failure are claimed
        // by arms that misdiagnose them ("check your API key", "provider
        // rejected the model or temperature"), pointing at things that were
        // never the problem. The anchors are template-engine strings no
        // unrelated provider error emits. Retryable: a fresh turn starts from
        // the user's new message.
        classified_with_detail(C::ChatTemplateRejected, err, provider, fallback_available)
    } else if is_rate_limit_text(&lower) {
        let retry_secs = parse_retry_after_secs(err);
        // Non-retryable business 429s ("plan does not include", balance
        // exhausted, Z.AI 1311/1113) also surface here — mark them
        // non-retryable so the FE can hide "Retry" and route to billing.
        let non_retryable = is_non_retryable_rate_limit_text(&lower);
        let (class, summary) = if non_retryable {
            (
                C::RateLimitedBilling,
                failure_copy(C::RateLimitedBilling).copy.to_string(),
            )
        } else {
            (
                C::RateLimited,
                format!(
                    "{}{}",
                    failure_copy(C::RateLimited).copy,
                    retry_after_hint(retry_secs)
                ),
            )
        };
        ClassifiedError {
            retry_after_ms: retry_secs.map(|s| s.saturating_mul(1000)),
            copy_params: copy_params(
                provider.as_deref(),
                retry_secs.filter(|_| !non_retryable),
                extract_provider_error_detail(err),
            ),
            ..classified(
                class,
                with_provider_detail(summary.as_str(), err),
                provider,
                fallback_available,
            )
        }
    } else if is_timeout_text(&lower) {
        classified_with_detail(C::Timeout, err, provider, fallback_available)
    } else if is_auth_error_text(&lower) {
        classified_with_detail(C::AuthError, err, provider, None)
    } else if is_payment_required_text(&lower)
        // Issue #3088: the managed backend reports no-credits as a 400 with
        // "Insufficient budget" (not a 402), which previously fell through to
        // the generic apology. Catch the canonical budget phrases here.
        || is_inference_budget_exceeded_error(err)
    {
        // `openhuman_billing` means OpenHuman's own credit system — a 402 with
        // the "openhuman" envelope (or none). When it comes from an upstream
        // provider envelope the limit belongs to that provider.
        let source: &'static str = match provider.as_deref() {
            Some("openhuman") | None => failure_copy(C::BudgetExhausted).source,
            Some(_) => "provider",
        };
        ClassifiedError {
            source,
            ..classified_with_detail(C::BudgetExhausted, err, provider, None)
        }
    } else if is_server_error_text(&lower) {
        classified_with_detail(C::ProviderUnavailable, err, provider, fallback_available)
    } else if is_context_length_text(&lower) {
        classified_with_detail(C::ContextOverflow, err, provider, None)
    } else if tinyinference_providers::is_provider_config_rejection_message(err) {
        // #2079 / #2076 / #2202: an abstract tier alias leaked to a custom
        // provider, a stale model pin, or a model-specific temperature
        // constraint. Checked BEFORE the generic model-unavailable arm so
        // config-rejection bodies that also contain "model" / "does not exist"
        // get the specific Settings remediation.
        classified_with_detail(C::ModelConfigRejected, err, provider, None)
    } else if is_model_unavailable_text(&lower) {
        // #5503: split on transience. A temporary outage ("currently
        // overloaded") is not a user misconfiguration — it routes to the
        // retryable provider copy; a genuine model rejection ("does not exist",
        // a stale pin) keeps the non-retryable config copy. Config-rejection
        // bodies are claimed by the arm above and never reach here.
        if is_transient_unavailability_text(&lower) {
            classified_with_detail(C::ProviderUnavailable, err, provider, fallback_available)
        } else {
            classified_with_detail(C::ModelUnavailable, err, provider, None)
        }
    } else if is_vision_unsupported_text(&lower) {
        // A multimodal turn sent image markers to a text-only model. Retrying
        // the same image against the same model can't help.
        classified_plain(C::CapabilityUnsupported, None, None)
    } else if is_provider_request_rejected_text(&lower) && is_malformed_tool_history_text(&lower) {
        // Poisoned-history rejection on a BYO/direct provider. The de-poison
        // guard already evicted the warm session, so resending works. Checked
        // BEFORE the generic 4xx arm so the actionable copy wins.
        classified_plain(C::MalformedHistory, provider, fallback_available)
    } else if is_provider_request_rejected_text(&lower) {
        // A 4xx none of the specific arms above claimed (generic 400, 404,
        // 422; the DeepSeek `reasoning_content` round-trip 400, #3197). MUST
        // stay below provider-config-rejection and model-unavailable so their
        // more specific verdicts win. Identical retry fails, so non-retryable;
        // the real reason is secret-scrubbed and quoted by
        // `with_provider_detail`.
        classified_with_detail(C::RequestRejected, err, provider, fallback_available)
    } else if is_connection_dropped_text(&lower) {
        // A transport-level drop with no provider status and no managed
        // `errorCode` (stale keep-alive after sleep/wake, a raw mid-stream SSE
        // drop). The turn's history is NOT poisoned, so it is cleanly
        // retryable. Placed LAST so every specific status / 4xx arm claims its
        // shape first.
        classified_with_detail(C::Network, err, provider, fallback_available)
    } else if is_transient_unavailability_text(&lower) {
        // A transient outage marker no more specific arm claimed (a bare 5xx
        // "overloaded" such as Anthropic's 529, "please retry later") — the
        // retryable "temporarily unavailable" copy rather than the flat
        // inference bucket (#5503).
        classified_plain(C::ProviderUnavailable, provider, fallback_available)
    } else {
        classified_with_detail(C::Inference, err, provider, fallback_available)
    };

    // Verbose diagnostics on the classification flow (per CLAUDE.md). Stable
    // grep-friendly prefix + low-cardinality fields only — the raw `err` (which
    // may carry provider payload / PII) is intentionally NOT logged here; the
    // caller (`web.rs::run_chat_task`) already records it at warn level and
    // routes it through `report_error_or_expected`.
    log::debug!(
        "[chat-error][classify] error_type={} source={} retryable={} provider={:?}",
        classified.error_type,
        classified.source,
        classified.retryable,
        classified.provider,
    );

    classified
}
