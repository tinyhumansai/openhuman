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
///
/// Every chain has one primary account at `index` 0, which is the only
/// account the TinyWallet module's transfers, swaps and x402 payments use.
/// A chain may also have secondary accounts from the same recovery phrase
/// (`index` 1 and up, see [`super::SECONDARY_ACCOUNTS`]); they sign only
/// through `wallet.execute_prepared_program`, under their own
/// `[wallet_policy.accounts.<chain>_<index>]` section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WalletAccount {
    pub chain: WalletChain,
    pub address: String,
    pub derivation_path: String,
    /// Account index on the chain. Absent in state written before secondary
    /// accounts existed, which reads as the primary account.
    #[serde(default, skip_serializing_if = "is_primary_index")]
    pub index: u32,
    /// Optional human label for a secondary account (`giveaway`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_primary_index(index: &u32) -> bool {
    *index == 0
}

impl WalletAccount {
    /// Whether this is the chain's primary (index 0) account.
    #[must_use]
    pub fn is_primary(&self) -> bool {
        self.index == 0
    }
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
    /// The primary (index 0) account of each chain.
    pub accounts: Vec<WalletAccount>,
    /// Secondary accounts (index 1 and up). Kept out of `accounts` so callers
    /// that expect one account per chain are unaffected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_accounts: Vec<WalletAccount>,
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
    /// Public addresses and derivation paths for the four supported chains'
    /// primary accounts, followed by the secondary accounts
    /// ([`super::SECONDARY_ACCOUNTS`]).
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

/// Non-secret projection for read-only inspection of a stored wallet snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StoredWalletInspection {
    pub consent_granted: bool,
    pub account_count: usize,
}

/// Deserialize using the canonical persisted wallet schema and expose only
/// the fields needed by health checks.
pub(crate) fn inspect_stored_wallet_state(
    bytes: &[u8],
) -> Result<StoredWalletInspection, serde_json::Error> {
    let state: StoredWalletState = serde_json::from_slice(bytes)?;
    Ok(StoredWalletInspection {
        consent_granted: state.consent_granted,
        account_count: state.accounts.len(),
    })
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
