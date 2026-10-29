use super::*;

#[test]
fn unauthenticated_cloud_routes_keep_the_optional_http_client_behavior() {
    assert!(build_cloud_http_client(AuthStyle::None, false, None)
        .expect("no-auth client")
        .is_none());
    assert!(build_cloud_http_client(
        AuthStyle::None,
        false,
        Some(include_str!("../../../util/tls/test-ca.pem")),
    )
    .expect("custom CA client")
    .is_some());
}

#[test]
fn authenticated_cloud_routes_always_install_a_redirect_safe_client() {
    for style in [AuthStyle::Bearer, AuthStyle::Anthropic] {
        assert!(build_cloud_http_client(style, false, None)
            .expect("authenticated client")
            .is_some());
    }
}
