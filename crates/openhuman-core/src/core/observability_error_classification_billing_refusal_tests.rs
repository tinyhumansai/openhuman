use super::*;

#[test]
fn classifies_hosted_billing_refusals_as_budget_exhausted() {
    // TAURI-RUST-13BR / -11G3: the hosted client flattens a live-voice billing
    // refusal onto these sentinels. The Gemini reserve message matches no
    // budget phrase, so only the sentinel keeps it out of Sentry.
    let credits = format!(
        "{INSUFFICIENT_CREDITS_PREFIX}POST /live failed (400): {{\"error\":\"Gemini Live requires an available balance of at least $1.00 (reserved per open session)\",\"errorCode\":\"USER_INSUFFICIENT_CREDITS\"}}"
    );
    let plan = format!("{PAID_PLAN_REQUIRED_PREFIX}GET /signed-url failed (403): {{}}");
    for msg in [credits, plan] {
        assert!(is_hosted_billing_refusal_message(&msg), "{msg}");
        assert!(
            matches!(
                expected_error_kind(&msg),
                Some(ExpectedErrorKind::BudgetExhausted)
            ),
            "{msg}"
        );
    }
    // A bare mention of the word is not the sentinel.
    assert!(!is_hosted_billing_refusal_message(
        "insufficient_credits counter overflowed"
    ));
}
