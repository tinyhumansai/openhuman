//! Host adapters over the wallet engine: the read and write operations wrapped
//! in the controller contract's [`Outcome`].
//!
//! The logic (balances, transfers, lookups, the quote store) lives in
//! `tinywallet_web3::crypto`. What this file adds is the `Outcome` envelope and
//! its log lines, and the engine the operations run on
//! ([`crate::web3::seams::engine`]).

use crate::config::rpc as config_rpc;
use crate::core::Outcome;
use crate::modules::{wallet as wallet_module, wallet_service};
use tinywallet_bus::names::methods;
use tinywallet_bus::wire::{NativeWalletServiceRequest, NativeWalletTransactionRequest};

pub use tinywallet_bus::wire::{
    BalanceInfo, ChainStatus, ExecutePreparedParams, ExecutionResult, PrepareTransferParams,
    PreparedKind, PreparedStatus, PreparedTransaction, ProviderStatus, SupportedAsset,
    TxLookupInfo, TxReceiptInfo, TxState, TxStatusInfo,
};

use tinywallet_bus::wire::{EvmNetwork, WalletNetworkDefaults};

use tinywallet_web3::crypto::wallet::WalletChain;

/// Default row for every supported network, with the host's endpoints.
pub async fn network_defaults() -> Result<Outcome<Vec<WalletNetworkDefaults>>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let result = wallet_service::with_native_wallet(&config, |service| async {
        wallet_module::call(
            &config,
            methods::NATIVE_WALLET_NETWORK_DEFAULTS,
            NativeWalletServiceRequest { service },
        )
        .await
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(Outcome::new(
        result,
        vec!["wallet network defaults listed".to_string()],
    ))
}

/// Every asset the wallet catalogues.
pub async fn supported_assets() -> Result<Outcome<Vec<SupportedAsset>>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let result = wallet_service::with_native_wallet(&config, |service| async {
        wallet_module::call(
            &config,
            methods::NATIVE_WALLET_SUPPORTED_ASSETS,
            NativeWalletServiceRequest { service },
        )
        .await
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(Outcome::new(
        result,
        vec!["wallet supported_assets listed".to_string()],
    ))
}

/// Which chains have an account and a provider.
pub async fn chain_status() -> Result<Outcome<Vec<ChainStatus>>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let result = wallet_service::with_native_wallet(&config, |service| async {
        wallet_module::call(
            &config,
            methods::NATIVE_WALLET_CHAIN_STATUS,
            NativeWalletServiceRequest { service },
        )
        .await
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(Outcome::new(
        result,
        vec!["wallet chain_status listed".to_string()],
    ))
}

/// Live native balances.
pub async fn balances() -> Result<Outcome<Vec<BalanceInfo>>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let result = wallet_service::with_native_wallet(&config, |service| async {
        wallet_module::call(
            &config,
            methods::NATIVE_WALLET_BALANCES,
            NativeWalletServiceRequest { service },
        )
        .await
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(Outcome::new(
        result,
        vec!["wallet balances listed".to_string()],
    ))
}

/// Validate a transfer and store a quote for it.
pub async fn prepare_transfer(
    params: PrepareTransferParams,
) -> Result<Outcome<PreparedTransaction>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let owner = crate::security::approval::APPROVAL_CHAT_CONTEXT
        .try_with(|context| tinywallet_bus::wire::QuoteOwner {
            thread_id: context.thread_id.clone(),
            client_id: context.client_id.clone(),
        })
        .ok();
    let quote = wallet_service::prepare_native_transfer(&config, owner, params)
        .await
        .map_err(|error| error.to_string())?;
    Ok(Outcome::new(
        quote,
        vec!["wallet transfer prepared".to_string()],
    ))
}

/// Confirm and execute a prepared transfer.
pub async fn execute_prepared(
    params: ExecutePreparedParams,
) -> Result<Outcome<ExecutionResult>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let owner = crate::security::approval::APPROVAL_CHAT_CONTEXT
        .try_with(|context| tinywallet_bus::wire::QuoteOwner {
            thread_id: context.thread_id.clone(),
            client_id: context.client_id.clone(),
        })
        .ok();
    let result = wallet_service::execute_native_transfer(&config, owner, params)
        .await
        .map_err(|error| error.to_string())?;
    Ok(Outcome::new(
        result,
        vec!["wallet transaction broadcast".to_string()],
    ))
}

/// Check the on-chain lifecycle state of a broadcast transaction.
pub async fn tx_status(
    chain: WalletChain,
    evm_network: Option<EvmNetwork>,
    hash: &str,
) -> Result<Outcome<TxStatusInfo>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let result = wallet_service::with_native_wallet(&config, |service| async {
        wallet_module::call(
            &config,
            methods::NATIVE_WALLET_TX_STATUS,
            NativeWalletTransactionRequest {
                service,
                chain,
                evm_network,
                hash: hash.to_owned(),
            },
        )
        .await
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(Outcome::new(
        result,
        vec!["wallet tx status fetched".to_string()],
    ))
}

/// Fetch the receipt of a broadcast transaction.
pub async fn tx_receipt(
    chain: WalletChain,
    evm_network: Option<EvmNetwork>,
    hash: &str,
) -> Result<Outcome<TxReceiptInfo>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let result = wallet_service::with_native_wallet(&config, |service| async {
        wallet_module::call(
            &config,
            methods::NATIVE_WALLET_TX_RECEIPT,
            NativeWalletTransactionRequest {
                service,
                chain,
                evm_network,
                hash: hash.to_owned(),
            },
        )
        .await
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(Outcome::new(
        result,
        vec!["wallet tx receipt fetched".to_string()],
    ))
}

/// Look up the raw transaction payload by hash.
pub async fn lookup_tx(
    chain: WalletChain,
    evm_network: Option<EvmNetwork>,
    hash: &str,
) -> Result<Outcome<TxLookupInfo>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let result = wallet_service::with_native_wallet(&config, |service| async {
        wallet_module::call(
            &config,
            methods::NATIVE_WALLET_LOOKUP_TX,
            NativeWalletTransactionRequest {
                service,
                chain,
                evm_network,
                hash: hash.to_owned(),
            },
        )
        .await
    })
    .await
    .map_err(|error| error.to_string())?;
    Ok(Outcome::new(
        result,
        vec!["wallet tx looked up".to_string()],
    ))
}

/// The quotes that can still be executed. Test support and the introspection
/// harness read the engine's store through this.
pub fn prepared_quotes_for_test() -> Vec<PreparedTransaction> {
    Vec::new()
}
