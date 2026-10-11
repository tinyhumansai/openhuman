//! Core-owned wallet onboarding metadata, derived account visibility, and
//! the agent-facing execution surface (balances, transfers, transaction
//! lookups).
//!
//! ## Where the logic lives
//!
//! The wallet engine — balances, the prepare/confirm/execute flow, the per-chain
//! signing choreography and the agent tools — lives in
//! `tinywallet_web3::crypto`. This module is the host adapter over it:
//!
//! - [`ops`] — setup, status, recovery-phrase reveal and `secret_material`
//!   (keyring, consent, credentials). Stays here: it is the host's state.
//! - [`execution`] — the engine's operations wrapped in the controller
//!   contract's `Outcome`.
//! - [`endpoints`] — the `OPENHUMAN_WALLET_RPC_*` / `OPENHUMAN_SOLANA_CLUSTER`
//!   environment resolution the engine's `RpcEndpoints` seam asks.
//! - [`transport`] / [`rpc`] — the host `Transport` seam: endpoint resolution,
//!   URL redaction and error classification over the shared `reqwest` client.
//! - `schemas` — the controllers, with their RPC namespace strings.
//! - The seam implementations and the process-wide engine are in
//!   [`crate::web3::seams`].
//!
//! ## Compile-time gate (`web3` feature)
//!
//! `pub mod wallet;` is ALWAYS compiled — it is a facade. The real
//! implementation (the submodules below and their re-exports) is gated behind
//! the default-ON `web3` Cargo feature (shared with `openhuman::web3` +
//! `openhuman::web3::x402`). When the feature is off, [`stub`] takes its place and
//! exposes the same public surface that always-on / other-gated callers depend
//! on (`WALLET_NOT_CONFIGURED_MESSAGE`, `status`, `secret_material`,
//! `WalletChain`, `prepare_transfer`, `execute_prepared`, the prepare/execute
//! param + result types, `solana_cluster` / `SolanaCluster`,
//! `prepared_quotes_for_test`, and the controller-registration
//! entry points) with no-op / disabled-error bodies. Signatures MUST match the real ones;
//! the disabled build
//! (`cargo check --no-default-features`) is
//! the only thing that catches drift.

#[cfg(feature = "web3")]
pub(crate) mod endpoints;
#[cfg(feature = "web3")]
mod execution;
#[cfg(feature = "web3")]
mod ops;
#[cfg(feature = "web3")]
pub(crate) mod rpc;

#[cfg(feature = "web3")]
pub(crate) mod registration;
#[cfg(feature = "web3")]
mod schemas;
/// The host side of the wallet primitives' `Transport` seam — endpoint resolution,
/// failover and redaction stay here, where the config lives.
#[cfg(feature = "web3")]
pub(crate) mod transport;

#[cfg(all(test, feature = "web3"))]
pub(crate) mod test_support;

#[cfg(feature = "web3")]
pub use endpoints::solana_cluster;
#[cfg(feature = "web3")]
pub use execution::{
    balances, chain_status, execute_prepared, lookup_tx,
    network_defaults as wallet_network_defaults, prepare_transfer, prepared_quotes_for_test,
    supported_assets, tx_receipt, tx_status, BalanceInfo, ChainStatus, ExecutePreparedParams,
    ExecutionResult, PrepareTransferParams, PreparedKind, PreparedStatus, PreparedTransaction,
    ProviderStatus, SupportedAsset, TxLookupInfo, TxReceiptInfo, TxState, TxStatusInfo,
};
#[cfg(feature = "web3")]
pub(crate) use ops::secret_material;
#[cfg(feature = "web3")]
pub use schemas::{
    all_controller_schemas, all_registered_controllers, all_wallet_controller_schemas,
    all_wallet_registered_controllers, schemas, wallet_schemas,
};
#[cfg(feature = "web3")]
pub use {
    ops::reveal_recovery_phrase, ops::setup, ops::status, ops::RevealRecoveryPhraseResult,
    ops::WalletSetupParams,
};

// Retained for the always-on RPC/observability boundary until #7406 exposes
// this shared producer error through the released TinyWallet bus contract.
// Embed must not acquire an implementation dependency for this sentinel.
#[cfg(feature = "web3")]
pub use tinywallet_web3::quote::WALLET_NOT_CONFIGURED_MESSAGE;

// ---------------------------------------------------------------------------
// Disabled facade — compiled only when the `web3` feature is OFF.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "web3"))]
mod stub;
#[cfg(not(feature = "web3"))]
pub use stub::*;
