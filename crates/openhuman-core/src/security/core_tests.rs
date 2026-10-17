use super::*;
use crate::security::{AutonomyLevel, SecretStore, SecurityPolicy};

#[test]
fn reexported_policy_is_usable() {
    let policy = SecurityPolicy::default();
    assert_eq!(policy.autonomy, AutonomyLevel::Supervised);
}

#[test]
fn reexported_secret_store_encrypt_decrypt_roundtrip() {
    let temp = tempfile::tempdir().unwrap();
    let store = SecretStore::new(temp.path(), false);

    let encrypted = store.encrypt("top-secret").unwrap();
    let decrypted = store.decrypt(&encrypted).unwrap();

    assert_eq!(decrypted, "top-secret");
}

#[test]
fn redact_hides_most_of_value() {
    assert_eq!(redact("abcdefgh"), "abcd***");
    assert_eq!(redact("ab"), "***");
    assert_eq!(redact(""), "***");
    assert_eq!(redact("12345"), "1234***");
}

#[test]
fn redact_utf8_boundary() {
    assert_eq!(redact("🦀🦀🦀🦀🦀"), "🦀🦀🦀🦀***");
}
