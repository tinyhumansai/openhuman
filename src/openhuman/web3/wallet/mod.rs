//! Core-owned wallet onboarding metadata, derived account visibility, and
//! the agent-facing execution surface (balances, transfers, swaps,
//! contract calls). See [`execution`] for the prepare/confirm/execute flow
//! and [`chains`] for the per-chain signing/broadcast implementations.
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
mod abi;
#[cfg(feature = "web3")]
mod chains;
#[cfg(feature = "web3")]
mod defaults;
#[cfg(feature = "web3")]
mod execution;
#[cfg(feature = "web3")]
mod ops;
#[cfg(feature = "web3")]
pub(crate) mod rpc;

#[cfg(feature = "web3")]
mod schemas;
#[cfg(feature = "web3")]
pub mod tools;
/// The host side of the wallet primitives' `Transport` seam — endpoint resolution,
/// failover and redaction stay here, where the config lives.
#[cfg(feature = "web3")]
pub(crate) mod transport;

#[cfg(all(test, feature = "web3"))]
pub(crate) mod test_support;

#[cfg(feature = "web3")]
pub use abi::encode_erc20_transfer;
#[cfg(feature = "web3")]
pub use defaults::{
    asset_catalog, default_rpc_url, env_var_for_chain, evm_asset_catalog, explorer_tx_url,
    find_asset, find_asset_for_network, network_defaults, rpc_source_for_chain, rpc_url_for_chain,
    rpc_url_for_evm_network, solana_cluster, EvmNetwork, RpcSource, SolanaCluster,
    WalletAssetDefinition, WalletNetworkDefaults,
};
#[cfg(feature = "web3")]
pub use execution::{
    balances, chain_status, execute_prepared, lookup_tx,
    network_defaults as wallet_network_defaults, prepare_transfer, prepared_quotes_for_test,
    supported_assets, tx_receipt, tx_status, BalanceInfo, ChainStatus, ExecutePreparedParams,
    ExecutionResult, PrepareTransferParams, PreparedKind, PreparedStatus, PreparedTransaction,
    ProviderStatus, SupportedAsset, TxLookupInfo, TxReceiptInfo, TxState, TxStatusInfo,
};
/// Crate-internal signing primitives the `web3` layer builds on. Not part of
/// the agent / RPC surface.
#[cfg(feature = "web3")]
pub(crate) use execution::{sign_and_broadcast_evm, sign_and_broadcast_solana};
#[cfg(feature = "web3")]
pub(crate) use ops::secret_material;
#[cfg(feature = "web3")]
pub use ops::{
    reveal_recovery_phrase, setup, status, RevealRecoveryPhraseResult, WalletAccount, WalletChain,
    WalletSetupParams, WalletSetupSource, WalletStatus, WALLET_NOT_CONFIGURED_MESSAGE,
};
#[cfg(feature = "web3")]
pub use schemas::{
    all_controller_schemas, all_registered_controllers, all_wallet_controller_schemas,
    all_wallet_registered_controllers, schemas, wallet_schemas,
};

// ---------------------------------------------------------------------------
// Disabled facade — compiled only when the `web3` feature is OFF.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "web3"))]
mod stub;
#[cfg(not(feature = "web3"))]
pub use stub::*;
