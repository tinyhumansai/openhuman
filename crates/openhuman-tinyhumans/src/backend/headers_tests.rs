use super::*;
use crate::backend::product::{
    product_identity_test_lock, reset_product_identity_for_test, set_product_identity,
    ProductIdentity, DEFAULT_PRODUCT_IDENTITY, PRODUCT_IDENTITY_HEADER,
};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn sanitize_client_version_strips_invalid_chars_and_clamps_length() {
    let raw = format!(" 1.2.3 (desktop)+build!?{} ", "a".repeat(80));
    let sanitized = sanitize_client_version(&raw).unwrap();
    assert_eq!(sanitized, format!("1.2.3desktop+build{}", "a".repeat(46)));
    assert_eq!(sanitized.len(), CLIENT_VERSION_HEADER_MAX_LEN);
    assert_eq!(sanitize_client_version(" ?! "), None);
}

#[test]
fn attribution_headers_carry_the_core_version_and_default_identity() {
    let _guard = product_identity_test_lock();
    reset_product_identity_for_test();
    let headers = attribution_headers().unwrap();
    assert_eq!(
        headers.get("x-core-version").and_then(|v| v.to_str().ok()),
        sanitize_client_version(env!("CARGO_PKG_VERSION")).as_deref()
    );
    assert_eq!(
        headers
            .get(PRODUCT_IDENTITY_HEADER)
            .and_then(|v| v.to_str().ok()),
        Some(DEFAULT_PRODUCT_IDENTITY)
    );
}

#[tokio::test]
async fn the_profile_client_stamps_versions_and_identity_on_raw_requests() {
    // `http_client` traffic (multipart STT upload, the Langfuse push) bypasses
    // the SDK, so the attribution has to ride the client's default headers.
    // The identity is baked into the client's default headers when it is
    // built, so hold the identity lock only for the build (never across an
    // await).
    let client = {
        let _guard = product_identity_test_lock();
        reset_product_identity_for_test();
        build_backend_client(TransportProfile::Api).unwrap()
    };
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/probe"))
        .and(header(PRODUCT_IDENTITY_HEADER, DEFAULT_PRODUCT_IDENTITY))
        .and(header(
            "x-core-version",
            sanitize_client_version(env!("CARGO_PKG_VERSION"))
                .unwrap()
                .as_str(),
        ))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let response = client
        .get(format!("{}/probe", server.uri()))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
}

#[tokio::test]
async fn the_shell_version_rides_along_when_exported() {
    // The crate-wide env lock, so tests that fingerprint the attribution
    // headers cannot observe this test's temporary variable.
    let _env = crate::backend::url::backend_env_test_lock();
    std::env::set_var(TAURI_VERSION_ENV_VAR, "9.8.7-shell+test");
    let headers = attribution_headers();
    std::env::remove_var(TAURI_VERSION_ENV_VAR);
    let headers = headers.unwrap();
    assert_eq!(
        headers.get("x-tauri-version").and_then(|v| v.to_str().ok()),
        Some("9.8.7-shell+test")
    );
    assert!(headers.get("x-core-version").is_some());
}

#[test]
fn an_embedding_product_can_override_the_identity() {
    let _guard = product_identity_test_lock();
    set_product_identity(ProductIdentity::new("opencompany").unwrap());
    let headers = attribution_headers();
    reset_product_identity_for_test();
    assert_eq!(
        headers
            .unwrap()
            .get(PRODUCT_IDENTITY_HEADER)
            .and_then(|v| v.to_str().ok()),
        Some("opencompany")
    );
}
