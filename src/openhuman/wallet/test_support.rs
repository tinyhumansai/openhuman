//! Shared test plumbing for wallet unit tests across all chains.
//!
//! Provides:
//! - [`TEST_LOCK`]: serializes wallet tests that mutate the global quote store
//!   and per-chain env-var overrides.
//! - [`setup_wallet_in`]: writes a configured wallet state into a
//!   [`tempfile::TempDir`] using the standard "abandon × 11 about" mnemonic
//!   so every chain's signer derives a deterministic address.
//! - Sample addresses corresponding to that mnemonic (one per chain).

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use tempfile::TempDir;

use super::ops::{setup, WalletAccount, WalletChain, WalletSetupParams, WalletSetupSource};
use crate::openhuman::config::rpc as config_rpc;

pub(crate) static TEST_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Standard BIP-39 test mnemonic — produces deterministic accounts per chain.
pub(crate) const TEST_MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

pub(crate) fn sample_evm_address() -> &'static str {
    "0x9858EfFD232B4033E47d90003D41EC34EcaEda94"
}
pub(crate) fn sample_btc_address() -> &'static str {
    "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"
}
pub(crate) fn sample_solana_address() -> &'static str {
    "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk"
}
pub(crate) fn sample_tron_address() -> &'static str {
    "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH"
}

pub(crate) fn sample_account(chain: WalletChain) -> WalletAccount {
    WalletAccount {
        chain,
        address: match chain {
            WalletChain::Evm => sample_evm_address().to_string(),
            WalletChain::Btc => sample_btc_address().to_string(),
            WalletChain::Solana => sample_solana_address().to_string(),
            WalletChain::Tron => sample_tron_address().to_string(),
        },
        derivation_path: match chain {
            WalletChain::Evm => "m/44'/60'/0'/0/0".to_string(),
            WalletChain::Btc => "m/84'/0'/0'/0/0".to_string(),
            WalletChain::Solana => "m/44'/501'/0'/0'".to_string(),
            WalletChain::Tron => "m/44'/195'/0'/0/0".to_string(),
        },
    }
}

/// RAII guard returned to callers so `OPENHUMAN_WORKSPACE` is restored when
/// the test scope ends — prevents one test's tempdir from leaking into the
/// next test in the same process. Drop the guard explicitly or let it fall
/// out of scope at the end of the test.
pub(crate) struct WorkspaceEnvGuard {
    prev: Option<std::ffi::OsString>,
}

impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        match self.prev.take() {
            Some(v) => std::env::set_var("OPENHUMAN_WORKSPACE", v),
            None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
        }
    }
}

pub(crate) async fn setup_wallet_in(temp: &TempDir) -> Result<(), String> {
    // We intentionally leak the env-var change for the duration of the test
    // (wallet state lookups rely on it). The shared `TEST_LOCK` mutex
    // serializes tests so concurrent tests can't race on the var.
    std::env::set_var("OPENHUMAN_WORKSPACE", temp.path());
    let config = config_rpc::load_config_with_timeout().await?;
    let encrypted = crate::openhuman::encryption::rpc::encrypt_secret(&config, TEST_MNEMONIC)
        .await?
        .value;
    setup(WalletSetupParams {
        consent_granted: true,
        source: WalletSetupSource::Imported,
        mnemonic_word_count: 12,
        encrypted_mnemonic: Some(encrypted),
        accounts: [
            WalletChain::Evm,
            WalletChain::Btc,
            WalletChain::Solana,
            WalletChain::Tron,
        ]
        .into_iter()
        .map(sample_account)
        .collect(),
    })
    .await?;
    Ok(())
}
