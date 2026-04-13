//! RPC-facing operations for the Composio domain.
//!
//! Each `composio_*` function wraps a [`ComposioClient`] call, translates
//! errors to strings, and returns an [`RpcOutcome`] so the controller
//! schemas can log a user-visible line. The handlers in [`super::schemas`]
//! call into these.
//!
//! These ops are also callable directly from other domains (e.g. the
//! agent harness) when they need composio data at runtime.

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

/// Result alias used by every `composio_*` op in this module.
///
/// We deliberately return a plain `String` error instead of
/// `anyhow::Error` — the controller layer in `schemas.rs` forwards
/// these straight into the RPC envelope, and `String` keeps the shape
/// obvious at a glance.
type OpResult<T> = std::result::Result<T, String>;

use std::sync::Arc;

use super::client::{build_composio_client, ComposioClient};
use super::providers::{
    get_provider, ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason,
};
use super::types::{
    ComposioAuthorizeResponse, ComposioConnectionsResponse, ComposioDeleteResponse,
    ComposioExecuteResponse, ComposioToolkitsResponse, ComposioToolsResponse,
    ComposioTriggerHistoryResult,
};

/// Resolve a [`ComposioClient`] from the root config, or return an
/// error string that the caller can surface over RPC.
///
/// Composio is always enabled — it is proxied through our backend and
/// has no client-side toggle or API key. The only reason this fails is
/// that no app-session JWT has been stored yet (i.e. the user hasn't
/// completed sign-in / `auth_store_session`).
fn resolve_client(config: &Config) -> OpResult<ComposioClient> {
    build_composio_client(config).ok_or_else(|| {
        "composio unavailable: no backend session token. Sign in first \
         (auth_store_session)."
            .to_string()
    })
}

// ── Toolkits ────────────────────────────────────────────────────────

pub async fn composio_list_toolkits(
    config: &Config,
) -> OpResult<RpcOutcome<ComposioToolkitsResponse>> {
    tracing::debug!("[composio] rpc list_toolkits");
    let client = resolve_client(config)?;
    let resp = client
        .list_toolkits()
        .await
        .map_err(|e| format!("[composio] list_toolkits failed: {e:#}"))?;
    let count = resp.toolkits.len();
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: {count} toolkit(s) enabled")],
    ))
}

// ── Connections ─────────────────────────────────────────────────────

pub async fn composio_list_connections(
    config: &Config,
) -> OpResult<RpcOutcome<ComposioConnectionsResponse>> {
    tracing::debug!("[composio] rpc list_connections");
    let client = resolve_client(config)?;
    let resp = client
        .list_connections()
        .await
        .map_err(|e| format!("[composio] list_connections failed: {e:#}"))?;
    let active = resp
        .connections
        .iter()
        .filter(|c| matches!(c.status.as_str(), "ACTIVE" | "CONNECTED"))
        .count();
    let total = resp.connections.len();
    Ok(RpcOutcome::new(
        resp,
        vec![format!(
            "composio: {total} connection(s) listed ({active} active)"
        )],
    ))
}

pub async fn composio_authorize(
    config: &Config,
    toolkit: &str,
) -> OpResult<RpcOutcome<ComposioAuthorizeResponse>> {
    tracing::debug!(toolkit = %toolkit, "[composio] rpc authorize");
    let client = resolve_client(config)?;
    let resp = client
        .authorize(toolkit)
        .await
        .map_err(|e| format!("[composio] authorize failed: {e:#}"))?;

    // Publish an event so any interested subscribers (e.g. UI refreshers,
    // analytics) can react to the new connection handoff.
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::ComposioConnectionCreated {
            toolkit: toolkit.to_string(),
            connection_id: resp.connection_id.clone(),
            connect_url: resp.connect_url.clone(),
        },
    );

    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: authorize flow started for {toolkit}")],
    ))
}

pub async fn composio_delete_connection(
    config: &Config,
    connection_id: &str,
) -> OpResult<RpcOutcome<ComposioDeleteResponse>> {
    tracing::debug!(connection_id = %connection_id, "[composio] rpc delete_connection");
    let client = resolve_client(config)?;
    let resp = client
        .delete_connection(connection_id)
        .await
        .map_err(|e| format!("[composio] delete_connection failed: {e:#}"))?;
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: connection {connection_id} deleted")],
    ))
}

// ── Tools ───────────────────────────────────────────────────────────

pub async fn composio_list_tools(
    config: &Config,
    toolkits: Option<Vec<String>>,
) -> OpResult<RpcOutcome<ComposioToolsResponse>> {
    tracing::debug!(?toolkits, "[composio] rpc list_tools");
    let client = resolve_client(config)?;
    let resp = client
        .list_tools(toolkits.as_deref())
        .await
        .map_err(|e| format!("[composio] list_tools failed: {e:#}"))?;
    let count = resp.tools.len();
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: {count} tool(s) listed")],
    ))
}

// ── Execute ─────────────────────────────────────────────────────────

pub async fn composio_execute(
    config: &Config,
    tool: &str,
    arguments: Option<serde_json::Value>,
) -> OpResult<RpcOutcome<ComposioExecuteResponse>> {
    tracing::debug!(tool = %tool, "[composio] rpc execute");
    let client = resolve_client(config)?;
    let started = std::time::Instant::now();
    let result = client.execute_tool(tool, arguments.clone()).await;
    let elapsed_ms = started.elapsed().as_millis() as u64;

    match result {
        Ok(resp) => {
            crate::core::event_bus::publish_global(
                crate::core::event_bus::DomainEvent::ComposioActionExecuted {
                    tool: tool.to_string(),
                    success: resp.successful,
                    error: resp.error.clone(),
                    cost_usd: resp.cost_usd,
                    elapsed_ms,
                },
            );
            Ok(RpcOutcome::new(
                resp,
                vec![format!("composio: executed {tool} ({elapsed_ms}ms)")],
            ))
        }
        Err(e) => {
            crate::core::event_bus::publish_global(
                crate::core::event_bus::DomainEvent::ComposioActionExecuted {
                    tool: tool.to_string(),
                    success: false,
                    error: Some(e.to_string()),
                    cost_usd: 0.0,
                    elapsed_ms,
                },
            );
            Err(format!("[composio] execute failed: {e:#}"))
        }
    }
}

// ── Trigger history ────────────────────────────────────────────────

pub async fn composio_list_trigger_history(
    config: &Config,
    limit: Option<usize>,
) -> OpResult<RpcOutcome<ComposioTriggerHistoryResult>> {
    let requested_limit = limit.unwrap_or(100).clamp(1, 500);
    let workspace_label = config
        .workspace_dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("<workspace>");
    tracing::debug!(
        limit = requested_limit,
        workspace = workspace_label,
        "[composio] rpc list_trigger_history"
    );

    let store = super::trigger_history::global().ok_or_else(|| {
        "[composio] trigger history unavailable: archive store is not initialized".to_string()
    })?;

    let history = store
        .list_recent(requested_limit)
        .map_err(|error| format!("[composio] list_trigger_history failed: {error}"))?;
    let count = history.entries.len();

    Ok(RpcOutcome::new(
        history,
        vec![format!(
            "composio: {count} trigger history entrie(s) loaded (archive present)"
        )],
    ))
}

// ── Provider-backed ops ─────────────────────────────────────────────
//
// `composio_get_user_profile` and `composio_sync` route through the
// per-toolkit `ComposioProvider` registry instead of executing a
// single Composio action directly. The caller passes a `connection_id`,
// the op resolves the connection's toolkit slug from the backend, looks
// up the provider, and dispatches to it.
//
// These exist because individual toolkits need to do *several*
// `composio.execute` calls + bespoke result reshaping to produce a
// usable user profile or sync snapshot — wrapping that in a single
// RPC method keeps the UI/agent surface tiny and consistent across
// toolkits.

/// Look up the toolkit slug for an existing connection. Returns an
/// error string if the connection is unknown to the backend.
async fn resolve_toolkit_for_connection(
    client: &ComposioClient,
    connection_id: &str,
) -> OpResult<String> {
    tracing::debug!(connection_id = %connection_id, "[composio] resolve_toolkit_for_connection");
    let resp = client
        .list_connections()
        .await
        .map_err(|e| format!("[composio] list_connections failed: {e:#}"))?;
    let conn = resp
        .connections
        .into_iter()
        .find(|c| c.id == connection_id)
        .ok_or_else(|| format!("[composio] no connection with id '{connection_id}'"))?;
    Ok(conn.toolkit)
}

/// `openhuman.composio_get_user_profile` — fetch a normalized user
/// profile for a connected account by dispatching to the toolkit's
/// registered [`super::providers::ComposioProvider`].
pub async fn composio_get_user_profile(
    config: &Config,
    connection_id: &str,
) -> OpResult<RpcOutcome<ProviderUserProfile>> {
    tracing::debug!(connection_id = %connection_id, "[composio] rpc get_user_profile");
    let client = resolve_client(config)?;
    let toolkit = resolve_toolkit_for_connection(&client, connection_id).await?;

    let provider = get_provider(&toolkit).ok_or_else(|| {
        format!("[composio] no native provider registered for toolkit '{toolkit}'")
    })?;

    let ctx = ProviderContext {
        config: Arc::new(config.clone()),
        client,
        toolkit: toolkit.clone(),
        connection_id: Some(connection_id.to_string()),
    };

    let profile = provider
        .fetch_user_profile(&ctx)
        .await
        .map_err(|e| format!("[composio] get_user_profile({toolkit}) failed: {e}"))?;

    // Side-effect: persist profile fields into the local user_profile
    // facet table so any RPC call also refreshes the local store.
    let facets = super::providers::profile::persist_provider_profile(&profile);
    tracing::debug!(
        toolkit = %toolkit,
        facets_written = facets,
        "[composio] profile facets persisted from get_user_profile"
    );

    Ok(RpcOutcome::new(
        profile,
        vec![format!(
            "composio: fetched {toolkit} profile for connection {connection_id}"
        )],
    ))
}

/// `openhuman.composio_sync` — run a sync pass for a connected account
/// by dispatching to the toolkit's registered provider. `reason` is
/// `"manual"` by default; the periodic scheduler passes `"periodic"`
/// and the OAuth event subscriber passes `"connection_created"`.
pub async fn composio_sync(
    config: &Config,
    connection_id: &str,
    reason: Option<String>,
) -> OpResult<RpcOutcome<SyncOutcome>> {
    let reason = parse_sync_reason(reason.as_deref())?;
    tracing::debug!(
        connection_id = %connection_id,
        reason = reason.as_str(),
        "[composio] rpc sync"
    );
    let client = resolve_client(config)?;
    let toolkit = resolve_toolkit_for_connection(&client, connection_id).await?;

    let provider = get_provider(&toolkit).ok_or_else(|| {
        format!("[composio] no native provider registered for toolkit '{toolkit}'")
    })?;

    let ctx = ProviderContext {
        config: Arc::new(config.clone()),
        client,
        toolkit: toolkit.clone(),
        connection_id: Some(connection_id.to_string()),
    };

    let outcome = provider
        .sync(&ctx, reason)
        .await
        .map_err(|e| format!("[composio] sync({toolkit}) failed: {e}"))?;

    let summary = outcome.summary.clone();
    Ok(RpcOutcome::new(outcome, vec![summary]))
}

/// Parse the optional `reason` parameter into a [`SyncReason`].
///
/// `None` and the explicit `"manual"` value both map to
/// [`SyncReason::Manual`]. Any other unrecognized string is rejected
/// with a clear error so a typo in a caller (UI, CLI, agent) surfaces
/// at the RPC boundary instead of being silently coerced.
fn parse_sync_reason(raw: Option<&str>) -> OpResult<SyncReason> {
    match raw {
        None | Some("manual") => Ok(SyncReason::Manual),
        Some("periodic") => Ok(SyncReason::Periodic),
        Some("connection_created") => Ok(SyncReason::ConnectionCreated),
        Some(other) => Err(format!(
            "[composio] unrecognized sync reason '{other}': expected one of \
             'manual', 'periodic', 'connection_created'"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sync_reason_accepts_known_values() {
        assert_eq!(parse_sync_reason(None).unwrap(), SyncReason::Manual);
        assert_eq!(
            parse_sync_reason(Some("manual")).unwrap(),
            SyncReason::Manual
        );
        assert_eq!(
            parse_sync_reason(Some("periodic")).unwrap(),
            SyncReason::Periodic
        );
        assert_eq!(
            parse_sync_reason(Some("connection_created")).unwrap(),
            SyncReason::ConnectionCreated
        );
    }

    #[test]
    fn parse_sync_reason_rejects_unknown_values() {
        let err = parse_sync_reason(Some("scheduled")).unwrap_err();
        assert!(err.contains("unrecognized sync reason"));
        assert!(err.contains("scheduled"));
        // Typo of a real value should also fail rather than coerce.
        assert!(parse_sync_reason(Some("Periodic")).is_err());
        assert!(parse_sync_reason(Some("")).is_err());
    }
}

// ── Helpers re-exported so callers can pull connection/tool types without
// reaching into the nested types module.
pub use super::types::{ComposioConnection as Connection, ComposioToolSchema as ToolSchemaType};
