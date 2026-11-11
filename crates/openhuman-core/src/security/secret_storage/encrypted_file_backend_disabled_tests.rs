use super::*;

#[test]
fn selecting_encrypted_file_fails_closed_when_the_feature_is_disabled() {
    let backend = EncryptedFileBackend::new(Path::new("unused"));
    assert_eq!(backend.name(), "encrypted_file");
    assert!(backend
        .get("token")
        .unwrap_err()
        .to_string()
        .contains("enable the secret-file feature"));
    assert!(backend
        .set("token", "value")
        .unwrap_err()
        .to_string()
        .contains("enable the secret-file feature"));
}

#[test]
fn profile_read_only_lookup_fails_closed_when_the_feature_is_disabled() {
    let backend = EncryptedFileBackend::new(Path::new("unused"));
    assert!(backend
        .get_profile_read_only("user:token")
        .unwrap_err()
        .to_string()
        .contains("enable the secret-file feature"));
}
