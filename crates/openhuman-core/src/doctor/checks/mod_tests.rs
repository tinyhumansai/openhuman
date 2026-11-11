use super::*;
use crate::doctor::contract::DoctorTier;

#[test]
fn memory_storage_and_keyring_checks_are_registered_at_their_declared_tiers() {
    let checks = builtins();
    let tier = |id: &str| {
        checks
            .iter()
            .find(|check| check.id() == id)
            .map(|check| check.tier())
    };

    assert_eq!(tier("memory/binding"), Some(DoctorTier::Static));
    assert_eq!(tier("memory/health"), Some(DoctorTier::Connect));
    assert_eq!(tier("storage/connect"), Some(DoctorTier::Connect));
    assert_eq!(tier("storage/layout"), Some(DoctorTier::Static));
    assert_eq!(tier("secrets/keyring"), Some(DoctorTier::Connect));
}

#[test]
fn v1_check_replacements_are_registered_with_their_native_ids_and_tiers() {
    let checks = builtins();
    let tier = |id: &str| {
        checks
            .iter()
            .find(|check| check.id() == id)
            .map(|check| check.tier())
    };

    for (id, expected_tier) in [
        ("config/runtime-settings", DoctorTier::Static),
        ("workspace/writable", DoctorTier::Static),
        ("workspace/layout", DoctorTier::Static),
        ("runtime/environment", DoctorTier::Static),
        ("runtime/agent-sdk", DoctorTier::Connect),
        ("embeddings/configured-model", DoctorTier::Connect),
    ] {
        assert_eq!(tier(id), Some(expected_tier), "missing or mistiered {id}");
    }
}
