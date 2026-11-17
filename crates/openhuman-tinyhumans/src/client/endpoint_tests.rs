use super::*;

#[test]
fn api_key_endpoint_is_bound_to_tinyhumans_or_loopback() {
    assert!(is_managed_endpoint_for_api_key(
        "https://api.tinyhumans.ai/openai/v1"
    ));
    assert!(is_managed_endpoint_for_api_key(
        "https://staging-api.tinyhumans.ai"
    ));
    assert!(is_managed_endpoint_for_api_key(
        "http://127.0.0.1:18765/openai/v1"
    ));
    assert!(is_managed_endpoint_for_api_key(
        "http://[::1]:18765/openai/v1"
    ));
    assert!(is_managed_endpoint_for_api_key("http://localhost:7788"));
    assert!(!is_managed_endpoint_for_api_key(
        "https://example.com/openai/v1"
    ));
    assert!(!is_managed_endpoint_for_api_key(
        "http://api.tinyhumans.ai/openai/v1"
    ));
    assert!(!is_managed_endpoint_for_api_key(
        "https://api.tinyhumans.ai.evil.example"
    ));
    assert!(!is_managed_endpoint_for_api_key("not a url"));
}

#[test]
fn bearer_endpoint_requires_https_or_loopback_http() {
    assert!(is_safe_endpoint_for_managed_bearer("https://example.com"));
    assert!(is_safe_endpoint_for_managed_bearer("http://127.0.0.1:1"));
    assert!(!is_safe_endpoint_for_managed_bearer("http://example.com"));
    assert!(!is_safe_endpoint_for_managed_bearer("ftp://127.0.0.1"));
    assert!(!is_safe_endpoint_for_managed_bearer(""));
}

#[test]
fn validate_credential_endpoint_reports_the_specific_refusal() {
    let key_err = validate_credential_endpoint("https://example.com", true).unwrap_err();
    assert!(key_err.to_string().contains("TinyHumans API key requires"));
    let bearer_err = validate_credential_endpoint("http://example.com", false).unwrap_err();
    assert!(bearer_err.to_string().contains("requires HTTPS"));
    validate_credential_endpoint("https://example.com", false).unwrap();
    validate_credential_endpoint("http://127.0.0.1:9", true).unwrap();
}
