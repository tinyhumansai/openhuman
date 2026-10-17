//! User-scoped lifecycle for the module-owned x402 ledger.

use crate::core::runtime::DispatchCtx;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use zeroize::Zeroize;

use tinywallet_bus::names::methods;
use tinywallet_bus::wire::{
    X402CloseRequest, X402ConfigResponse, X402ConfigureRequest, X402DetailedSpendingSummary,
    X402ListPaymentsRequest, X402ListPaymentsResponse, X402PreparePaymentRequest,
    X402PreparePaymentResponse, X402RecordPaymentRequest, X402ServiceHandle,
    X402UpdateBudgetRequest,
};

use crate::config::Config;

use super::wallet::{self, WalletCallError};

/// Prepare and sign a 402 challenge inside TinyWallet. Both supported rails'
/// secret material is sent only through the attested confidential call; the
/// module parses the challenge and selects the matching chain.
pub(crate) async fn prepare_payment(
    ctx: &DispatchCtx,
    config: &Config,
    url: String,
    payment_required: String,
    owner: Option<tinywallet_bus::wire::QuoteOwner>,
) -> Result<(X402ServiceHandle, X402PreparePaymentResponse), WalletCallError> {
    let mut active = service().lock().await;
    ensure_service(ctx, config, &mut active).await?;
    let service_handle = active
        .as_ref()
        .ok_or_else(|| WalletCallError::Failed("x402 service is unavailable".into()))?
        .handle
        .clone();

    let evm = signing_secret(ctx, config, crate::web3::wallet::WalletChain::Evm).await?;
    let solana = signing_secret(ctx, config, crate::web3::wallet::WalletChain::Solana).await?;
    let mut request = X402PreparePaymentRequest {
        service: service_handle.clone(),
        url,
        payment_required,
        owner,
        secret: evm,
        additional_secrets: vec![solana],
    };
    let response = wallet::call_confidential(config, methods::X402_PREPARE_PAYMENT, &request).await;
    request.secret.mnemonic.zeroize();
    for secret in &mut request.additional_secrets {
        secret.mnemonic.zeroize();
    }
    Ok((service_handle, response?))
}

/// Record the retry result through the same module-owned payment ledger.
pub(crate) async fn record_payment(
    config: &Config,
    service: X402ServiceHandle,
    payment_id: String,
    response_status: u16,
    settlement_response: Option<String>,
) -> Result<(), WalletCallError> {
    wallet::call(
        config,
        methods::X402_RECORD_PAYMENT,
        X402RecordPaymentRequest {
            service,
            payment_id,
            response_status,
            settlement_response,
        },
    )
    .await
}

async fn signing_secret(
    ctx: &DispatchCtx,
    config: &Config,
    chain: crate::web3::wallet::WalletChain,
) -> Result<tinywallet_bus::wire::SecretMaterial, WalletCallError> {
    let stored = crate::web3::wallet::secret_material(ctx, chain)
        .await
        .map_err(WalletCallError::Failed)?;
    let phrase =
        crate::security::encryption::rpc::decrypt_secret(config, &stored.encrypted_mnemonic)
            .await
            .map_err(|error| WalletCallError::Failed(error.to_string()))?
            .value;
    Ok(tinywallet_bus::wire::SecretMaterial {
        mnemonic: phrase,
        derivation_path: stored.derivation_path,
        chain: chain.to_chain(),
    })
}

struct X402Service {
    workspace: PathBuf,
    accounts: tinywallet_bus::wire::WalletAccountsResponse,
    config: X402ConfigResponse,
    handle: X402ServiceHandle,
}

static X402_SERVICE: OnceLock<tokio::sync::Mutex<Option<X402Service>>> = OnceLock::new();
static X402_SHUTDOWN_HOOK_REGISTERED: AtomicBool = AtomicBool::new(false);

fn service() -> &'static tokio::sync::Mutex<Option<X402Service>> {
    X402_SERVICE.get_or_init(|| tokio::sync::Mutex::new(None))
}

async fn ensure_service(
    ctx: &DispatchCtx,
    config: &Config,
    active: &mut Option<X402Service>,
) -> Result<(), WalletCallError> {
    register_shutdown_hook();
    let workspace = config.workspace_dir.clone();
    let accounts = super::wallet_host::wallet_accounts_snapshot(ctx)
        .await
        .map_err(WalletCallError::Failed)?;
    let budget = budget_from(|name| std::env::var(name).ok());

    if active.as_ref().is_some_and(|current| {
        current.workspace != workspace
            || current.accounts != accounts
            || current.config.per_request_max_atomic != budget.0
            || current.config.daily_max_atomic != budget.1
            || current.config.monthly_max_atomic != budget.2
    }) {
        if let Some(current) = active.as_ref() {
            wallet::call::<_, ()>(
                config,
                methods::X402_CLOSE,
                X402CloseRequest {
                    service: current.handle.clone(),
                },
            )
            .await?;
        }
        *active = None;
    }

    if active.is_none() {
        // The module's ledger opens on its storage: bring the payments file of
        // an earlier install across first (once per workspace, best effort).
        if !super::import_legacy_x402(ctx, &workspace).await {
            // Opening the ledger without the earlier payments would restore the
            // spending allowance; stay inactive and retry on the next call.
            return Err(WalletCallError::Unavailable(
                "x402 legacy payment import failed; the service is not opened".to_string(),
            ));
        }
        let service_config = X402ConfigResponse {
            workspace_dir: workspace.display().to_string(),
            session_id: format!("x402-{}", uuid::Uuid::new_v4()),
            per_request_max_atomic: budget.0,
            daily_max_atomic: budget.1,
            monthly_max_atomic: budget.2,
        };
        let handle: X402ServiceHandle = wallet::call(
            config,
            methods::X402_CONFIGURE,
            X402ConfigureRequest {
                config: service_config.clone(),
            },
        )
        .await?;
        *active = Some(X402Service {
            workspace,
            accounts,
            config: service_config,
            handle,
        });
    }
    Ok(())
}

/// Close the active module ledger on sign-out after joining any current
/// operation. If close fails, the handle stays cached so a later lifecycle
/// attempt can retry instead of silently abandoning pending reservations.
pub(crate) async fn close_service(config: &Config) -> Result<(), WalletCallError> {
    register_shutdown_hook();
    let mut active = service().lock().await;
    let Some(current) = active.as_ref() else {
        return Ok(());
    };
    wallet::call::<_, ()>(
        config,
        methods::X402_CLOSE,
        X402CloseRequest {
            service: current.handle.clone(),
        },
    )
    .await?;
    *active = None;
    Ok(())
}

fn register_shutdown_hook() {
    if X402_SHUTDOWN_HOOK_REGISTERED.swap(true, Ordering::AcqRel) {
        return;
    }
    crate::core::shutdown::register(|| async {
        X402_SHUTDOWN_HOOK_REGISTERED.store(false, Ordering::Release);
        let mut active = service().lock().await;
        let Some(current) = active.as_ref() else {
            return;
        };
        // A shutdown hook carries no context of its own.
        let Ok(config) = DispatchCtx::host_default().load_config().await else {
            tracing::warn!(
                "[modules:x402] could not load configuration to close service at shutdown"
            );
            return;
        };
        match wallet::call::<_, ()>(
            &config,
            methods::X402_CLOSE,
            X402CloseRequest {
                service: current.handle.clone(),
            },
        )
        .await
        {
            Ok(()) => *active = None,
            Err(error) => {
                tracing::warn!(%error, "[modules:x402] failed to close service at shutdown")
            }
        }
    });
}

async fn with_service<R, F, Fut>(
    ctx: &DispatchCtx,
    config: &Config,
    operation: F,
) -> Result<R, WalletCallError>
where
    F: FnOnce(X402ServiceHandle) -> Fut,
    Fut: std::future::Future<Output = Result<R, WalletCallError>>,
{
    let mut active = service().lock().await;
    ensure_service(ctx, config, &mut active).await?;
    let handle = active
        .as_ref()
        .ok_or_else(|| WalletCallError::Failed("x402 service is unavailable".into()))?
        .handle
        .clone();
    operation(handle).await
}

/// Read the module-owned spending totals and budgets.
pub(crate) async fn summary(
    ctx: &DispatchCtx,
    config: &Config,
) -> Result<X402DetailedSpendingSummary, WalletCallError> {
    let config = config.clone();
    let operation_config = config.clone();
    with_service(ctx, &config, move |service| async move {
        wallet::call(
            &operation_config,
            methods::X402_SPENDING_SUMMARY_DETAILED,
            tinywallet_bus::wire::X402SummaryRequest { service },
        )
        .await
    })
    .await
}

/// Read recent module-owned payment records.
pub(crate) async fn list_payments(
    ctx: &DispatchCtx,
    config: &Config,
    limit: u64,
) -> Result<X402ListPaymentsResponse, WalletCallError> {
    let config = config.clone();
    let operation_config = config.clone();
    with_service(ctx, &config, move |service| async move {
        wallet::call(
            &operation_config,
            methods::X402_LIST_PAYMENTS,
            X402ListPaymentsRequest { service, limit },
        )
        .await
    })
    .await
}

/// Update live module-owned spending limits without replacing the ledger.
pub(crate) async fn update_budget(
    ctx: &DispatchCtx,
    config: &Config,
    per_request_max_atomic: Option<u64>,
    daily_max_atomic: Option<u64>,
    monthly_max_atomic: Option<u64>,
) -> Result<tinywallet_bus::wire::X402SpendingSummary, WalletCallError> {
    let mut active = service().lock().await;
    ensure_service(ctx, config, &mut active).await?;
    let current = active
        .as_mut()
        .ok_or_else(|| WalletCallError::Failed("x402 service is unavailable".into()))?;
    let result: tinywallet_bus::wire::X402SpendingSummary = wallet::call(
        config,
        methods::X402_UPDATE_BUDGET,
        X402UpdateBudgetRequest {
            service: current.handle.clone(),
            per_request_max_atomic,
            daily_max_atomic,
            monthly_max_atomic,
        },
    )
    .await?;
    current.config.per_request_max_atomic = result.per_request_max_atomic;
    current.config.daily_max_atomic = result.daily_max_atomic;
    current.config.monthly_max_atomic = result.monthly_max_atomic;
    Ok(result)
}

fn budget_from(get: impl Fn(&str) -> Option<String>) -> (u64, u64, u64) {
    let mut per_request_max_atomic = 1_000_000;
    let mut daily_max_atomic = 10_000_000;
    let mut monthly_max_atomic = 100_000_000;
    if let Some(value) = get("OPENHUMAN_X402_PER_REQUEST_MAX").and_then(|value| value.parse().ok())
    {
        per_request_max_atomic = value;
    }
    if let Some(value) = get("OPENHUMAN_X402_DAILY_MAX").and_then(|value| value.parse().ok()) {
        daily_max_atomic = value;
    }
    if let Some(value) = get("OPENHUMAN_X402_MONTHLY_MAX").and_then(|value| value.parse().ok()) {
        monthly_max_atomic = value;
    }
    (per_request_max_atomic, daily_max_atomic, monthly_max_atomic)
}

#[cfg(test)]
#[path = "x402_service_tests.rs"]
mod tests;
