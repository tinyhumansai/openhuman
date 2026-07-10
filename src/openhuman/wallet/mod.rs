//! Core-owned wallet onboarding metadata, derived account visibility, and
//! the agent-facing execution surface (balances, transfers, swaps,
//! contract calls). See [`execution`] for the prepare/confirm/execute flow
//! and [`chains`] for the per-chain signing/broadcast implementations.

mod abi;
mod chains;
mod defaults;
mod execution;
mod ops;
pub(crate) mod rpc;
mod schemas;
pub mod tools;

#[cfg(test)]
pub(crate) mod test_support;

pub use abi::encode_erc20_transfer;
/// 32-byte Ed25519 seed for the tiny.place LocalSigner. Derived from the user's
/// primary Solana wallet key via SLIP-0010; consumed in-process and never exposed.
pub(crate) use chains::solana::tinyplace_signer_seed;
pub use defaults::{
    asset_catalog, default_rpc_url, env_var_for_chain, evm_asset_catalog, explorer_tx_url,
    find_asset, find_asset_for_network, network_defaults, rpc_source_for_chain, rpc_url_for_chain,
    rpc_url_for_evm_network, solana_cluster, tinyplace_solana_rpc_endpoints, EvmNetwork, RpcSource,
    SolanaCluster, WalletAssetDefinition, WalletNetworkDefaults,
};
pub use execution::{
    balances, chain_status, execute_prepared, lookup_tx,
    network_defaults as wallet_network_defaults, prepare_transfer, prepared_quotes_for_test,
    supported_assets, tx_receipt, tx_status, BalanceInfo, ChainStatus, ExecutePreparedParams,
    ExecutionResult, PrepareTransferParams, PreparedKind, PreparedStatus, PreparedTransaction,
    ProviderStatus, SupportedAsset, TxLookupInfo, TxReceiptInfo, TxState, TxStatusInfo,
};
/// Crate-internal signing primitives the `web3` layer builds on. Not part of
/// the agent / RPC surface.
pub(crate) use execution::{sign_and_broadcast_evm, sign_and_broadcast_solana};
pub(crate) use ops::secret_material;
pub use ops::{
    reveal_recovery_phrase, setup, status, RevealRecoveryPhraseResult, WalletAccount, WalletChain,
    WalletSetupParams, WalletSetupSource, WalletStatus, WALLET_NOT_CONFIGURED_MESSAGE,
};
/// Reduce an RPC URL to `scheme://host` for logging so private provider tokens
/// embedded in the path/query never reach the logs.
pub(crate) use rpc::redact_rpc_url;
pub use schemas::{
    all_controller_schemas, all_registered_controllers, all_wallet_controller_schemas,
    all_wallet_registered_controllers, schemas, wallet_schemas,
};
