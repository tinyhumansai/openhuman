use crate::api::models::socket::SocketState;

use super::SocketManager;

async fn connect_static_using(
    manager: &SocketManager,
    url: &str,
    token: &str,
    clear_workflows: impl FnOnce(),
) -> Result<SocketState, String> {
    let _rebind = manager.lock_identity_rebind().await;
    manager.disconnect().await?;
    clear_workflows();
    manager.connect(url, token).await?;
    Ok(manager.get_state())
}

pub async fn connect_static(
    manager: &SocketManager,
    url: &str,
    token: &str,
) -> Result<SocketState, String> {
    log::info!("[socket:rpc] connect — disabling identity-bound workflow plane");
    connect_static_using(
        manager,
        url,
        token,
        super::medulla::workflows::clear_workflow_bridge,
    )
    .await
}

pub async fn disconnect(manager: &SocketManager) -> Result<SocketState, String> {
    log::info!("[socket:rpc] disconnect");
    let _rebind = manager.lock_identity_rebind().await;
    manager.disconnect().await?;
    Ok(manager.get_state())
}

/// Shared body of [`connect_with_session`], with the credential lookup and the
/// workflow-bridge install lifted out so the transaction can be driven directly
/// from tests.
///
/// The `manager.is_live_for` short-circuit is the fix for #6181: the core's
/// bootstrap auto-connect and this RPC both fire on a cold start, roughly two
/// seconds apart, and each used to tear the other's freshly handshaked socket
/// down. When the live connection already serves this exact url+token there is
/// no second EIO session to open.
///
/// The socket is reusable; the workflow bridge is not. It is pinned to a
/// `Config` that a workspace switch invalidates, and `connect_static` clears it
/// outright while leaving a matching connection identity behind — so skipping
/// `install_bridge` on the reuse path would leave the workflow plane pointing at
/// a stale workspace, or disabled entirely. `set_workflow_bridge` re-advertises
/// over an already-`ready` socket by design, so the install runs on both paths
/// and only the handshake is skipped.
async fn connect_with_session_using(
    manager: &SocketManager,
    url: &str,
    token: &str,
    provider: super::token_provider::TokenProvider,
    install_bridge: impl FnOnce(),
) -> Result<SocketState, String> {
    let _rebind = manager.lock_identity_rebind().await;
    if manager.is_live_for(url, token) {
        install_bridge();
        log::info!(
            "[socket:rpc] connect_with_session — {url} already connected with this session; reusing the live socket"
        );
        return Ok(manager.get_state());
    }
    manager.disconnect().await?;
    install_bridge();
    manager.connect_with_provider(url, provider).await?;
    Ok(manager.get_state())
}

pub async fn connect_with_session(manager: &SocketManager) -> Result<SocketState, String> {
    log::info!("[socket:rpc] connect_with_session — resolving credentials");
    let config =
        std::sync::Arc::new(crate::openhuman::config::rpc::load_config_with_timeout().await?);
    let api_url = crate::api::config::effective_backend_api_url(&config.api_url);
    let token = crate::api::jwt::get_session_token(&config)
        .map_err(|e| format!("failed to read session token: {e}"))?
        .ok_or("no session token stored — user must log in first")?;

    let provider =
        super::token_provider::token_provider_from_config(std::sync::Arc::clone(&config));
    connect_with_session_using(manager, &api_url, &token, provider, move || {
        #[cfg(feature = "flows")]
        if crate::core::runtime::context::CoreContext::current()
            .is_some_and(|context| context.domains().flows)
        {
            crate::openhuman::flows::medulla_bridge::install(config);
        }
    })
    .await
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
