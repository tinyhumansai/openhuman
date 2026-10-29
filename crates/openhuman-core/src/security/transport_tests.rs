use super::*;

#[test]
fn bearer_transport_allows_tls_and_only_http_loopback() {
    assert!(is_safe_endpoint_for_bearer("https://api.example.test/v1"));
    assert!(is_safe_endpoint_for_bearer("http://localhost:8080/v1"));
    assert!(is_safe_endpoint_for_bearer("http://[::1]:8080/v1"));
    assert!(is_safe_endpoint_for_bearer("http://127.0.0.1:8080/v1"));
    assert!(is_safe_endpoint_for_bearer("http://127.200.10.20:8080/v1"));

    assert!(!is_safe_endpoint_for_bearer("http://192.168.1.10:8080/v1"));
    assert!(!is_safe_endpoint_for_bearer(
        "http://127.attacker.example/v1"
    ));
    assert!(!is_safe_endpoint_for_bearer("ftp://localhost/v1"));
    assert!(!is_safe_endpoint_for_bearer("not a url"));
}

#[test]
fn provider_endpoint_requires_http_shape_without_embedded_credentials_or_fragments() {
    for invalid in [
        "ftp://api.example.test/v1",
        "https://user:pass@api.example.test/v1",
        "https://api.example.test/v1?token=secret",
        "https://api.example.test/v1#fragment",
        "https://api.example.test/v1\n",
        "http://",
    ] {
        assert!(!is_valid_provider_endpoint(invalid), "{invalid}");
        assert!(!is_safe_endpoint_for_bearer(invalid), "{invalid}");
    }
    for valid in ["https://api.example.test/v1", "http://localhost:8080/v1", "http://192.168.1.10/v1"] {
        assert!(is_valid_provider_endpoint(valid), "{valid}");
    }
}

#[test]
fn bearer_header_names_are_case_insensitive_and_safe_headers_do_not_count() {
    assert!(has_credential_bearing_headers(&[("AuThOrIzAtIoN".into(), "Bearer secret".into())]));
    assert!(has_credential_bearing_headers(&[("X-API-Key".into(), "secret".into())]));
    assert!(has_credential_bearing_headers(&[("Cookie".into(), "session=secret".into())]));
    assert!(!has_credential_bearing_headers(&[("X-Trace-ID".into(), "run-1".into())]));
    assert!(!has_credential_bearing_headers(&[]));
}
