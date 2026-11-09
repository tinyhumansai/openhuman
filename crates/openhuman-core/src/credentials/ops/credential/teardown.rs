use super::*;
use crate::config::user_scope::{deactivate_user_scope, rebind_after_credential_change};
use crate::core::observability::sentry_scope;
use crate::core::Outcome;
use crate::credentials::{identity, AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};
use std::future::Future;

/// The session teardown: gate closed first so no worker fires at a backend
/// about to be invalidated, then profile removal, socket / workflow-bridge
/// teardown, user-dir deactivation, service stop, rebind to the signed-out
/// workspace, and Sentry / identity clear. Callers hold
/// [`CREDENTIAL_MUTATION_LOCK`].
pub(crate) async fn clear_session_credential(
    ctx: &DispatchCtx,
    config: &Config,
) -> Result<Outcome<bool>, String> {
    crate::platform::llm_gate::set_signed_out(true);
    after_device_pairing_lock(
        || crate::security::devices::bus::lock_device_pairing_scope_for_logout(ctx, config),
        || clear_session_credential_after_pairing_lock(ctx, config),
    )
    .await
}

async fn clear_session_credential_after_pairing_lock(
    ctx: &DispatchCtx,
    config: &Config,
) -> Result<Outcome<bool>, String> {
    let mut logs = Vec::new();
    #[cfg(feature = "web3")]
    if let Err(error) = crate::modules::web3_service::close_service(config).await {
        tracing::warn!(%error, "{LOG_PREFIX} failed to close Web3 service during sign-out");
        logs.push("Web3 service cleanup will retry on replacement or shutdown".into());
    }
    #[cfg(feature = "web3")]
    if let Err(error) = crate::modules::x402_service::close_service(config).await {
        tracing::warn!(%error, "{LOG_PREFIX} failed to close x402 service during sign-out");
        logs.push("x402 service cleanup will retry on replacement or shutdown".into());
    }
    #[cfg(feature = "web3")]
    if let Err(error) = crate::modules::wallet_service::close_native_wallet_service().await {
        tracing::warn!(%error, "{LOG_PREFIX} failed to close native wallet service during sign-out");
        logs.push("native wallet service cleanup will retry on replacement or shutdown".into());
    }
    identity::clear_current_user();
    // Local/CLI model authentication can outlive an OpenHuman account. The old
    // account's listeners and workspace-bound channel state cannot.
    #[cfg(feature = "channels")]
    crate::channels::session::invalidate_channel_session();

    let removed = AuthService::from_config(config)
        .remove_profile(APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME)
        .await
        .map_err(|e| e.to_string())?;
    if removed {
        crate::integrations::composio::invalidate_connected_integrations_cache();
    }

    // The core process stays alive on sign-out. Tear down its authenticated
    // Socket.IO transport and the user-pinned workflow bridge so neither can
    // keep serving the signed-out account until a later reconnect.
    if let Some(manager) = crate::platform::socket::global_socket_manager() {
        if let Err(error) = manager.disconnect().await {
            tracing::warn!(%error, "{LOG_PREFIX} failed to disconnect backend socket on sign-out");
        }
    }

    deactivate_user_scope()?;
    stop_credential_gated_services(ctx, config).await;

    // Every process-global store must follow the now-active pre-login
    // workspace, or a signed-out caller keeps reading the previous account's
    // context until the process restarts.
    match ctx.load_config().await {
        Ok(signed_out_config) => {
            logs.extend(rebind_after_credential_change(
                &signed_out_config,
                "credential cleared",
            )?);
            logs.push(format!(
                "process globals rebound to signed-out workspace {}",
                signed_out_config.workspace_dir.display()
            ));
        }
        Err(error) => return Err(format!("failed to resolve signed-out workspace: {error}")),
    }

    sentry_scope::clear();
    logs.push("session cleared".to_string());
    Ok(Outcome::new(removed, logs))
}

async fn after_device_pairing_lock<Lock, LockFuture, Continue, ContinueFuture, ResultValue>(
    lock_pairings: Lock,
    continue_teardown: Continue,
) -> ResultValue
where
    Lock: FnOnce() -> LockFuture,
    LockFuture: Future<Output = ()>,
    Continue: FnOnce() -> ContinueFuture,
    ContinueFuture: Future<Output = ResultValue>,
{
    lock_pairings().await;
    continue_teardown().await
}

/// A process-wide credential is a single-user concept: installing one
/// activates a user directory and rebinds process globals. A SaaS process
/// keeps one credential per profile instead (`profiles.set_credential`).
pub(crate) fn refuse_process_credential_in_saas(operation: &str) -> Result<(), String> {
    process_credential_refusal(crate::core::runtime::is_saas(), operation)
}

pub(crate) fn process_credential_refusal(saas: bool, operation: &str) -> Result<(), String> {
    if saas {
        tracing::warn!(
            domain = "credentials",
            operation,
            "{LOG_PREFIX} refused: process-wide credentials are not used in SaaS mode"
        );
        return Err(format!(
            "{operation} is not available in SaaS mode; the gateway installs each user's \
             credential through profiles.set_credential"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "teardown_tests.rs"]
mod tests;

/// Tell credential-derived caches (the search module's managed routes) to
/// refresh. Carries only the kind, never the credential.
pub(crate) fn publish_credential_changed(ctx: &DispatchCtx, kind: &str) {
    tracing::debug!(kind, "{LOG_PREFIX} publishing CredentialChanged");
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::CredentialChanged {
        kind: kind.to_string(),
        tenant: ctx.event_tenant(),
    });
}
