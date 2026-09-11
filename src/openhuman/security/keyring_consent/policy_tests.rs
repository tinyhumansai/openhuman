use super::*;
use std::sync::{Mutex, OnceLock};

fn cache_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("keyring consent cache test lock")
}

#[test]
fn classify_failure_linux() {
    if cfg!(target_os = "linux") {
        let reason = classify_failure_reason("os");
        assert_eq!(reason, KeyringFailureReason::NoSecretService);
    }
}

#[test]
fn classify_failure_macos() {
    if cfg!(target_os = "macos") {
        let reason = classify_failure_reason("os");
        assert_eq!(reason, KeyringFailureReason::AccessDenied);
    }
}

#[test]
fn classify_failure_encrypted_file() {
    let reason = classify_failure_reason("encrypted_file");
    assert_eq!(reason, KeyringFailureReason::MasterKeyUnavailable);
}

#[test]
fn classify_failure_unknown() {
    let reason = classify_failure_reason("weird_backend");
    assert!(matches!(reason, KeyringFailureReason::Unknown(_)));
}

#[test]
fn record_consent_updates_cache() {
    let _lock = cache_test_lock();
    let pref = record_consent("local_encrypted");
    assert_eq!(pref.storage_mode, "local_encrypted");
    assert!(pref.consented_at_ms.is_some());

    let cached = CONSENT_CACHE.read().clone();
    assert!(cached.is_some());
    assert_eq!(cached.unwrap().storage_mode, "local_encrypted");
}

#[test]
fn initialize_populates_cache() {
    let _lock = cache_test_lock();
    *CONSENT_CACHE.write() = None;
    let pref = ConsentPreference {
        storage_mode: "declined".to_string(),
        consented_at_ms: Some(12345),
    };
    initialize(Some(pref.clone()));
    let cached = CONSENT_CACHE.read().clone();
    assert_eq!(cached.unwrap().storage_mode, "declined");
}

#[test]
fn initialize_is_change_gated() {
    let _lock = cache_test_lock();
    *CONSENT_CACHE.write() = None;

    // First real value populates the cache and reports it applied (the INFO
    // log + write happened).
    let pref = ConsentPreference {
        storage_mode: "local_encrypted".to_string(),
        consented_at_ms: Some(111),
    };
    assert!(initialize(Some(pref.clone())), "first value should apply");
    assert_eq!(CONSENT_CACHE.read().clone(), Some(pref.clone()));

    // Repeat with the identical value — the no-op path: returns false (no
    // write, no INFO log), which is what every app_state_snapshot hits.
    // Asserting the return value proves the side effect is suppressed, not
    // merely that the resulting cache value is unchanged.
    assert!(
        !initialize(Some(pref.clone())),
        "identical value must be a no-op (no re-log / re-write)"
    );
    assert_eq!(CONSENT_CACHE.read().clone(), Some(pref));

    // A genuine change is still applied (returns true).
    let changed = ConsentPreference {
        storage_mode: "declined".to_string(),
        consented_at_ms: Some(222),
    };
    assert!(
        initialize(Some(changed.clone())),
        "a genuine change should apply"
    );
    assert_eq!(CONSENT_CACHE.read().clone(), Some(changed));
}

fn consent(mode: &str) -> ConsentPreference {
    ConsentPreference {
        storage_mode: mode.to_string(),
        consented_at_ms: Some(1),
    }
}

/// #6076: the `os` backend is the only one that may report `os_keyring`, and
/// only while its probe passes.
#[test]
fn active_mode_os_backend_reports_os_keyring_only_when_available() {
    assert_eq!(
        active_mode_for(true, BACKEND_OS, None),
        StorageMode::OsKeyring
    );
    // Probe failed, nothing recorded yet → the consent prompt is pending.
    assert_eq!(
        active_mode_for(false, BACKEND_OS, None),
        StorageMode::ConsentPending
    );
    // A recorded decision is honoured, in both directions.
    assert_eq!(
        active_mode_for(false, BACKEND_OS, Some(&consent("local_encrypted"))),
        StorageMode::LocalEncrypted
    );
    assert_eq!(
        active_mode_for(false, BACKEND_OS, Some(&consent("declined"))),
        StorageMode::Declined
    );
    // An unparseable persisted value must not be read as consent.
    assert_eq!(
        active_mode_for(false, BACKEND_OS, Some(&consent("something_else"))),
        StorageMode::ConsentPending
    );
}

/// The regression itself: `probe_availability` short-circuits to `true` for
/// every non-OS backend, so `available` alone reported `os_keyring` for all of
/// them. The mode must now follow the backend identity.
#[test]
fn active_mode_non_os_backends_never_report_os_keyring() {
    for backend in [
        BACKEND_ENCRYPTED_FILE,
        BACKEND_FILE,
        BACKEND_MOCK,
        "brand_new",
    ] {
        for available in [true, false] {
            for cached in [
                None,
                Some(consent("local_encrypted")),
                Some(consent("declined")),
            ] {
                let mode = active_mode_for(available, backend, cached.as_ref());
                assert_ne!(
                    mode,
                    StorageMode::OsKeyring,
                    "backend={backend} available={available} must not claim os_keyring"
                );
            }
        }
    }
}

/// `encrypted_file` is the staging/production backend: secrets are in
/// `{workspace}/secrets.enc`, so it gets its own mode rather than borrowing the
/// consent outcome's `local_encrypted`.
#[test]
fn active_mode_encrypted_file_is_its_own_mode_and_ignores_consent() {
    for cached in [
        None,
        Some(consent("local_encrypted")),
        Some(consent("declined")),
    ] {
        assert_eq!(
            active_mode_for(true, BACKEND_ENCRYPTED_FILE, cached.as_ref()),
            StorageMode::LocalEncryptedFile
        );
    }
}

/// `file` is plaintext `dev-keychain.json` with no OS keychain involvement at
/// all — it must not be labelled "encrypted" in either sense.
#[test]
fn active_mode_plaintext_backends_report_plaintext() {
    for backend in [BACKEND_FILE, BACKEND_MOCK] {
        let mode = active_mode_for(true, backend, Some(&consent("local_encrypted")));
        assert_eq!(mode, StorageMode::LocalPlaintextFile, "backend={backend}");
    }
}

/// A backend added upstream without extending the table must claim nothing
/// rather than defaulting back into the bug.
#[test]
fn active_mode_unknown_backend_claims_nothing() {
    assert_eq!(
        active_mode_for(true, "brand_new", None),
        StorageMode::ConsentPending
    );
}

/// The self-contradiction from the report: `activeMode: os_keyring` shipping
/// beside `backendName: file` in one object. Asserted over the real
/// `KeyringStatus` payload, not just the mode, since that pairing is what a
/// consumer sees.
#[test]
fn status_payload_mode_and_backend_never_contradict() {
    for (backend, available) in [
        (BACKEND_OS, true),
        (BACKEND_OS, false),
        (BACKEND_ENCRYPTED_FILE, true),
        (BACKEND_FILE, true),
        (BACKEND_MOCK, true),
    ] {
        let status = KeyringStatus {
            available,
            failure_reason: None,
            active_mode: active_mode_for(available, backend, None),
            backend_name: backend.to_string(),
        };
        if status.active_mode == StorageMode::OsKeyring {
            assert_eq!(
                status.backend_name, BACKEND_OS,
                "only the os backend may report os_keyring"
            );
        }
    }
}
