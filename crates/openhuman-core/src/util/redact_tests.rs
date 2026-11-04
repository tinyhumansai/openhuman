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
