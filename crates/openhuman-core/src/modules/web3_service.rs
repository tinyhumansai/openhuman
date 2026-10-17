//! User-scoped lifecycle and host transport for TinyWallet Web3 quotes.

use crate::core::runtime::DispatchCtx;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use serde::Deserialize;
use tinywallet_bus::names::methods;
use tinywallet_bus::wire::{
    QuoteOwner, WalletChain, Web3CloseRequest, Web3ConfigureRequest, Web3ConfigureResponse,
    Web3ExecuteRequest, Web3QuoteRequest, Web3QuoteResponse,
};

use crate::config::Config;

use super::{wallet, wallet_host};

struct Web3Service {
    workspace: PathBuf,
    accounts: tinywallet_bus::wire::WalletAccountsResponse,
    configuration: Web3ConfigureRequest,
    response: Web3ConfigureResponse,
    quotes: HashMap<String, Web3QuoteResponse>,
}

static WEB3_SERVICE: OnceLock<tokio::sync::Mutex<Option<Web3Service>>> = OnceLock::new();
static WEB3_SHUTDOWN_HOOK_REGISTERED: AtomicBool = AtomicBool::new(false);

fn service() -> &'static tokio::sync::Mutex<Option<Web3Service>> {
    WEB3_SERVICE.get_or_init(|| tokio::sync::Mutex::new(None))
}

async fn ensure_service(
    ctx: &DispatchCtx,
    config: &Config,
    active: &mut Option<Web3Service>,
) -> Result<(), wallet::WalletCallError> {
    register_shutdown_hook();
    let accounts = wallet_host::wallet_accounts_snapshot(ctx)
        .await
        .map_err(wallet::WalletCallError::Failed)?;
    let workspace = config.workspace_dir.clone();
    let native_configuration = super::wallet_service::native_wallet_config_request();
    let configuration = Web3ConfigureRequest {
        solana_cluster: native_configuration.solana_cluster,
        overridden_endpoints: native_configuration.overridden_endpoints,
    };
    if active.as_ref().is_some_and(|current| {
        current.workspace != workspace
            || current.accounts != accounts
            || current.configuration != configuration
    }) {
        if let Some(current) = active.as_ref() {
            wallet::call::<_, ()>(
                config,
                methods::WEB3_CLOSE,
                Web3CloseRequest {
                    service: current.response.service.clone(),
                },
            )
            .await?;
            wallet_host::clear_endpoint_references(&current.response.service.handle);
        }
        *active = None;
    }
    if active.is_none() {
        let response: Web3ConfigureResponse = wallet::call(
            config,
            methods::WEB3_CONFIGURE_WITH_ENDPOINTS,
            configuration.clone(),
        )
        .await?;
        if let Err(error) = wallet_host::bind_endpoint_references(
            &response.service.handle,
            &response.endpoint_references,
        ) {
            let _ = wallet::call::<_, ()>(
                config,
                methods::WEB3_CLOSE,
                Web3CloseRequest {
                    service: response.service.clone(),
                },
            )
            .await;
            return Err(wallet::WalletCallError::Failed(error));
        }
        *active = Some(Web3Service {
            workspace,
            accounts,
            configuration,
            response,
            quotes: HashMap::new(),
        });
    }
    Ok(())
}

/// The quote owner of a turn whose chat routing is `chat`: quotes are bound to
/// the thread and client that prepared them, so another chat cannot execute
/// them. `None` outside a chat turn.
pub(crate) fn owner_of(
    chat: Option<&crate::security::approval::ApprovalChatContext>,
) -> Option<QuoteOwner> {
    chat.map(|context| QuoteOwner {
        thread_id: context.thread_id.clone(),
        client_id: context.client_id.clone(),
    })
}

/// Close the active module service on sign-out. The mutex joins any in-flight
/// quote, route, or execution call before its owner-scoped handles are cleared.
pub(crate) async fn close_service(config: &Config) -> Result<(), wallet::WalletCallError> {
    register_shutdown_hook();
    let mut active = service().lock().await;
    let Some(current) = active.as_ref() else {
        return Ok(());
    };
    wallet::call::<_, ()>(
        config,
        methods::WEB3_CLOSE,
        Web3CloseRequest {
            service: current.response.service.clone(),
        },
    )
    .await?;
    wallet_host::clear_endpoint_references(&current.response.service.handle);
    *active = None;
    Ok(())
}

fn register_shutdown_hook() {
    if WEB3_SHUTDOWN_HOOK_REGISTERED.swap(true, Ordering::AcqRel) {
        return;
    }
    crate::core::shutdown::register(|| async {
        WEB3_SHUTDOWN_HOOK_REGISTERED.store(false, Ordering::Release);
        let mut active = service().lock().await;
        let Some(current) = active.as_ref() else {
            return;
        };
        // A shutdown hook carries no context of its own.
        let config = DispatchCtx::host_default().load_config().await;
        let Ok(config) = config else {
            tracing::warn!(
                "[modules:web3] could not load configuration to close service at shutdown"
            );
            return;
        };
        match wallet::call::<_, ()>(
            &config,
            methods::WEB3_CLOSE,
            Web3CloseRequest {
                service: current.response.service.clone(),
            },
        )
        .await
        {
            Ok(()) => {
                wallet_host::clear_endpoint_references(&current.response.service.handle);
                *active = None;
            }
            Err(error) => {
                tracing::warn!(%error, "[modules:web3] failed to close service at shutdown")
            }
        }
    });
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExecuteParams {
    pub(crate) quote_id: String,
    pub(crate) confirmed: bool,
}

/// Prepare one implementation-owned Web3 quote.
pub(crate) async fn quote(
    ctx: &DispatchCtx,
    config: &Config,
    owner: Option<QuoteOwner>,
    member: &'static str,
    params: serde_json::Value,
) -> Result<Web3QuoteResponse, wallet::WalletCallError> {
    let mut active = service().lock().await;
    ensure_service(ctx, config, &mut active).await?;
    let service_handle = active
        .as_ref()
        .expect("ensure_service installs a handle")
        .response
        .service
        .clone();
    let response: Web3QuoteResponse = wallet::call(
        config,
        member,
        Web3QuoteRequest {
            service: service_handle,
            owner,
            params,
        },
    )
    .await?;
    if let Some(current) = active.as_mut() {
        current
            .quotes
            .insert(response.quote_id.clone(), response.clone());
    }
    Ok(response)
}

/// Call the module-owned authenticated-backend route operation.
pub(crate) async fn routes(
    ctx: &DispatchCtx,
    config: &Config,
) -> Result<serde_json::Value, wallet::WalletCallError> {
    let mut active = service().lock().await;
    ensure_service(ctx, config, &mut active).await?;
    wallet::call(config, methods::WEB3_ROUTES, ()).await
}

/// Execute one explicitly confirmed quote using its module-declared signer chain.
pub(crate) async fn execute(
    ctx: &DispatchCtx,
    config: &Config,
    owner: Option<QuoteOwner>,
    quote_id: String,
    confirmed: bool,
) -> Result<serde_json::Value, wallet::WalletCallError> {
    if !confirmed {
        return Err(wallet::WalletCallError::InvalidInput(
            "Web3 execution requires confirmed: true".into(),
        ));
    }
    let mut active = service().lock().await;
    ensure_service(ctx, config, &mut active).await?;
    let service_handle = active
        .as_ref()
        .expect("ensure_service installs a handle")
        .response
        .service
        .clone();
    let quote = active
        .as_ref()
        .and_then(|active| active.quotes.get(&quote_id))
        .cloned()
        .ok_or_else(|| {
            wallet::WalletCallError::InvalidInput("prepared Web3 quote is unavailable".into())
        })?;
    let chain = match quote.signing_chain {
        WalletChain::Evm => crate::web3::wallet::WalletChain::Evm,
        WalletChain::Btc => crate::web3::wallet::WalletChain::Btc,
        WalletChain::Solana => crate::web3::wallet::WalletChain::Solana,
        WalletChain::Tron => crate::web3::wallet::WalletChain::Tron,
    };
    let stored = crate::web3::wallet::secret_material(ctx, chain)
        .await
        .map_err(|error| wallet::WalletCallError::Failed(error.to_string()))?;
    let mnemonic =
        crate::security::encryption::rpc::decrypt_secret(config, &stored.encrypted_mnemonic)
            .await
            .map_err(|error| wallet::WalletCallError::Failed(error.to_string()))?
            .value;
    let mut request = Web3ExecuteRequest {
        service: service_handle,
        owner,
        quote_id: quote_id.clone(),
        confirmed,
        secret: tinywallet_bus::wire::SecretMaterial {
            mnemonic,
            derivation_path: stored.derivation_path,
            chain: quote.signing_chain.to_chain(),
        },
    };
    let result = wallet_host::with_confirmed_execution(
        wallet_host::ApprovalKind::Web3,
        &quote_id,
        None,
        Some(quote),
        wallet::call_confidential(config, methods::WEB3_EXECUTE, &request),
    )
    .await;
    zeroize::Zeroize::zeroize(&mut request.secret.mnemonic);
    if result.is_ok() {
        active
            .as_mut()
            .map(|active| active.quotes.remove(&quote_id));
    }
    result
}
