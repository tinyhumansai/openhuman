//! Types for the keyring consent domain.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageMode {
    OsKeyring,
    LocalEncrypted,
    ConsentPending,
    Declined,
}

impl std::fmt::Display for StorageMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OsKeyring => write!(f, "os_keyring"),
            Self::LocalEncrypted => write!(f, "local_encrypted"),
            Self::ConsentPending => write!(f, "consent_pending"),
            Self::Declined => write!(f, "declined"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyringFailureReason {
    NoSecretService,
    KeychainLocked,
    AccessDenied,
    MasterKeyUnavailable,
    Unknown(String),
}

impl std::fmt::Display for KeyringFailureReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSecretService => write!(f, "No Secret Service daemon available"),
            Self::KeychainLocked => write!(f, "OS keychain is locked"),
            Self::AccessDenied => write!(f, "Access to OS keychain was denied"),
            Self::MasterKeyUnavailable => write!(f, "Master encryption key unavailable"),
            Self::Unknown(msg) => write!(f, "{msg}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyringStatus {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<KeyringFailureReason>,
    pub active_mode: StorageMode,
    pub backend_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConsentPreference {
    #[serde(default)]
    pub storage_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consented_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDecision {
    Proceed,
    ConsentRequired,
    Declined,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_mode_serialization_roundtrip() {
        let modes = [
            StorageMode::OsKeyring,
            StorageMode::LocalEncrypted,
            StorageMode::ConsentPending,
            StorageMode::Declined,
        ];
        for mode in modes {
            let json = serde_json::to_string(&mode).unwrap();
            let deserialized: StorageMode = serde_json::from_str(&json).unwrap();
            assert_eq!(mode, deserialized);
        }
    }

    #[test]
    fn storage_mode_display() {
        assert_eq!(StorageMode::OsKeyring.to_string(), "os_keyring");
        assert_eq!(StorageMode::ConsentPending.to_string(), "consent_pending");
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
}
