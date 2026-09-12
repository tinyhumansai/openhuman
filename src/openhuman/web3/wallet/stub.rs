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
//! (`cargo check --no-default-features`) is
//! the only thing that catches drift — if a real signature changes, update the
//! mirror below until that build is green again.
//!
//! Consumers covered here (all outside `wallet`, so all must keep compiling):
//! - `core/jsonrpc.rs` — `WALLET_NOT_CONFIGURED_MESSAGE`
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

/// A derived per-chain account. Mirrors the fields wallet-signed callers read
/// (`chain`, `address`).
#[derive(Debug, Clone, Serialize)]
pub struct WalletAccount {
    pub chain: WalletChain,
    pub address: String,
}

/// Wallet status snapshot. Only `accounts` is read by out-of-module callers
/// (EOA resolution for wallet-signed writes); with the wallet disabled it is always empty.
#[derive(Debug, Clone, Default, Serialize)]
pub struct WalletStatus {
    pub accounts: Vec<WalletAccount>,
}

/// Decrypted secret handle. Wallet-signed callers read `encrypted_mnemonic` +
/// `derivation_path` — but `secret_material` never returns `Ok` here, so these
/// are never actually produced. Kept nameable for the return type.
pub(crate) struct WalletSecretMaterial {
    pub encrypted_mnemonic: String,
    pub derivation_path: String,
}

/// Disabled: no wallet is configured, so the status carries no accounts. Kept
/// `Ok` (not `Err`) so wallet-signed callers degrade to the clean "run wallet setup"
/// message instead of a decrypt-context error.
pub async fn status() -> Result<RpcOutcome<WalletStatus>, String> {
    log::debug!("[wallet-stub] status requested (web3 disabled) — no accounts");
    Ok(RpcOutcome::new(
        WalletStatus::default(),
        vec!["wallet disabled at compile time".to_string()],
    ))
}

/// Always errors: secret material cannot be produced with the wallet compiled
/// out. Callers `?`-propagate (wallet-signed writes surface the disabled error).
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

/// Inputs to `prepare_transfer`. `evm_network` is `Option<()>` because the
/// real `Option<EvmNetwork>` cannot be named with `defaults` compiled out.
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

/// A prepared quote. Test support serializes this collection; the real type is
/// far richer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedTransaction {
    pub quote_id: String,
}

/// Result of an execute. `execute_prepared` never returns `Ok`, so it is
/// never actually produced in the disabled build.
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

// ---------------------------------------------------------------------------
// Solana cluster metadata (mirrors `defaults::{SolanaCluster, solana_cluster}`)
// ---------------------------------------------------------------------------

/// Public Solana clusters. Mirrors [`super::defaults::SolanaCluster`].
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
// degraded contract that always-on callers depend on.
#[cfg(test)]
#[path = "stub_tests.rs"]
mod tests;
