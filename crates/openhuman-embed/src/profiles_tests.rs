use super::*;

#[test]
fn a_missing_service_token_is_written_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("service.token");
    ensure_service_token(&path).unwrap();
    let token = std::fs::read_to_string(&path).unwrap();
    assert_eq!(token.len(), 64, "two uuids' worth of hex");
    assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    // The boot guard accepts what was written.
    assert!(matches!(
        openhuman_core::core::runtime::boot_guard::ServiceToken::read(&path),
        openhuman_core::core::runtime::boot_guard::ServiceToken::Valid(_)
    ));
}

#[test]
fn an_existing_service_token_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("service.token");
    std::fs::write(&path, "operator-chosen").unwrap();
    ensure_service_token(&path).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "operator-chosen");
}

#[test]
fn a_token_that_cannot_be_written_is_a_boot_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing-dir").join("service.token");
    let error = ensure_service_token(&path).unwrap_err();
    assert!(matches!(error, ProfileError::Boot(_)), "{error}");
}

#[test]
fn the_builder_edits_the_saas_settings() {
    let builder = ProfileRuntime::builder(SaasConfig::new("/srv/openhuman")).configure(|c| {
        c.max_profiles_open = 2;
        c.profile_ids = ProfileIdMode::Hashed;
    });
    assert_eq!(builder.config.max_profiles_open, 2);
    assert_eq!(builder.config.profile_ids, ProfileIdMode::Hashed);
    assert!(builder.session_store.is_none());
}

#[test]
fn open_errors_keep_their_meaning() {
    let id = ProfileId::parse("alice").unwrap();
    let error = ProfileError::from(OpenError::NotProvisioned(id));
    assert!(error.to_string().contains("not provisioned"), "{error}");
    let error = ProfileError::Turn {
        message: "no backend".into(),
        error_type: Some("backend_unavailable".into()),
    };
    assert_eq!(error.to_string(), "turn failed: no backend");
}
