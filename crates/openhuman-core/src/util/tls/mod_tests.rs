use super::*;

#[test]
fn platform_tls_client_builds_with_native_roots_enabled() {
    // The native-roots setter is gated behind reqwest's feature. On macOS and
    // Linux, removing that feature makes tls_client_builder fail to compile.
    assert!(tls_client_builder().build().is_ok());
}

#[test]
fn private_ca_bundle_is_accepted_without_disabling_public_roots() {
    let pem = include_str!("test-ca.pem");
    assert_eq!(parse_ca_bundle(pem).unwrap().len(), 1);
    assert!(client_with_ca_bundle(pem, "provider.compatible").is_ok());
}

#[test]
fn invalid_ca_bundle_and_private_key_material_are_rejected() {
    assert!(parse_ca_bundle("not a PEM certificate").is_err());
    let private_key_marker = concat!("-----BEGIN ", "PRIVATE KEY-----");
    assert!(parse_ca_bundle(private_key_marker).is_err());
}

#[tokio::test]
async fn authenticated_http_client_never_forwards_x_api_key_to_a_redirect() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let destination = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/capture"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&destination)
        .await;

    let origin = MockServer::start().await;
    let redirected_to = format!("{}/capture", destination.uri());
    Mock::given(method("GET"))
        .and(path("/model"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", redirected_to.as_str()))
        .expect(1)
        .mount(&origin)
        .await;

    let http = credentialed_client_builder("provider.anthropic", None)
        .unwrap()
        .build()
        .unwrap();
    let result = http
        .get(format!("{}/model", origin.uri()))
        .header("x-api-key", "provider-secret")
        .send()
        .await
        .unwrap();
    assert_eq!(result.status(), reqwest::StatusCode::FOUND);
    assert!(destination.received_requests().await.unwrap().is_empty());
}

#[test]
fn authenticated_http_client_accepts_additional_certificate_roots() {
    let builder =
        credentialed_client_builder("provider.compatible", Some(include_str!("test-ca.pem")))
            .unwrap();
    assert!(builder.build().is_ok());
}
