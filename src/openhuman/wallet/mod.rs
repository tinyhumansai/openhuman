//! Core-owned wallet onboarding metadata, derived account visibility, and
//! the agent-facing execution surface (balances, transfers, swaps,
//! contract calls). See [`execution`] for the prepare/confirm/execute flow.

mod abi;
mod defaults;
mod execution;
mod ops;
mod rpc;
mod schemas;

pub use abi::encode_erc20_transfer;
pub use defaults::{
    asset_catalog, default_rpc_url, env_var_for_chain, explorer_tx_url, find_asset,
    network_defaults, rpc_source_for_chain, rpc_url_for_chain, RpcSource, WalletAssetDefinition,
    WalletNetworkDefaults,
};
pub use execution::{
    balances, chain_status, execute_prepared, network_defaults as wallet_network_defaults,
    prepare_contract_call, prepare_swap, prepare_transfer, prepared_quotes_for_test,
    supported_assets, BalanceInfo, ChainStatus, ExecutePreparedParams, ExecutionResult,
    PrepareContractCallParams, PrepareSwapParams, PrepareTransferParams, PreparedKind,
    PreparedStatus, PreparedTransaction, ProviderStatus, SupportedAsset,
};
pub(crate) use ops::secret_material;
pub use ops::{
    setup, status, WalletAccount, WalletChain, WalletSetupParams, WalletSetupSource, WalletStatus,
};
pub use schemas::{
    all_controller_schemas, all_registered_controllers, all_wallet_controller_schemas,
    all_wallet_registered_controllers, schemas, wallet_schemas,
};
