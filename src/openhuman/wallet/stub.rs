//! Disabled-wallet facade.
//!
//! Compiled only when the `web3` Cargo feature is OFF (see the gate in
//! [`super`]). It mirrors the subset of the real `wallet` public surface that
//! always-on / other-gated callers depend on, with no-op / `None` /
//! disabled-error bodies so the crate still compiles, boots, and serves `/rpc`
//! without the wallet + web3 + x402 domains.
//!
//! The signatures here MUST match the real ones exactly (return types
//! included). The disabled build
//! (`cargo check --no-default-features --features tokenjuice-treesitter`) is
//! the only thing that catches drift — if a real signature changes, update the
//! mirror below until that build is green again.
//!
//! Consumers covered here (all outside `wallet`, so all must keep compiling):
//! - `core/jsonrpc.rs` — `WALLET_NOT_CONFIGURED_MESSAGE`
//! - `tools/impl/network/polymarket.rs` — `secret_material`, `status`,
//!   `WalletChain`
//! - `tinyplace/payment.rs` — `prepare_transfer`, `execute_prepared`, the
//!   param/result types, `SolanaCluster`, `solana_cluster`,
//!   `tinyplace_solana_rpc_endpoints`, `rpc::with_tinyplace_solana_endpoints`
//! - `tinyplace/manifest.rs` — `solana_cluster`, `tinyplace_solana_rpc_endpoints`,
//!   `redact_rpc_url`, `SolanaCluster::usdc_mint`
//! - `tinyplace/signal_store.rs`, `tinyplace/state.rs` — `tinyplace_signer_seed`
//! - `test_support/introspect.rs` — `prepared_quotes_for_test`,
//!   `PreparedTransaction`
//! - `core/all.rs` — `all_wallet_registered_controllers`

use serde::Serialize;

use crate::core::all::RegisteredController;
use crate::core::ControllerSchema;
use crate::rpc::RpcOutcome;

/// Error text returned by every disabled-path operation that must yield a
/// `Result`. Shared so callers/log-greps see one stable string.
const DISABLED_MSG: &str = "web3/wallet feature disabled at compile time";

/// Mirrors the real `ops::WALLET_NOT_CONFIGURED_MESSAGE` verbatim. `jsonrpc.rs`
/// compares Sentry-noise errors against this exact string, so it must not drift.
pub const WALLET_NOT_CONFIGURED_MESSAGE: &str = "wallet is not configured; run wallet setup first";

// ---------------------------------------------------------------------------
// Chain / status surface (mirrors `ops::{WalletChain, WalletAccount,
// WalletStatus, status, secret_material}`)
// ---------------------------------------------------------------------------

/// The four wallet chains. Mirrors [`super::ops::WalletChain`] (real build).
/// Callers pattern-match on `Evm` / `Solana`; the full set is kept for parity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WalletChain {
    Evm,
    Btc,
    Solana,
    Tron,
}

/// A derived per-chain account. Mirrors the fields Polymarket reads
/// (`chain`, `address`).
#[derive(Debug, Clone, Serialize)]
pub struct WalletAccount {
    pub chain: WalletChain,
    pub address: String,
}

/// Wallet status snapshot. Only `accounts` is read by out-of-module callers
/// (Polymarket EOA resolution); with the wallet disabled it is always empty.
#[derive(Debug, Clone, Default, Serialize)]
pub struct WalletStatus {
    pub accounts: Vec<WalletAccount>,
}

/// Decrypted secret handle. Polymarket reads `encrypted_mnemonic` +
/// `derivation_path` — but `secret_material` never returns `Ok` here, so these
/// are never actually produced. Kept nameable for the return type.
pub(crate) struct WalletSecretMaterial {
    pub encrypted_mnemonic: String,
    pub derivation_path: String,
}

/// Disabled: no wallet is configured, so the status carries no accounts. Kept
/// `Ok` (not `Err`) so Polymarket degrades to the clean "run wallet setup"
/// message instead of a decrypt-context error.
pub async fn status() -> Result<RpcOutcome<WalletStatus>, String> {
    log::debug!("[wallet-stub] status requested (web3 disabled) — no accounts");
    Ok(RpcOutcome::new(
        WalletStatus::default(),
        vec!["wallet disabled at compile time".to_string()],
    ))
}

/// Always errors: secret material cannot be produced with the wallet compiled
/// out. Callers `?`-propagate (Polymarket writes surface the disabled error).
pub(crate) async fn secret_material(_chain: WalletChain) -> Result<WalletSecretMaterial, String> {
    log::debug!(
        "[wallet-stub] secret_material requested (web3 disabled) — returning disabled error"
    );
    Err(DISABLED_MSG.to_string())
}

// ---------------------------------------------------------------------------
// Prepare / execute surface (mirrors `execution::{prepare_transfer,
// execute_prepared, PrepareTransferParams, ExecutePreparedParams,
// PreparedTransaction, ExecutionResult, prepared_quotes_for_test}`)
// ---------------------------------------------------------------------------

/// Inputs to `prepare_transfer`. Mirrors the fields `tinyplace/payment.rs`
/// sets. `evm_network` is `Option<()>` (the real `Option<EvmNetwork>` cannot be
/// named with `defaults` compiled out); the only external constructor passes
/// `None`, so the placeholder is behaviourally identical.
#[derive(Debug, Clone)]
pub struct PrepareTransferParams {
    pub chain: WalletChain,
    pub to_address: String,
    pub amount_raw: String,
    pub asset_symbol: Option<String>,
    pub evm_network: Option<()>,
}

/// Inputs to `execute_prepared`. Mirrors the real type.
#[derive(Debug, Default, Clone)]
pub struct ExecutePreparedParams {
    pub quote_id: String,
    pub confirmed: bool,
}

/// A prepared quote. Out-of-module callers read `quote_id` (`tinyplace`) and
/// serialize the collection (`test_support`); `Serialize` + that field are the
/// only requirements. The real type is far richer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedTransaction {
    pub quote_id: String,
}

/// Result of an execute. Out-of-module callers read `transaction_hash`
/// (`tinyplace/payment.rs`); `execute_prepared` never returns `Ok`, so it is
/// never actually produced.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub transaction_hash: String,
}

/// Disabled: no transfer can be prepared with the wallet compiled out.
pub async fn prepare_transfer(
    _params: PrepareTransferParams,
) -> Result<RpcOutcome<PreparedTransaction>, String> {
    log::debug!(
        "[wallet-stub] prepare_transfer requested (web3 disabled) — returning disabled error"
    );
    Err(DISABLED_MSG.to_string())
}

/// Disabled: no prepared transfer can be executed with the wallet compiled out.
pub async fn execute_prepared(
    _params: ExecutePreparedParams,
) -> Result<RpcOutcome<ExecutionResult>, String> {
    log::debug!(
        "[wallet-stub] execute_prepared requested (web3 disabled) — returning disabled error"
    );
    Err(DISABLED_MSG.to_string())
}

/// Always empty: there is no quote store when the wallet is compiled out.
pub fn prepared_quotes_for_test() -> Vec<PreparedTransaction> {
    Vec::new()
}

/// Disabled: the tiny.place signer seed derives from the Solana wallet key,
/// which does not exist. Callers `?`-propagate → "unlock wallet" prompt.
pub(crate) async fn tinyplace_signer_seed() -> Result<[u8; 32], String> {
    log::debug!(
        "[wallet-stub] tinyplace_signer_seed requested (web3 disabled) — returning disabled error"
    );
    Err(DISABLED_MSG.to_string())
}

// ---------------------------------------------------------------------------
// Solana cluster metadata (mirrors `defaults::{SolanaCluster, solana_cluster,
// tinyplace_solana_rpc_endpoints}`)
// ---------------------------------------------------------------------------

/// Public Solana clusters. Mirrors [`super::defaults::SolanaCluster`] including
/// the `usdc_mint` accessor `tinyplace/{payment,manifest}.rs` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolanaCluster {
    Mainnet,
    Devnet,
}

impl SolanaCluster {
    /// USDC SPL-token mint address for the cluster. Same literals as the real
    /// `defaults` module so any residual log/compare paths see stable values.
    pub fn usdc_mint(self) -> &'static str {
        match self {
            Self::Mainnet => "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            Self::Devnet => "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
        }
    }
}

/// Resolve the configured Solana cluster. With the wallet disabled nothing
/// settles on-chain, so the default (Mainnet) is returned unconditionally.
pub fn solana_cluster() -> SolanaCluster {
    SolanaCluster::Mainnet
}

/// Always empty: the wallet is compiled out, so there are no settlement
/// endpoints. `tinyplace/manifest.rs`'s balance loop therefore no-ops and the
/// balance shows as unknown — the correct degraded state.
pub fn tinyplace_solana_rpc_endpoints() -> Vec<String> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// rpc submodule (mirrors `rpc::{redact_rpc_url, with_tinyplace_solana_endpoints}`)
// ---------------------------------------------------------------------------

pub(crate) mod rpc {
    /// Redact an RPC URL for logging. With the wallet disabled we never build
    /// real endpoints, but `tinyplace/manifest.rs` still calls this on any URL
    /// it iterates; return a constant so no token can ever leak.
    pub(crate) fn redact_rpc_url(_raw: &str) -> String {
        "<rpc-redacted>".to_string()
    }

    /// Run `fut` unchanged — there is no tiny.place endpoint scope to install
    /// when the wallet is compiled out. Matches the real generic signature so
    /// `tinyplace/payment.rs` type-checks; the future itself resolves to a
    /// disabled error from `prepare_transfer`.
    pub(crate) async fn with_tinyplace_solana_endpoints<F, T>(_endpoints: Vec<String>, fut: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        fut.await
    }
}

/// Re-export mirrors the real `pub(crate) use rpc::redact_rpc_url;` so
/// `wallet::redact_rpc_url` resolves at the module root for `tinyplace`.
pub(crate) use rpc::redact_rpc_url;

// ---------------------------------------------------------------------------
// Agent-tool facade (mirrors `pub mod tools`, re-exported via tools/mod.rs)
// ---------------------------------------------------------------------------

/// Empty tools module. `tools/mod.rs` glob-re-exports `wallet::tools::*`; the
/// concrete wallet tool constructors it names are `#[cfg(feature = "web3")]`
/// at their registration sites, so nothing is referenced here when off.
pub mod tools {}

// ---------------------------------------------------------------------------
// Controller registration (mirrors `schemas::{all_wallet_registered_controllers,
// all_wallet_controller_schemas}`)
// ---------------------------------------------------------------------------

/// No wallet controllers are registered when the wallet is compiled out — the
/// `openhuman.wallet_*` RPCs become unknown-method.
pub fn all_wallet_registered_controllers() -> Vec<RegisteredController> {
    Vec::new()
}

/// No wallet controller schemas when the wallet is compiled out.
pub fn all_wallet_controller_schemas() -> Vec<ControllerSchema> {
    Vec::new()
}

// This module is only compiled when the `web3` feature is OFF (see the
// `#[cfg(not(feature = "web3"))] mod stub;` gate in `super`), so a plain
// `#[cfg(test)]` here already runs only in the disabled build — it locks in the
// degraded contract that always-on callers (tinyplace payments, Polymarket
// writes) depend on.
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn status_reports_no_accounts() {
        let outcome = status().await.expect("stub status is always Ok");
        assert!(
            outcome.value.accounts.is_empty(),
            "disabled wallet must expose no accounts"
        );
    }

    #[tokio::test]
    async fn secret_material_is_disabled_error() {
        // `WalletSecretMaterial` intentionally omits `Debug` (mirrors the real
        // type, which never logs a mnemonic), so match rather than `expect_err`.
        match secret_material(WalletChain::Evm).await {
            Ok(_) => panic!("secret material must be unavailable when the wallet is compiled out"),
            Err(msg) => assert_eq!(msg, DISABLED_MSG),
        }
    }

    #[tokio::test]
    async fn prepare_transfer_is_disabled_error() {
        let err = prepare_transfer(PrepareTransferParams {
            chain: WalletChain::Solana,
            to_address: "recipient".to_string(),
            amount_raw: "1".to_string(),
            asset_symbol: None,
            evm_network: None,
        })
        .await
        .expect_err("no transfer can be prepared when the wallet is compiled out");
        assert_eq!(err, DISABLED_MSG);
    }

    #[tokio::test]
    async fn execute_prepared_is_disabled_error() {
        let err = execute_prepared(ExecutePreparedParams {
            quote_id: "quote".to_string(),
            confirmed: true,
        })
        .await
        .expect_err("no prepared transfer can execute when the wallet is compiled out");
        assert_eq!(err, DISABLED_MSG);
    }

    #[tokio::test]
    async fn tinyplace_signer_seed_is_disabled_error() {
        let err = tinyplace_signer_seed()
            .await
            .expect_err("no signer seed derives when the wallet is compiled out");
        assert_eq!(err, DISABLED_MSG);
    }

    #[test]
    fn prepared_quotes_and_rpc_endpoints_are_empty() {
        assert!(prepared_quotes_for_test().is_empty());
        assert!(tinyplace_solana_rpc_endpoints().is_empty());
    }

    #[test]
    fn solana_cluster_defaults_to_mainnet_with_stable_usdc_mint() {
        assert_eq!(solana_cluster(), SolanaCluster::Mainnet);
        assert_eq!(
            SolanaCluster::Mainnet.usdc_mint(),
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
        );
    }

    #[test]
    fn registration_entry_points_are_empty() {
        assert!(all_wallet_registered_controllers().is_empty());
        assert!(all_wallet_controller_schemas().is_empty());
    }
}
