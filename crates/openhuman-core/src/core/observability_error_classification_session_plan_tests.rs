use super::*;

#[test]
fn memory_plan_refusal_is_backend_user_state_not_session_expiry() {
    // #8081: memory not in the account's plan is user state for Sentry, and
    // must not read as an expired session (which signs the user out).
    let plan =
        "unauthorized: [MEMORY_PLAN_REQUIRED] memory API memory/scopes on api.tinyhumans.ai: \
                the session expired or the API key was rejected (HTTP 403 Forbidden); \
                re-authenticate — memory is not included in this account's plan";
    assert!(is_plan_required_message(plan));
    assert!(!is_session_expired_message(plan));
    assert_eq!(
        expected_error_kind(plan),
        Some(ExpectedErrorKind::BackendUserError)
    );
    assert!(!is_plan_required_message(
        "[USER_INSUFFICIENT_CREDITS] top up"
    ));
    // Only the delimited code in a hosted failure head counts, not a mention.
    assert!(!is_plan_required_message(
        "backend returned 500: unexpected [MEMORY_PLAN_REQUIRED] token"
    ));
    assert!(!is_plan_required_message(
        "engine error: saw [not a code_PLAN_REQUIRED] memory API x"
    ));
}
