use super::redact_url_for_log;

#[cfg(feature = "security-module")]
#[test]
fn endpoint_redaction_never_returns_query_or_fragment_credentials() {
    let input = "https://user:password@example.test/path?token=query-secret#fragment-secret";
    let output = redact_url_for_log(input);
    assert!(!output.contains("password"));
    assert!(!output.contains("query-secret"));
    assert!(!output.contains("fragment-secret"));
}

const CREDENTIALED_URL: &str =
    "https://user:password@example.test/path?token=query-secret#fragment-secret";

#[cfg(not(feature = "security-module"))]
#[test]
fn a_build_without_the_module_logs_the_endpoint_unredacted() {
    assert_eq!(redact_url_for_log(CREDENTIALED_URL), CREDENTIALED_URL);
}

#[cfg(feature = "security-module")]
#[test]
fn security_configured_off_logs_the_endpoint_unredacted() {
    let _off = crate::security::switch::off_for_this_thread();
    assert_eq!(
        redact_url_for_log(&format!(" {CREDENTIALED_URL} ")),
        CREDENTIALED_URL
    );
}

/// With security disabled a token in the query string is logged as given.
fn assert_token_query_is_logged_unchanged() {
    let url = "https://api.example.test/v1/items?token=sk-live-123&page=2";
    let logged = redact_url_for_log(url);
    assert_eq!(logged, url);
    assert!(logged.contains("token=sk-live-123"));
}

#[cfg(not(feature = "security-module"))]
#[test]
fn a_build_without_the_module_logs_a_token_query_param_unchanged() {
    assert_token_query_is_logged_unchanged();
}

#[cfg(feature = "security-module")]
#[test]
fn security_configured_off_logs_a_token_query_param_unchanged() {
    let _off = crate::security::switch::off_for_this_thread();
    assert_token_query_is_logged_unchanged();
}
