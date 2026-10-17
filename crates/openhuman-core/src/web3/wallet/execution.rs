//! Typed TinyWallet module operations adapted to product controller results.
//! This host supplies configuration, custody, transport and approvals; wallet
//! algorithms and user-scoped service state remain in the compiled module.

use crate::core::runtime::DispatchCtx;
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

use super::ops::WalletChain;

/// Default row for every supported network, with the host's endpoints.
pub async fn network_defaults(
    ctx: &DispatchCtx,
) -> Result<Outcome<Vec<WalletNetworkDefaults>>, String> {
    let config = ctx.load_config().await?;
    let result = wallet_service::with_native_wallet(ctx, &config, |service| async {
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
pub async fn supported_assets(ctx: &DispatchCtx) -> Result<Outcome<Vec<SupportedAsset>>, String> {
    let config = ctx.load_config().await?;
    let result = wallet_service::with_native_wallet(ctx, &config, |service| async {
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
pub async fn chain_status(ctx: &DispatchCtx) -> Result<Outcome<Vec<ChainStatus>>, String> {
    let config = ctx.load_config().await?;
    let result = wallet_service::with_native_wallet(ctx, &config, |service| async {
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
pub async fn balances(ctx: &DispatchCtx) -> Result<Outcome<Vec<BalanceInfo>>, String> {
    let config = ctx.load_config().await?;
    let result = wallet_service::with_native_wallet(ctx, &config, |service| async {
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
    ctx: &DispatchCtx,
    params: PrepareTransferParams,
    owner: Option<tinywallet_bus::wire::QuoteOwner>,
) -> Result<Outcome<PreparedTransaction>, String> {
    let config = ctx.load_config().await?;
    let quote = wallet_service::prepare_native_transfer(ctx, &config, owner, params)
        .await
        .map_err(|error| error.to_string())?;
    Ok(Outcome::new(
        quote,
        vec!["wallet transfer prepared".to_string()],
    ))
}

/// Confirm and execute a prepared transfer.
pub async fn execute_prepared(
    ctx: &DispatchCtx,
    params: ExecutePreparedParams,
    owner: Option<tinywallet_bus::wire::QuoteOwner>,
) -> Result<Outcome<ExecutionResult>, String> {
    let config = ctx.load_config().await?;
    let result = wallet_service::execute_native_transfer(ctx, &config, owner, params)
        .await
        .map_err(|error| error.to_string())?;
    Ok(Outcome::new(
        result,
        vec!["wallet transaction broadcast".to_string()],
    ))
}

/// Check the on-chain lifecycle state of a broadcast transaction.
pub async fn tx_status(
    ctx: &DispatchCtx,
    chain: WalletChain,
    evm_network: Option<EvmNetwork>,
    hash: &str,
) -> Result<Outcome<TxStatusInfo>, String> {
    let config = ctx.load_config().await?;
    let result = wallet_service::with_native_wallet(ctx, &config, |service| async {
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
    ctx: &DispatchCtx,
    chain: WalletChain,
    evm_network: Option<EvmNetwork>,
    hash: &str,
) -> Result<Outcome<TxReceiptInfo>, String> {
    let config = ctx.load_config().await?;
    let result = wallet_service::with_native_wallet(ctx, &config, |service| async {
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
    ctx: &DispatchCtx,
    chain: WalletChain,
    evm_network: Option<EvmNetwork>,
    hash: &str,
) -> Result<Outcome<TxLookupInfo>, String> {
    let config = ctx.load_config().await?;
    let result = wallet_service::with_native_wallet(ctx, &config, |service| async {
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

/// Encode ERC-20 transfer calldata in the loaded wallet module.
pub async fn encode_erc20_transfer(
    ctx: &DispatchCtx,
    to_address: &str,
    amount_raw: &str,
) -> Result<String, String> {
    let config = ctx.load_config().await?;
    wallet_module::encode_erc20_transfer(&config, to_address, amount_raw)
        .await
        .map_err(|error| error.to_string())
}

/// The quotes that can still be executed. Test support and the introspection
/// harness read the engine's store through this.
pub fn prepared_quotes_for_test() -> Vec<PreparedTransaction> {
    Vec::new()
}
