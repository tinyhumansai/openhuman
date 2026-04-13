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
    // Bust the integrations cache so the next prompt reflects the removal.
    invalidate_connected_integrations_cache();
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

// ── Prompt integration discovery ────────────────────────────────────

use crate::openhuman::context::prompt::{ConnectedIntegration, ConnectedIntegrationTool};
use std::collections::HashMap;
use std::sync::RwLock;

/// Process-wide cache for connected integrations, keyed by the config
/// identity (the `config_path` string) so different user contexts don't
/// collide. Each entry is populated on first fetch and returned on
/// subsequent calls until explicitly invalidated.
static INTEGRATIONS_CACHE: RwLock<HashMap<String, Vec<ConnectedIntegration>>> =
    RwLock::new(HashMap::new());

/// Derive a stable cache key from a [`Config`]. We use the stringified
/// `config_path` because it uniquely identifies a user context (it
/// resolves to the per-user openhuman dir).
fn cache_key(config: &Config) -> String {
    config.config_path.display().to_string()
}

/// Clear cached connected integrations so the next call to
/// [`fetch_connected_integrations`] hits the backend again.
///
/// Called by [`super::bus::ComposioConnectionCreatedSubscriber`] when a
/// new OAuth connection completes, and can also be called from tests.
/// Clears the entire map because the bus subscriber doesn't carry a
/// config reference.
pub fn invalidate_connected_integrations_cache() {
    if let Ok(mut guard) = INTEGRATIONS_CACHE.write() {
        guard.clear();
        tracing::debug!("[composio] connected integrations cache invalidated");
    }
}

/// Fetch the user's active Composio connections and their available
/// tool actions, returning a prompt-ready summary.
///
/// This is the **single source of truth** for connected integration
/// data injected into system prompts — both the agent turn loop and
/// the debug dump CLI call this function.
///
/// Results are cached process-wide (keyed by config identity) and
/// returned instantly on subsequent calls. The cache is invalidated
/// when a new connection is created
/// (via [`invalidate_connected_integrations_cache`]) or on process
/// restart.
///
/// Best-effort: returns an empty vec when the user isn't signed in,
/// the backend is unreachable, or any step fails.
pub async fn fetch_connected_integrations(config: &Config) -> Vec<ConnectedIntegration> {
    let key = cache_key(config);

    // Fast path: return cached result.
    if let Ok(guard) = INTEGRATIONS_CACHE.read() {
        if let Some(cached) = guard.get(&key) {
            tracing::debug!(
                count = cached.len(),
                key = %key,
                "[composio] fetch_connected_integrations: returning cached result"
            );
            return cached.clone();
        }
    }

    match fetch_connected_integrations_uncached(config).await {
        Some(result) => {
            // Backend was reachable — cache the result (even if empty).
            if let Ok(mut guard) = INTEGRATIONS_CACHE.write() {
                guard.insert(key, result.clone());
            }
            result
        }
        None => {
            // No auth / client unavailable — do NOT cache so a
            // subsequent call with a different config can retry.
            Vec::new()
        }
    }
}

/// The actual backend fetch, called on cache miss.
///
/// Returns `Some(vec)` when the backend was reachable (even if the user
/// has zero connections — that's a valid cacheable state). Returns `None`
/// when we couldn't even build a client (no auth), signalling the caller
/// should NOT cache this result.
async fn fetch_connected_integrations_uncached(
    config: &Config,
) -> Option<Vec<ConnectedIntegration>> {
    use super::providers::toolkit_description;

    let Some(client) = build_composio_client(config) else {
        tracing::debug!("[composio] fetch_connected_integrations: no client (not signed in?)");
        return None;
    };

    let connections = match client.list_connections().await {
        Ok(resp) => resp.connections,
        Err(e) => {
            tracing::warn!("[composio] fetch_connected_integrations: list_connections failed: {e}");
            return Some(Vec::new());
        }
    };

    let active: Vec<_> = connections
        .iter()
        .filter(|c| c.status == "ACTIVE" || c.status == "CONNECTED")
        .collect();

    if active.is_empty() {
        return Some(Vec::new());
    }

    // Collect the unique toolkit slugs so we can batch-fetch their tools.
    let toolkit_slugs: Vec<String> = {
        let mut slugs: Vec<String> = active.iter().map(|c| c.toolkit.clone()).collect();
        slugs.sort();
        slugs.dedup();
        slugs
    };

    // Fetch available tool schemas for all connected toolkits in one call.
    let tools_by_toolkit = match client.list_tools(Some(&toolkit_slugs)).await {
        Ok(resp) => resp.tools,
        Err(e) => {
            tracing::warn!("[composio] fetch_connected_integrations: list_tools failed: {e}");
            Vec::new()
        }
    };

    // Build the per-toolkit integration entries.
    let mut integrations: Vec<ConnectedIntegration> = toolkit_slugs
        .iter()
        .map(|slug| {
            let tools: Vec<ConnectedIntegrationTool> = tools_by_toolkit
                .iter()
                .filter(|t| {
                    // Composio action slugs are prefixed with the toolkit
                    // name in uppercase, e.g. GMAIL_SEND_EMAIL.
                    t.function.name.starts_with(&slug.to_uppercase())
                })
                .map(|t| ConnectedIntegrationTool {
                    name: t.function.name.clone(),
                    description: t.function.description.clone().unwrap_or_default(),
                })
                .collect();

            ConnectedIntegration {
                toolkit: slug.clone(),
                description: toolkit_description(slug).to_string(),
                tools,
            }
        })
        .collect();

    integrations.sort_by(|a, b| a.toolkit.cmp(&b.toolkit));

    tracing::info!(
        count = integrations.len(),
        "[composio] fetch_connected_integrations: done"
    );
    for ci in &integrations {
        tracing::debug!(
            toolkit = %ci.toolkit,
            tool_count = ci.tools.len(),
            "[composio] connected integration"
        );
    }

    Some(integrations)
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
