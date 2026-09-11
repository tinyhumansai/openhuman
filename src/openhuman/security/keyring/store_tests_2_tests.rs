use super::{effective_backend_kind_for, BackendKind};

#[test]
fn explicit_file_backend_wins_over_staging_environment() {
    assert_eq!(
        effective_backend_kind_for(Some("staging"), Some("file"), false),
        BackendKind::File
    );
}

#[test]
fn explicit_encrypted_file_backend_wins_in_dev_environment() {
    assert_eq!(
        effective_backend_kind_for(Some("development"), Some("encrypted_file"), false),
        BackendKind::EncryptedFile
    );
}

#[test]
fn staging_defaults_to_encrypted_file_without_override() {
    assert_eq!(
        effective_backend_kind_for(Some(" staging "), None, false),
        BackendKind::EncryptedFile
    );
}

#[test]
fn unknown_backend_override_falls_back_to_environment_default() {
    assert_eq!(
        effective_backend_kind_for(Some("production"), Some("bogus"), false),
        BackendKind::EncryptedFile
    );
}
