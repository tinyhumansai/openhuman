use super::*;
use openhuman_embed::__host::core::observability::{
    expected_error_kind as core_expected_error_kind, is_session_expired_error,
    is_session_expired_message,
};

#[test]
fn classifies_hosted_billing_refusals_as_budget_exhausted() {
    register();
    // TAURI-RUST-13BR / -11G3: the hosted client flattens a live-voice billing
    // refusal onto these sentinels. The Gemini reserve message matches no
    // budget phrase, so only the sentinel keeps it out of Sentry.
    let credits = format!(
        "{INSUFFICIENT_CREDITS_PREFIX}POST /live failed (400): {{\"error\":\"Gemini Live requires an available balance of at least $1.00 (reserved per open session)\",\"errorCode\":\"USER_INSUFFICIENT_CREDITS\"}}"
    );
    let plan = format!("{PAID_PLAN_REQUIRED_PREFIX}GET /signed-url failed (403): {{}}");
    for msg in [credits, plan] {
        assert!(is_hosted_billing_refusal_message(&msg), "{msg}");
        assert_eq!(
            expected_error_kind(&msg),
            Some(ExpectedErrorKind::BudgetExhausted),
            "{msg}"
        );
        assert_eq!(
            core_expected_error_kind(&msg),
            Some(ExpectedErrorKind::BudgetExhausted),
            "the registered classifier reaches the core: {msg}"
        );
    }
    // A bare mention of the word is not the sentinel.
    assert!(!is_hosted_billing_refusal_message(
        "insufficient_credits counter overflowed"
    ));
}

#[test]
fn classifies_api_key_rejected_as_expected_credential_lapse() {
    register();
    // A 401 on a TinyHumans API-key credential flattens to the
    // `API_KEY_REJECTED:` sentinel (`client::error`). The remedy is a new key,
    // so it must not reach Sentry as an RPC error.
    let msg = format!("{API_KEY_REJECTED_PREFIX} backend rejected api key on GET /teams/me/usage");
    assert!(is_api_key_rejected_message(&msg));
    assert_eq!(
        core_expected_error_kind(&msg),
        Some(ExpectedErrorKind::SessionExpired)
    );
    // …but it is not a session expiry: the JSON-RPC publish boundary keys off
    // the session-expiry predicate, which must stay false so a bad key never
    // clears a signed-in session.
    assert!(!is_session_expired_message(&msg));
    crate::session::expiry::register();
    assert!(!is_session_expired_error(&msg));
}

#[test]
fn unrelated_messages_are_not_claimed() {
    for msg in [
        "",
        "BACKEND_UNAVAILABLE: no transport",
        "GET /x failed (500)",
    ] {
        assert_eq!(expected_error_kind(msg), None, "{msg}");
    }
}
