use super::*;

#[test]
fn generic_http_401_is_not_classified_as_invalid_api_key() {
    assert!(!is_invalid_api_key_error("HTTP 401"));
    assert!(is_invalid_api_key_error("HTTP 401: Invalid API key"));
}

#[test]
fn non_auth_failure_resets_invalid_key_streak() {
    let key_id = fingerprint_api_key("ck_test_streak_reset");
    reset_direct_auth_failure(key_id);

    assert_eq!(
        record_direct_auth_failure(key_id, "HTTP 401: Invalid API key"),
        DirectAuthFailureDecision::RetryAllowed { consecutive: 1 }
    );
    assert_eq!(
        record_direct_auth_failure(key_id, "HTTP 401: Invalid API key"),
        DirectAuthFailureDecision::RetryAllowed { consecutive: 2 }
    );
    assert_eq!(
        record_direct_auth_failure(key_id, "HTTP 500: upstream unavailable"),
        DirectAuthFailureDecision::NotAuthFailure
    );
    assert!(
        direct_auth_backoff_error(key_id).is_none(),
        "non-auth failures must clear stale invalid-key counts"
    );
    assert_eq!(
        record_direct_auth_failure(key_id, "HTTP 401: Invalid API key"),
        DirectAuthFailureDecision::RetryAllowed { consecutive: 1 }
    );

    reset_direct_auth_failure(key_id);
}
