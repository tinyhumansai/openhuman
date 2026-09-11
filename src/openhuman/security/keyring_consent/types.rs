//! Types for the keyring consent domain.

use serde::{Deserialize, Serialize};

/// Where the process's secrets actually live.
///
/// The variants fall into three groups. They are named here rather than
/// referred to by position, because position is what rots:
///
/// - `OsKeyring` — no consent was ever needed; the `os` backend's probe
///   succeeded and secrets are in the OS credential store.
/// - `LocalEncrypted` / `ConsentPending` / `Declined` — **consent outcomes**,
///   reachable only on the `os` path: the OS keyring could not be used, and
///   the user has either answered the consent prompt or not answered yet.
/// - `LocalEncryptedFile` / `LocalPlaintextFile` — **operator-configured
///   backends**: nobody was asked anything, `OPENHUMAN_KEYRING_BACKEND` (or
///   the staging/production default) simply selected a different store.
///
/// The last two groups must stay apart because they are not the same storage:
/// a consented `LocalEncrypted` fallback means per-field `SecretStore`
/// encryption inside the config, while `LocalEncryptedFile` means the
/// `encrypted_file` backend's single `{workspace}/secrets.enc`.
///
/// Read [`KeyringStatus::backend_name`] alongside this — the two answer
/// different questions and must never contradict each other (#6076: every
/// non-OS backend reported `OsKeyring` while `backend_name` said otherwise).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageMode {
    /// The `os` backend, probed and working: macOS Keychain, Windows
    /// Credential Manager, or Linux Secret Service.
    OsKeyring,
    /// Consent outcome: the OS keyring failed and the user agreed to the
    /// local encrypted fallback.
    LocalEncrypted,
    /// Operator-configured `encrypted_file` backend — one ChaCha20-Poly1305
    /// `{workspace}/secrets.enc`, unlocked by a master key held in the OS
    /// keychain. The staging/production default.
    LocalEncryptedFile,
    /// Operator-configured `file` (or test-only `mock`) backend. `file` is a
    /// **plaintext** `{workspace}/dev-keychain.json` with no encryption and no
    /// OS keychain involvement at all — dev/test only.
    LocalPlaintextFile,
    /// The OS keyring is unavailable and the user has not answered yet.
    ConsentPending,
    /// Consent outcome: the user refused local storage.
    Declined,
}

impl std::fmt::Display for StorageMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OsKeyring => write!(f, "os_keyring"),
            Self::LocalEncrypted => write!(f, "local_encrypted"),
            Self::LocalEncryptedFile => write!(f, "local_encrypted_file"),
            Self::LocalPlaintextFile => write!(f, "local_plaintext_file"),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
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
#[path = "types_tests.rs"]
mod tests;
