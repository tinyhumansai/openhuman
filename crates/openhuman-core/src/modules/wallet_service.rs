//! Serialized, user-scoped access to the module-owned native wallet service.

use crate::core::runtime::DispatchCtx;
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use tinywallet_bus::names::methods;
use tinywallet_bus::wire::{
    EvmNetwork, NativeWalletCloseRequest, NativeWalletConfigRequest, NativeWalletConfigResponse,
    NativeWalletServiceHandle, RpcEndpointKey, WalletAccountsResponse,
};
use zeroize::Zeroize;

use crate::config::Config;

use super::wallet::{self, WalletCallError};

struct NativeWalletService {
    config: Config,
    workspace: PathBuf,
    configuration: NativeWalletConfigRequest,
    accounts: WalletAccountsResponse,
    response: NativeWalletConfigResponse,
    prepared_quotes: HashMap<String, tinywallet_bus::wire::PreparedTransaction>,
}

static NATIVE_WALLET_SERVICE: OnceLock<tokio::sync::Mutex<Option<NativeWalletService>>> =
    OnceLock::new();
static NATIVE_WALLET_SHUTDOWN_HOOK_REGISTERED: AtomicBool = AtomicBool::new(false);

fn native_wallet_service() -> &'static tokio::sync::Mutex<Option<NativeWalletService>> {
    NATIVE_WALLET_SERVICE.get_or_init(|| tokio::sync::Mutex::new(None))
}

/// Run one operation under the current user's module service. The lock spans
/// the complete bus operation so a user or endpoint change cannot replace the
/// module's quote store while an earlier request is still using it.
pub(crate) async fn with_native_wallet<R, F, Fut>(
    ctx: &DispatchCtx,
    config: &Config,
    operation: F,
) -> Result<R, WalletCallError>
where
    F: FnOnce(NativeWalletServiceHandle) -> Fut + Send,
    Fut: Future<Output = Result<R, WalletCallError>> + Send,
    R: Send,
{
    let mut active = native_wallet_service().lock().await;
    register_native_wallet_shutdown_hook();
    let accounts = super::wallet_host::wallet_accounts_snapshot(ctx)
        .await
        .map_err(WalletCallError::Failed)?;
    let configuration = native_wallet_config_request();
    let workspace = config.workspace_dir.clone();

    ensure_native_wallet(config, &mut active, workspace, configuration, accounts).await?;

    let Some(service) = active
        .as_ref()
        .map(|service| service.response.service.clone())
    else {
        return Err(WalletCallError::Failed(
            "native wallet service configuration did not produce a handle".into(),
        ));
    };
    operation(service).await
}

async fn ensure_native_wallet(
    config: &Config,
    active: &mut Option<NativeWalletService>,
    workspace: PathBuf,
    configuration: NativeWalletConfigRequest,
    accounts: WalletAccountsResponse,
) -> Result<(), WalletCallError> {
    if active.as_ref().is_some_and(|service| {
        service_requires_replacement(service, &workspace, &configuration, &accounts)
    }) {
        if let Some(service) = active.as_ref() {
            close_native_wallet(config, service).await?;
        }
        *active = None;
    }

    if active.is_none() {
        let response: NativeWalletConfigResponse = wallet::call(
            config,
            methods::NATIVE_WALLET_CONFIGURE,
            configuration.clone(),
        )
        .await?;
        if let Err(error) = super::wallet_host::bind_endpoint_references(
            &response.service.handle,
            &response.endpoint_references,
        ) {
            let _ = wallet::call::<_, ()>(
                config,
                methods::NATIVE_WALLET_CLOSE,
                NativeWalletCloseRequest {
                    service: response.service.clone(),
                },
            )
            .await;
            return Err(WalletCallError::Failed(error));
        }
        *active = Some(NativeWalletService {
            config: config.clone(),
            workspace,
            configuration,
            accounts,
            response,
            prepared_quotes: HashMap::new(),
        });
        register_native_wallet_shutdown_hook();
    }
    Ok(())
}

/// Prepare a native transfer through the module and retain its exact quote so
/// the host can bind approval and signer selection to the same immutable facts.
pub(crate) async fn prepare_native_transfer(
    ctx: &DispatchCtx,
    config: &Config,
    owner: Option<tinywallet_bus::wire::QuoteOwner>,
    params: tinywallet_bus::wire::PrepareTransferParams,
) -> Result<tinywallet_bus::wire::PreparedTransaction, WalletCallError> {
    let mut active = native_wallet_service().lock().await;
    register_native_wallet_shutdown_hook();
    let accounts = super::wallet_host::wallet_accounts_snapshot(ctx)
        .await
        .map_err(WalletCallError::Failed)?;
    ensure_native_wallet(
        config,
        &mut active,
        config.workspace_dir.clone(),
        native_wallet_config_request(),
        accounts,
    )
    .await?;
    let service = active
        .as_mut()
        .ok_or_else(|| WalletCallError::Failed("native wallet service is unavailable".into()))?;
    let quote: tinywallet_bus::wire::PreparedTransaction = wallet::call(
        config,
        methods::NATIVE_WALLET_PREPARE_TRANSFER,
        tinywallet_bus::wire::NativeWalletPrepareTransferRequest {
            service: service.response.service.clone(),
            owner,
            params,
        },
    )
    .await?;
    let now = now_ms();
    service
        .prepared_quotes
        .retain(|_, prepared| prepared.expires_at_ms > now);
    service
        .prepared_quotes
        .insert(quote.quote_id.clone(), quote.clone());
    Ok(quote)
}

/// Execute one explicitly confirmed native quote. The host retrieves only the
/// matching account secret and delivers it over the attested confidential bus
/// call; the module owns quote validation, approval facts, signing and broadcast.
pub(crate) async fn execute_native_transfer(
    ctx: &DispatchCtx,
    config: &Config,
    owner: Option<tinywallet_bus::wire::QuoteOwner>,
    params: tinywallet_bus::wire::ExecutePreparedParams,
) -> Result<tinywallet_bus::wire::ExecutionResult, WalletCallError> {
    if !params.confirmed {
        return Err(WalletCallError::InvalidInput(
            "native wallet execution requires confirmed: true".into(),
        ));
    }

    let mut active = native_wallet_service().lock().await;
    register_native_wallet_shutdown_hook();
    let accounts = super::wallet_host::wallet_accounts_snapshot(ctx)
        .await
        .map_err(WalletCallError::Failed)?;
    ensure_native_wallet(
        config,
        &mut active,
        config.workspace_dir.clone(),
        native_wallet_config_request(),
        accounts,
    )
    .await?;
    let service = active
        .as_mut()
        .ok_or_else(|| WalletCallError::Failed("native wallet service is unavailable".into()))?;
    let quote = service
        .prepared_quotes
        .get(&params.quote_id)
        .filter(|quote| quote.expires_at_ms > now_ms())
        .cloned()
        .ok_or_else(|| {
            WalletCallError::InvalidInput("prepared wallet quote is unavailable".into())
        })?;
    let secret = signing_secret(ctx, config, quote.chain).await?;
    let mut request = tinywallet_bus::wire::NativeWalletExecuteRequest {
        service: service.response.service.clone(),
        owner,
        quote_id: params.quote_id.clone(),
        confirmed: params.confirmed,
        secret,
    };
    let result = super::wallet_host::with_confirmed_execution(
        super::wallet_host::ApprovalKind::Native,
        &params.quote_id,
        Some(quote),
        None,
        wallet::call_confidential(config, methods::NATIVE_WALLET_EXECUTE_PREPARED, &request),
    )
    .await;
    request.secret.mnemonic.zeroize();
    if result.is_ok() {
        service.prepared_quotes.remove(&params.quote_id);
    }
    result
}

async fn signing_secret(
    ctx: &DispatchCtx,
    config: &Config,
    chain: tinywallet_bus::wire::WalletChain,
) -> Result<tinywallet_bus::wire::SecretMaterial, WalletCallError> {
    let local_chain = match chain {
        tinywallet_bus::wire::WalletChain::Evm => crate::web3::wallet::WalletChain::Evm,
        tinywallet_bus::wire::WalletChain::Btc => crate::web3::wallet::WalletChain::Btc,
        tinywallet_bus::wire::WalletChain::Solana => crate::web3::wallet::WalletChain::Solana,
        tinywallet_bus::wire::WalletChain::Tron => crate::web3::wallet::WalletChain::Tron,
    };
    let stored = crate::web3::wallet::secret_material(ctx, local_chain)
        .await
        .map_err(WalletCallError::Failed)?;
    let mnemonic =
        crate::security::encryption::rpc::decrypt_secret(config, &stored.encrypted_mnemonic)
            .await
            .map_err(|error| WalletCallError::Failed(error.to_string()))?
            .value;
    Ok(tinywallet_bus::wire::SecretMaterial {
        mnemonic,
        derivation_path: stored.derivation_path,
        chain: chain.to_chain(),
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

fn service_requires_replacement(
    service: &NativeWalletService,
    workspace: &std::path::Path,
    configuration: &NativeWalletConfigRequest,
    accounts: &WalletAccountsResponse,
) -> bool {
    service.workspace != workspace
        || &service.configuration != configuration
        || &service.accounts != accounts
}

async fn close_native_wallet(
    config: &Config,
    service: &NativeWalletService,
) -> Result<(), WalletCallError> {
    wallet::call::<_, ()>(
        config,
        methods::NATIVE_WALLET_CLOSE,
        NativeWalletCloseRequest {
            service: service.response.service.clone(),
        },
    )
    .await?;
    super::wallet_host::clear_endpoint_references(&service.response.service.handle);
    Ok(())
}

/// Close the current per-user service on sign-out or process shutdown. The
/// mutex waits for any in-flight bus call to finish before releasing quote and
/// endpoint state, so a later user cannot inherit an old handle.
pub(crate) async fn close_native_wallet_service() -> Result<(), WalletCallError> {
    register_native_wallet_shutdown_hook();
    close_native_wallet_service_inner().await
}

async fn close_native_wallet_service_inner() -> Result<(), WalletCallError> {
    let mut active = native_wallet_service().lock().await;
    let config = active.as_ref().map(|service| service.config.clone());
    close_active_native_wallet_with(&mut active, move |request| async move {
        let Some(config) = config else {
            return Ok(());
        };
        wallet::call::<_, ()>(&config, methods::NATIVE_WALLET_CLOSE, request).await
    })
    .await
}

async fn close_active_native_wallet_with<F, Fut>(
    active: &mut Option<NativeWalletService>,
    close: F,
) -> Result<(), WalletCallError>
where
    F: FnOnce(NativeWalletCloseRequest) -> Fut,
    Fut: Future<Output = Result<(), WalletCallError>>,
{
    let handle = if let Some(service) = active.as_ref() {
        close(NativeWalletCloseRequest {
            service: service.response.service.clone(),
        })
        .await?;
        Some(service.response.service.handle.clone())
    } else {
        None
    };
    *active = None;
    if let Some(handle) = handle {
        super::wallet_host::clear_endpoint_references(&handle);
    }
    Ok(())
}

fn register_native_wallet_shutdown_hook() {
    if !begin_shutdown_hook_registration(&NATIVE_WALLET_SHUTDOWN_HOOK_REGISTERED) {
        return;
    }
    crate::core::shutdown::register(|| async {
        finish_shutdown_hook_registration(&NATIVE_WALLET_SHUTDOWN_HOOK_REGISTERED);
        if let Err(error) = close_native_wallet_service_inner().await {
            tracing::warn!(%error, "[modules:wallet] failed to close native wallet service at shutdown");
        }
    });
}

fn begin_shutdown_hook_registration(registered: &AtomicBool) -> bool {
    !registered.swap(true, Ordering::AcqRel)
}

fn finish_shutdown_hook_registration(registered: &AtomicBool) {
    registered.store(false, Ordering::Release);
}

pub(super) fn native_wallet_config_request() -> NativeWalletConfigRequest {
    let mut overridden_endpoints = Vec::new();
    for network in EvmNetwork::ALL {
        let key = RpcEndpointKey::Evm { network };
        if crate::web3::wallet::endpoints::override_for_endpoint(key).is_some() {
            overridden_endpoints.push(key);
        }
    }
    for key in [
        RpcEndpointKey::Btc,
        RpcEndpointKey::Solana,
        RpcEndpointKey::Tron,
    ] {
        if crate::web3::wallet::endpoints::override_for_endpoint(key).is_some() {
            overridden_endpoints.push(key);
        }
    }
    let solana_cluster = crate::web3::wallet::solana_cluster();
    NativeWalletConfigRequest {
        solana_cluster,
        overridden_endpoints,
    }
}

#[cfg(test)]
#[path = "wallet_service_tests.rs"]
mod tests;
