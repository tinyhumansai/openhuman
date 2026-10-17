//! Core-owned wallet onboarding metadata, derived account visibility, and
//! the agent-facing execution surface (balances, transfers, transaction
//! lookups).
//!
//! ## Where the logic lives
//!
//! The wallet engine — balances, the prepare/confirm/execute flow, signing,
//! transaction encoding, provider interaction, and agent tools — lives in the
//! compiled TinyWallet module. This module holds host configuration, custody,
//! approvals, product RPC adapters, and module lifecycle:
//!
//! - [`ops`] — setup, status, recovery-phrase reveal and `secret_material`
//!   (keyring, consent, credentials). Stays here: it is the host's state.
//! - [`execution`] — typed module operations adapted to product controller
//!   responses.
//! - [`endpoints`] / [`rpc`] — host endpoint configuration and guarded,
//!   credential-aware transport callbacks.
//! - `schemas` — the controllers, with their RPC namespace strings.
//! - [`crate::modules::wallet_service`] owns the native module service lifetime.
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
mod schemas;
#[cfg(all(test, feature = "web3"))]
pub(crate) mod test_support;
/// Wallet agent tool adapters over the declarations in `tinywallet-bus`.
#[cfg(feature = "web3")]
pub mod tools;

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
pub use ops::{
    derive_recovery_material, generate_recovery_phrase, reveal_recovery_phrase, setup, status,
    validate_recovery_phrase, GeneratedRecoveryPhrase, RecoveryPhraseValidation,
    RevealRecoveryPhraseResult, WalletAccount, WalletChain, WalletRecoveryMaterial,
    WalletSetupParams, WalletSetupSource, WalletStatus, WALLET_NOT_CONFIGURED_MESSAGE,
};
#[cfg(feature = "web3")]
pub use schemas::{
    all_controller_schemas, all_registered_controllers, all_wallet_controller_schemas,
    all_wallet_registered_controllers, schemas, wallet_schemas,
};
#[cfg(feature = "web3")]
pub use tinywallet_bus::wire::{
    EvmNetwork, RpcSource, SolanaCluster, WalletAssetDefinition, WalletNetworkDefaults,
};

// ---------------------------------------------------------------------------
// Disabled facade — compiled only when the `web3` feature is OFF.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "web3"))]
mod stub;
#[cfg(not(feature = "web3"))]
pub use stub::*;
