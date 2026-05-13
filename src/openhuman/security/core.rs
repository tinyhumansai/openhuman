/// Redact sensitive values for safe logging. Shows first 4 chars + "***" suffix.
/// This function intentionally breaks the data-flow taint chain for static analysis.
pub fn redact(value: &str) -> String {
    if value.chars().count() <= 4 {
        "***".to_string()
    } else {
        crate::openhuman::util::truncate_with_suffix(value, 4, "***")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::security::{AutonomyLevel, PairingGuard, SecretStore, SecurityPolicy};

    #[test]
    fn reexported_policy_and_pairing_types_are_usable() {
        let policy = SecurityPolicy::default();
        assert_eq!(policy.autonomy, AutonomyLevel::Supervised);

        let (guard, _) = PairingGuard::new(false, &[]);
        assert!(!guard.require_pairing());
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
}
