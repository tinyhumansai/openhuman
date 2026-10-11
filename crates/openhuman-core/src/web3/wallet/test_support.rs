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

use tempfile::TempDir;

use crate::config::rpc as config_rpc;
use crate::config::test_env::EnvVarGuard;
use {
    super::ops::setup, super::ops::WalletSetupParams,
    tinywallet_web3::crypto::wallet::WalletAccount, tinywallet_web3::crypto::wallet::WalletChain,
    tinywallet_web3::crypto::wallet::WalletSetupSource,
};

pub(crate) static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

/// Every environment variable that overrides a wallet endpoint.
const WALLET_RPC_ENV_VARS: [&str; 9] = [
    "OPENHUMAN_WALLET_RPC_EVM",
    "OPENHUMAN_WALLET_RPC_BASE",
    "OPENHUMAN_WALLET_RPC_ARBITRUM",
    "OPENHUMAN_WALLET_RPC_OPTIMISM",
    "OPENHUMAN_WALLET_RPC_POLYGON",
    "OPENHUMAN_WALLET_RPC_BSC",
    "OPENHUMAN_WALLET_RPC_BTC",
    "OPENHUMAN_WALLET_RPC_SOLANA",
    "OPENHUMAN_WALLET_RPC_TRON",
];

/// Serialises tests that read or write the wallet endpoint variables, which are
/// process-global.
pub(crate) static RPC_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Points every wallet endpoint at a loopback port nothing listens on, so a
/// test that reaches a chain fails fast and never leaves the machine. The
/// previous values are restored on drop.
pub(crate) struct UnreachableRpcGuard {
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
    _env_lock: std::sync::MutexGuard<'static, ()>,
}

impl UnreachableRpcGuard {
    pub(crate) fn set() -> Self {
        let env_lock = RPC_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Bind then drop to learn a port that is free, hence refusing.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .map_or(1, |addr| addr.port());
        let previous = WALLET_RPC_ENV_VARS
            .iter()
            .map(|name| (*name, std::env::var_os(name)))
            .collect();
        for name in WALLET_RPC_ENV_VARS {
            std::env::set_var(name, format!("http://127.0.0.1:{port}"));
        }
        Self {
            previous,
            _env_lock: env_lock,
        }
    }
}

impl Drop for UnreachableRpcGuard {
    fn drop(&mut self) {
        for (name, value) in self.previous.drain(..) {
            match value {
                Some(v) => std::env::set_var(name, v),
                None => std::env::remove_var(name),
            }
        }
    }
}

pub(crate) async fn set_workspace_env_for_test(temp: &TempDir) -> EnvVarGuard {
    EnvVarGuard::workspace_async(temp.path()).await
}

pub(crate) async fn setup_wallet_in(temp: &TempDir) -> Result<EnvVarGuard, String> {
    // Wallet state lookups rely on OPENHUMAN_WORKSPACE for the duration of
    // each test. Return a guard so the tempdir path does not leak into later
    // parallel tests after this test's TempDir has been dropped.
    let workspace_guard = set_workspace_env_for_test(temp).await;
    let config = config_rpc::load_config_with_timeout().await?;
    let encrypted = crate::security::encryption::rpc::encrypt_secret(&config, TEST_MNEMONIC)
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
