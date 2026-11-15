//! Sentinels the TinyHumans layer stamps on its backend's refusals, and the
//! classifier that tells the core what they mean.
//!
//! The hosted client flattens a typed backend refusal onto one of these
//! prefixes (`client::error`, `hosted::client`, `voice`). The core never
//! produces or names them: [`register`] installs [`expected_error_kind`] in the
//! core's expected-error classifier slot, so `report_error_or_expected` demotes
//! them like any other user-state failure. Wire strings are stable — the UI and
//! log searches key on them.

use std::sync::Arc;

use openhuman_embed::__host::core::observability::{
    register_expected_error_classifier, ExpectedErrorKind,
};

/// Sentinel prefix on the error string a backend call returns when the backend
/// rejects the stored TinyHumans API key. Demoted to
/// [`ExpectedErrorKind::SessionExpired`]: the fix is a new key, not a code
/// change. Deliberately not a session-expiry marker for the `SessionExpired`
/// publish: an API key has no session to clear.
pub const API_KEY_REJECTED_PREFIX: &str = "API_KEY_REJECTED:";

/// Sentinel prefix on the error string a backend call returns when the backend
/// refuses a gated hosted feature because the account balance is too low (an
/// `errorCode: USER_INSUFFICIENT_CREDITS` response, or a 403 asking the user to
/// add credits). Demoted to [`ExpectedErrorKind::BudgetExhausted`]: the fix is
/// a top-up, not a code change.
pub const INSUFFICIENT_CREDITS_PREFIX: &str = "INSUFFICIENT_CREDITS:";

/// Sentinel prefix on the error string a backend call returns when the backend
/// refuses a hosted feature that needs a paid plan (a 403). Demoted like
/// [`INSUFFICIENT_CREDITS_PREFIX`].
pub const PAID_PLAN_REQUIRED_PREFIX: &str = "PAID_PLAN_REQUIRED:";

/// The id the classifier registers under.
const CLASSIFIER_ID: &str = "tinyhumans";

/// Whether `msg` carries the [`INSUFFICIENT_CREDITS_PREFIX`] or
/// [`PAID_PLAN_REQUIRED_PREFIX`] sentinel anywhere in its chain.
pub fn is_hosted_billing_refusal_message(msg: &str) -> bool {
    msg.contains(INSUFFICIENT_CREDITS_PREFIX) || msg.contains(PAID_PLAN_REQUIRED_PREFIX)
}

/// Whether `msg` carries the [`API_KEY_REJECTED_PREFIX`] sentinel anywhere in
/// its chain.
pub fn is_api_key_rejected_message(msg: &str) -> bool {
    msg.contains(API_KEY_REJECTED_PREFIX)
}

/// What the core should treat one of this layer's sentinels as.
///
/// - [`API_KEY_REJECTED_PREFIX`] → `SessionExpired` (the credential-lapse
///   bucket: the key was revoked or mistyped and only a new key recovers it).
/// - [`INSUFFICIENT_CREDITS_PREFIX`] / [`PAID_PLAN_REQUIRED_PREFIX`] →
///   `BudgetExhausted` (TAURI-RUST-13BR / -11G3: a gated hosted feature such
///   as live voice refused for the balance or the plan).
pub fn expected_error_kind(message: &str) -> Option<ExpectedErrorKind> {
    if is_api_key_rejected_message(message) {
        return Some(ExpectedErrorKind::SessionExpired);
    }
    if is_hosted_billing_refusal_message(message) {
        return Some(ExpectedErrorKind::BudgetExhausted);
    }
    None
}

/// Installs [`expected_error_kind`] in the core's classifier slot
/// (idempotent).
pub(crate) fn register() {
    log::debug!("[tinyhumans] errors: registering the sentinel classifier");
    register_expected_error_classifier(CLASSIFIER_ID, Arc::new(expected_error_kind));
}

#[cfg(test)]
#[path = "errors_tests.rs"]
mod tests;
