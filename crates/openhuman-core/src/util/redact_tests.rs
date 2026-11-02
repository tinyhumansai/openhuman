use super::redact_url_for_log;

#[test]
fn endpoint_redaction_never_returns_query_or_fragment_credentials() {
    let input = "https://user:password@example.test/path?token=query-secret#fragment-secret";
    let output = redact_url_for_log(input);
    assert!(!output.contains("password"));
    assert!(!output.contains("query-secret"));
    assert!(!output.contains("fragment-secret"));
}
