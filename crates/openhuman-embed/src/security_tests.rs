use super::*;

#[test]
fn bearer_endpoints_require_https_or_loopback_http() {
    for endpoint in [
        "https://api.example.com/v1",
        "http://localhost:8080/v1",
        "http://127.0.0.1:8080/v1",
        "http://[::1]:8080/v1",
    ] {
        assert!(is_safe_endpoint_for_bearer(endpoint), "{endpoint}");
    }

    for endpoint in [
        "http://api.example.com/v1",
        "ftp://localhost/v1",
        "not a URL",
        "http://192.168.1.10/v1",
    ] {
        assert!(!is_safe_endpoint_for_bearer(endpoint), "{endpoint}");
    }
}
