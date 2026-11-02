use super::*;

fn identity() -> tinysecurity_bus::SecretBackendIdentity2 {
    tinysecurity_bus::SecretBackendIdentity2 {
        backend_id: "target-backend".into(),
        registration_id: "registration-id".into(),
        generation: 1,
        operation_id: "operation-id".into(),
        phase: SecretPhase2::TargetSet,
        slot: MasterSlotKind2::User,
    }
}

fn backend() -> SecretBackend {
    SecretBackend::new(
        "registration-id".into(),
        "user-id".into(),
        "target-backend".into(),
        "legacy-backend".into(),
        PathBuf::from("/unused/legacy-key"),
        crate::security::secret_storage::store::effective_backend_kind(),
    )
}

#[test]
fn target_mutations_reject_a_different_registration_backend_or_slot() {
    let backend = backend();
    let mut wrong_registration = identity();
    wrong_registration.registration_id.push('x');
    let mut wrong_backend = identity();
    wrong_backend.backend_id.push('x');
    let mut wrong_slot = identity();
    wrong_slot.slot = MasterSlotKind2::File;
    for identity in [wrong_registration, wrong_backend, wrong_slot] {
        let response = backend.set_request(SecretBackendSetRequest2 {
            identity,
            encoded: b"deadbeef".to_vec(),
        });
        assert_eq!(response.status, SecretBackendSetStatus2::Denied);
    }
}

#[test]
fn legacy_source_is_never_a_mutation_target() {
    let mut identity = identity();
    identity.backend_id = "legacy-backend".into();
    identity.slot = MasterSlotKind2::File;
    let response = backend().set_request(SecretBackendSetRequest2 {
        identity,
        encoded: b"deadbeef".to_vec(),
    });
    assert_eq!(response.status, SecretBackendSetStatus2::Denied);
}

#[test]
fn explicit_existing_master_key_is_read_once_and_set_is_denied() {
    let backend = backend();
    let encoded = "ab".repeat(32);
    let _read = backend
        .arm_existing_master_read(&encoded)
        .expect("valid supplied key can arm one read");

    let mut load_identity = identity();
    load_identity.phase = SecretPhase2::TargetRead;
    let response = backend.load_request(SecretBackendLoadRequest2 {
        identity: load_identity,
    });
    assert_eq!(
        response.status,
        SecretBackendLoadStatus2::Present {
            encoded: encoded.as_bytes().to_vec(),
        }
    );
    let mut repeated_read = identity();
    repeated_read.phase = SecretPhase2::TargetRead;
    let response = backend.load_request(SecretBackendLoadRequest2 {
        identity: repeated_read,
    });
    assert_eq!(response.status, SecretBackendLoadStatus2::Denied);

    let response = backend.set_request(SecretBackendSetRequest2 {
        identity: identity(),
        encoded: b"must-not-be-written".to_vec(),
    });
    assert_eq!(response.status, SecretBackendSetStatus2::Denied);
}

#[test]
fn invalid_explicit_master_key_cannot_arm_a_read_or_write() {
    let backend = backend();
    assert!(backend.arm_existing_master_read("not-a-key").is_err());
    assert!(backend.existing_master_read.lock().is_none());
    assert!(backend.pending_targets.lock().is_empty());
}

#[test]
fn malformed_hex_key_cannot_arm_a_read_or_write() {
    let backend = backend();
    assert!(backend.arm_existing_master_read(&"zz".repeat(32)).is_err());
    assert!(backend.existing_master_read.lock().is_none());
    assert!(backend.pending_targets.lock().is_empty());
}
