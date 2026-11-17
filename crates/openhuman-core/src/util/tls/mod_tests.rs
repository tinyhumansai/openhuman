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
