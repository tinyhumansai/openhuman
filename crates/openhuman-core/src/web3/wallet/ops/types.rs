//! Wire and domain types for wallet setup, status, and secret material.

use serde::{Deserialize, Serialize};

/// Wallet chain identifiers are shared contract vocabulary.
pub use tinywallet_bus::wire::WalletChain;

/// Error returned when no local wallet has been configured.
pub const WALLET_NOT_CONFIGURED_MESSAGE: &str = "wallet is not configured; run wallet setup first";

/// How the host's local wallet state was created.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WalletSetupSource {
    Generated,
    Imported,
}

/// A public account retained in OpenHuman's wallet setup state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WalletAccount {
    pub chain: WalletChain,
    pub address: String,
    pub derivation_path: String,
}

/// Host-owned wallet setup status exposed by the existing RPC surface.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
pub struct WalletStatus {
    pub configured: bool,
    pub onboarding_completed: bool,
    pub consent_granted: bool,
    pub secret_stored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<WalletSetupSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mnemonic_word_count: Option<u8>,
    pub accounts: Vec<WalletAccount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WalletSetupParams {
    pub consent_granted: bool,
    pub source: WalletSetupSource,
    pub mnemonic_word_count: u8,
    #[serde(default)]
    pub encrypted_mnemonic: Option<String>,
    pub accounts: Vec<WalletAccount>,
    /// When `true`, allows overwriting an existing wallet.
    /// Requires explicit user confirmation in the frontend.
    /// Defaults to `false` — a guard against silent overwrites.
    #[serde(default)]
    pub force: bool,
}

/// Phrase generated or validated by TinyWallet and the derived accounts used
/// by the local wallet setup flow.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WalletRecoveryMaterial {
    /// Standard BIP-39 seed bytes. The frontend uses these only for the
    /// existing OpenHuman storage-key KDF and clears its temporary buffer.
    pub seed: Vec<u8>,
    /// Public addresses and derivation paths for the four supported chains.
    pub accounts: Vec<WalletAccount>,
}

impl std::fmt::Debug for WalletRecoveryMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WalletRecoveryMaterial")
            .field("seed", &"<redacted>")
            .field("accounts", &self.accounts)
            .finish()
    }
}

/// Generated recovery phrase returned for encryption and user confirmation.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedRecoveryPhrase {
    pub phrase: String,
    pub word_count: u8,
}

impl std::fmt::Debug for GeneratedRecoveryPhrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeneratedRecoveryPhrase")
            .field("phrase", &"<redacted>")
            .field("word_count", &self.word_count)
            .finish()
    }
}

/// Result of checking a recovery phrase with TinyWallet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryPhraseValidation {
    pub valid: bool,
}

/// Persisted on-disk (and/or keychain-backed) representation of wallet state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoredWalletState {
    pub consent_granted: bool,
    pub source: WalletSetupSource,
    pub mnemonic_word_count: u8,
    #[serde(default)]
    pub encrypted_mnemonic: Option<String>,
    pub accounts: Vec<WalletAccount>,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WalletSecretMaterial {
    pub encrypted_mnemonic: String,
    pub derivation_path: String,
}

/// Result returned by `reveal_recovery_phrase`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevealRecoveryPhraseResult {
    pub phrase: String,
    pub word_count: usize,
}
