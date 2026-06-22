//! Shared test plumbing for wallet unit tests across all chains.
//!
//! Provides:
//! - [`TEST_LOCK`]: serializes wallet tests that mutate the global quote store
//!   and per-chain env-var overrides. Tests that mutate
//!   `OPENHUMAN_WORKSPACE` also hold the config `TEST_ENV_LOCK`.
//! - [`setup_wallet_in`]: writes a configured wallet state into a
//!   [`tempfile::TempDir`] using the standard "abandon × 11 about" mnemonic
//!   so every chain's signer derives a deterministic address.
//! - Sample addresses corresponding to that mnemonic (one per chain).

use std::path::Path;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use tempfile::TempDir;

use super::ops::{setup, WalletAccount, WalletChain, WalletSetupParams, WalletSetupSource};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::TEST_ENV_LOCK;

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
    _env_lock: std::sync::MutexGuard<'static, ()>,
}

impl WorkspaceEnvGuard {
    pub(crate) fn set(path: impl AsRef<Path>) -> Self {
        // OPENHUMAN_WORKSPACE is process-global, so hold the shared config env
        // lock for the full lifetime of the test workspace override.
        let env_lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("OPENHUMAN_WORKSPACE");
        std::env::set_var("OPENHUMAN_WORKSPACE", path.as_ref());
        Self {
            prev,
            _env_lock: env_lock,
        }
    }
}

impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        match self.prev.take() {
            Some(v) => std::env::set_var("OPENHUMAN_WORKSPACE", v),
            None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
        }
    }
}

pub(crate) fn set_workspace_env_for_test(temp: &TempDir) -> WorkspaceEnvGuard {
    WorkspaceEnvGuard::set(temp.path())
}

pub(crate) async fn setup_wallet_in(temp: &TempDir) -> Result<WorkspaceEnvGuard, String> {
    // Wallet state lookups rely on OPENHUMAN_WORKSPACE for the duration of
    // each test. Return a guard so the tempdir path does not leak into later
    // parallel tests after this test's TempDir has been dropped.
    let workspace_guard = set_workspace_env_for_test(temp);
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
        // Test helper: force=true allows re-setup in tests that already have a wallet.
        force: true,
    })
    .await?;
    Ok(workspace_guard)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn workspace_env_guard_restores_workspace_env_when_dropped() {
        let env_lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
        std::env::set_var("OPENHUMAN_WORKSPACE", "/tmp/openhuman-existing-workspace");

        let temp = TempDir::new().expect("temp dir");
        let prev = std::env::var_os("OPENHUMAN_WORKSPACE");
        std::env::set_var("OPENHUMAN_WORKSPACE", temp.path());
        let workspace_guard = WorkspaceEnvGuard {
            prev,
            _env_lock: env_lock,
        };
        assert_eq!(
            std::env::var_os("OPENHUMAN_WORKSPACE"),
            Some(temp.path().as_os_str().to_os_string())
        );

        drop(workspace_guard);
        let _cleanup_lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            std::env::var_os("OPENHUMAN_WORKSPACE"),
            Some(std::ffi::OsString::from(
                "/tmp/openhuman-existing-workspace"
            ))
        );

        match previous {
            Some(value) => std::env::set_var("OPENHUMAN_WORKSPACE", value),
            None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
        }
    }
}
