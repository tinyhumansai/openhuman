//! Applying registered provider-owned semantic metadata before generic copy
//! heuristics in [`classify_inference_error`](super::classify::classify_inference_error).

use super::classify::ClassifiedError;
use super::retry::retry_after_hint;
use tinyinference_llm::failure::{is_malformed_tool_history_text, parse_retry_after_secs};

/// Apply registered provider-owned semantic metadata when present.
///
/// Returns `Some` only when a registered provider classifier claims the error
/// and supplies a known semantic kind. Provider-specific token parsing stays
/// with that provider; the core uses only the provider-neutral classification.
///
/// `None` falls through to generic message heuristics, including unknown or
/// unowned provider errors.
pub(super) fn classify_by_backend_error_code(
    err: &str,
    provider: Option<String>,
    fallback_available: Option<bool>,
) -> Option<ClassifiedError> {
    use crate::inference::registry::{classify_backend_error, BackendErrorKind};

    // The provider classifier owns this distinction. Message-shaped lookalikes
    // alone never claim provider ownership.
    let classification = classify_backend_error(None, err)?;
    if !classification.provider_owned {
        return None;
    }
    let kind = classification.kind?;

    // Log only the semantic kind; the raw error may carry provider payload or
    // user data.
    log::debug!(
        "[chat-error][classify][provider] kind={:?} provider={:?}",
        kind,
        provider,
    );

    use super::classify::{classified, classified_plain, copy_params};
    use crate::inference::failure_copy::{failure_copy, FailureClass as C};

    let classified = match kind {
        BackendErrorKind::RateLimited => {
            let retry_secs = parse_retry_after_secs(err);
            ClassifiedError {
                retry_after_ms: retry_secs.map(|s| s.saturating_mul(1000)),
                copy_params: copy_params(provider.as_deref(), retry_secs, None),
                ..classified(
                    C::ManagedRateLimited,
                    format!(
                        "{}{}",
                        failure_copy(C::ManagedRateLimited).copy,
                        retry_after_hint(retry_secs)
                    ),
                    provider,
                    fallback_available,
                )
            }
        }
        BackendErrorKind::InsufficientCredits => {
            classified_plain(C::ManagedBudgetExhausted, provider, None)
        }
        // Operator fault (our key/account/quota/5xx) OR operator registry /
        // routing misconfig — NOT user-actionable. Both route to the same
        // calm "we've been notified" copy; the backend already paged. We
        // deliberately DROP the "check your API key" (F4) and "pick a
        // different model" (F6) copy the BYO substring arms would emit.
        BackendErrorKind::UpstreamUnavailable | BackendErrorKind::ModelUnavailable => {
            classified_plain(C::ManagedUnavailable, provider, fallback_available)
        }
        BackendErrorKind::PayloadTooLarge => classified_plain(C::PayloadTooLarge, provider, None),
        BackendErrorKind::ContextLengthExceeded => {
            classified_plain(C::ContextOverflow, provider, None)
        }
        BackendErrorKind::BadRequest => {
            // Same code, three shapes. FIRST: a tool-ordering rejection
            // (`validateToolMessageOrdering` — an orphaned `role:'tool'` message
            // with no matching assistant `tool_call`) is *poisoned history*, not
            // a model/param problem. The de-poison guard in `run_task.rs` has
            // already evicted the offending warm session by the time this copy
            // is built, so the next turn cold-boots clean — tell the user
            // exactly that (and mark retryable, because resending now works).
            if is_malformed_tool_history_text(&err.to_lowercase()) {
                classified_plain(C::MalformedHistory, provider, None)
            // Else two shapes (B8/F8): a backend-flagged *malformed*
            // payload is a client bug (the request was built wrong — it pages
            // Sentry at the FE layer, gated elsewhere), while a plain
            // user-parameter rejection is a model/param mismatch the user can
            // fix. The copy differs: don't tell the user to abandon the thread
            // for a one-off malformation (only this turn failed).
            } else if classification.malformed_bad_request {
                classified_plain(C::ManagedMalformedRequest, provider, None)
            } else {
                classified_plain(C::ManagedRequestRejected, provider, None)
            }
        }
        // Backend already paged its own 500; the FE must not double-report
        // (gated in the Sentry classifier) and the user just retries.
        BackendErrorKind::Internal => {
            classified_plain(C::ManagedInternal, provider, fallback_available)
        }
    };

    Some(classified)
}
