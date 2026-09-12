use super::*;

#[test]
fn storage_mode_serialization_roundtrip() {
    let modes = [
        StorageMode::OsKeyring,
        StorageMode::LocalEncrypted,
        StorageMode::LocalEncryptedFile,
        StorageMode::LocalPlaintextFile,
        StorageMode::ConsentPending,
        StorageMode::Declined,
    ];
    for mode in modes {
        let json = serde_json::to_string(&mode).unwrap();
        let deserialized: StorageMode = serde_json::from_str(&json).unwrap();
        assert_eq!(mode, deserialized);
    }
}

/// `Display` and the serde representation must agree: `activeMode` reaches the
/// frontend as the serialized string, while logs and `RpcOutcome` messages use
/// `Display`, and `SecurityPanel` keys its badge variant + i18n lookup off the
/// serialized form. A variant whose two spellings diverge renders as an unstyled
/// unknown mode.
#[test]
fn storage_mode_display_matches_serde() {
    for (mode, wire) in [
        (StorageMode::OsKeyring, "os_keyring"),
        (StorageMode::LocalEncrypted, "local_encrypted"),
        (StorageMode::LocalEncryptedFile, "local_encrypted_file"),
        (StorageMode::LocalPlaintextFile, "local_plaintext_file"),
        (StorageMode::ConsentPending, "consent_pending"),
        (StorageMode::Declined, "declined"),
    ] {
        assert_eq!(mode.to_string(), wire, "Display for {mode:?}");
        assert_eq!(
            serde_json::to_value(mode).unwrap(),
            serde_json::Value::String(wire.to_string()),
            "serde for {mode:?}"
        );
    }
}

#[test]
fn failure_reason_display() {
    assert_eq!(
        KeyringFailureReason::NoSecretService.to_string(),
        "No Secret Service daemon available"
    );
    assert_eq!(
        KeyringFailureReason::Unknown("custom".to_string()).to_string(),
        "custom"
    );
}

#[test]
fn keyring_status_serialization() {
    let status = KeyringStatus {
        available: false,
        failure_reason: Some(KeyringFailureReason::NoSecretService),
        active_mode: StorageMode::ConsentPending,
        backend_name: "os".to_string(),
    };
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(json["available"], false);
    assert_eq!(json["activeMode"], "consent_pending");
    assert_eq!(json["failureReason"], "no_secret_service");
}

#[test]
fn keyring_status_omits_none_failure_reason() {
    let status = KeyringStatus {
        available: true,
        failure_reason: None,
        active_mode: StorageMode::OsKeyring,
        backend_name: "os".to_string(),
    };
    let json = serde_json::to_value(&status).unwrap();
    assert!(!json.as_object().unwrap().contains_key("failureReason"));
}

#[test]
fn consent_preference_defaults() {
    let pref = ConsentPreference::default();
    assert_eq!(pref.storage_mode, "");
    assert!(pref.consented_at_ms.is_none());
}
