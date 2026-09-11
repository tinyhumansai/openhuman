//! The Tauri surface for gateways.
//!
//! Thin by design: every command resolves arguments, delegates to [`store`] or
//! [`registry`], and maps the result. The rules live there.

use super::registry;
use super::store;
use super::types::{Gateway, GatewayStatus, GatewaySummary};

/// Every configured gateway, the built-in desktop one first.
///
/// Returns a credential-free [`GatewaySummary`], not the full [`Gateway`]: a
/// `Remote` bearer and a `Box`/SSH destination or identity path belong to the
/// shell store, and the renderer only needs the id, label and kind badge.
#[tauri::command]
pub(crate) fn gateway_list() -> Vec<GatewaySummary> {
    let gateways = store::list();
    log::debug!("[gateway][cmd] list -> {} gateway(s)", gateways.len());
    gateways.iter().map(GatewaySummary::from).collect()
}

/// Add or replace a gateway.
///
/// Saving does not activate: a user editing an SSH destination should not have
/// their session torn down and re-provisioned on every keystroke that lands in
/// a save.
#[tauri::command]
pub(crate) fn gateway_save(gateway: Gateway) -> Result<(), String> {
    log::info!(
        "[gateway][cmd] save id={} kind={}",
        gateway.id,
        gateway.spec.kind()
    );
    store::save(gateway)
}

/// Forget a gateway.
///
/// Removing the active one does not switch away from it — the running session
/// keeps working, and the next launch falls back to the desktop core. Tearing a
/// working connection down as a side effect of tidying a list would be a
/// surprise.
#[tauri::command]
pub(crate) fn gateway_delete(id: String) -> Result<(), String> {
    log::info!("[gateway][cmd] delete id={id}");
    store::delete(&id)
}

/// Make a gateway the one RPC goes to.
///
/// Returns an empty acknowledgment, not the [`ActiveGateway`]: the endpoint and
/// bearer stay in the shell registry, where `core_rpc_url` / `core_rpc_token`
/// answer from. Handing the bearer to the renderer would let an XSS exfiltrate
/// a credential the renderer never needs — it only needs to know the switch
/// happened.
#[tauri::command]
pub(crate) async fn gateway_activate(
    id: String,
    desktop: tauri::State<'_, crate::core_process::CoreProcessHandle>,
) -> Result<(), String> {
    let gateway = store::get(&id).ok_or_else(|| format!("no gateway named {id}"))?;
    registry::activate(&gateway, desktop.inner()).await?;
    Ok(())
}

/// Which gateway is active.
#[tauri::command]
pub(crate) async fn gateway_active() -> String {
    registry::active_id().await
}

/// What a gateway is doing right now.
#[tauri::command]
pub(crate) async fn gateway_status(id: String) -> GatewayStatus {
    registry::status_of(&id).await
}
