//! Connection listing, authorization, deletion, and identity enrichment ops.

use std::collections::HashMap;

use super::super::client::{direct_list_connections, resolve_composio_route, ComposioRoute};
use super::super::module_client::{self as connectors};
use crate::config::Config;
use crate::core::Outcome;
use tinyconnectors_bus::names::methods;

use super::super::connected_integrations::{
    fetch_connected_integrations_status, invalidate_connected_integrations_cache,
    sync_cache_with_connections, FetchConnectedIntegrationsStatus,
};
use super::super::identity_store::{delete_connected_identity_facets, load_connected_identities};
use super::super::types::{
    ComposioAuthorizeRequest, ComposioAuthorizeResponse, ComposioConnectionsResponse,
    ComposioDeleteConnectionRequest, ComposioDeleteResponse,
};

/// Matches cached identity records to normalized provider/account keys shared
/// by the settings response and the flow connection picker.
pub(crate) fn identity_lookup<'a>(
    identities: &'a [crate::integrations::composio::contract::ConnectedIdentity],
    normalized_keys: &HashMap<String, String>,
) -> HashMap<(String, String), &'a crate::integrations::composio::contract::ConnectedIdentity> {
    let normalize = |value: &str| {
        normalized_keys
            .get(value)
            .cloned()
            .unwrap_or_else(|| value.to_string())
    };
    identities
        .iter()
        .map(|identity| {
            (
                (normalize(&identity.source), normalize(&identity.identifier)),
                identity,
            )
        })
        .collect()
}
use super::error_utils::{
    backend_mode_without_session, direct_mode_without_key, report_composio_op_error, OpResult,
    COMPOSIO_NO_SESSION,
};
use crate::integrations::composio::contract::normalize_connection_identifiers;

/// Secret-free Composio picker row consumed by the flow RPC adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FlowConnectionEntry {
    pub connection_ref: String,
    pub display: String,
    pub toolkit: String,
    pub platform_user_id: Option<String>,
}

/// Builds Composio picker rows and attaches cached member identities.
pub(crate) fn flow_connection_entries(
    connections: &[crate::integrations::composio::ComposioConnection],
    identities: &[crate::integrations::composio::contract::ConnectedIdentity],
    keys: &HashMap<String, String>,
) -> Vec<FlowConnectionEntry> {
    let identity_lookup = identity_lookup(identities, keys);
    connections
        .iter()
        .filter(|connection| connection.is_active())
        .map(|connection| {
            let toolkit = connection.normalized_toolkit();
            let normalize = |raw: &str| keys.get(raw).cloned().unwrap_or_else(|| raw.to_string());
            let platform_user_id = identity_lookup
                .get(&(normalize(&toolkit), normalize(&connection.id)))
                .and_then(|identity| identity.user_id.clone());
            let title = title_case_toolkit(&toolkit);
            let identity = connection
                .account_email
                .as_deref()
                .or(connection.workspace.as_deref())
                .or(connection.username.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty());
            FlowConnectionEntry {
                connection_ref: format!("composio:{toolkit}:{}", connection.id),
                display: identity.map_or(title.clone(), |value| format!("{title} · {value}")),
                toolkit,
                platform_user_id,
            }
        })
        .collect()
}

/// Title-cases a toolkit slug for display.
pub(crate) fn title_case_toolkit(toolkit: &str) -> String {
    let trimmed = toolkit.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed
        .split(['_', '-', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub async fn composio_list_connections(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
) -> OpResult<Outcome<ComposioConnectionsResponse>> {
    tracing::debug!("[composio] rpc list_connections");
    if direct_mode_without_key(config).await? {
        tracing::debug!(
            "[composio] list_connections: direct mode selected, no api key configured yet \
             — returning empty connection list (valid setup state, not an error)"
        );
        return Ok(Outcome::new(
            ComposioConnectionsResponse {
                connections: Vec::new(),
            },
            vec!["composio: direct mode — no api key configured yet, 0 connection(s)".to_string()],
        ));
    }
    if backend_mode_without_session(config).await {
        // No session means no proxy route, so the connector module cannot
        // answer — but the connections may well exist server-side. That makes
        // this "unavailable", not "none": callers that tell the two apart
        // (`flows::validate_connection_refs` fails open on `Err`) must keep
        // doing so, which an empty `Ok` would silently defeat. It is also not
        // a fault, so it is not reported here: nothing reaches Sentry for a
        // user who has simply not signed in yet (#6176). The wording is the one
        // every other backend-mode member gives in this state, which the
        // JSON-RPC boundary also recognises as expected.
        tracing::debug!(
            "[composio] list_connections: backend mode selected, not signed in yet \
             — connections unavailable until sign-in (valid setup state, not reported)"
        );
        // A module configured while the user was signed in still holds that
        // bearer, and the per-call route reconciliation that would tell it to
        // drop it is exactly what answering here skips. Give an already-loaded
        // module the instruction now; a module that was never loaded holds no
        // credential and is not loaded for this.
        if let Err(error) = connectors::reconcile_route_if_loaded(config).await {
            tracing::warn!(
                "[composio] list_connections: could not drop the connector module's route \
                 after sign-out ({error}); the next routed call retries"
            );
        }
        return Err(COMPOSIO_NO_SESSION.to_string());
    }
    // The connector module owns the backend-proxied route. Direct mode stays
    // host-side because its client accepts the local loopback overrides used
    // by desktop development and its v3 response mapper lives here.
    let resp = if config.composio.mode.trim() == crate::config::schema::COMPOSIO_MODE_DIRECT {
        let ComposioRoute::Direct(direct) = resolve_composio_route(config)
            .await
            .map_err(|error| format!("[composio-direct] list_connections: {error:#}"))?
        else {
            unreachable!("direct Composio mode must construct a direct client")
        };
        direct_list_connections(config, &direct)
            .await
            .map_err(|error| {
                report_composio_op_error("list_connections", &error);
                format!("[composio-direct] list_connections: {error:#}")
            })?
    } else {
        connectors::call_bare::<ComposioConnectionsResponse>(config, methods::LIST_CONNECTIONS)
            .await
            .map_err(|error| {
                report_composio_op_error("list_connections", &anyhow::anyhow!("{error}"));
                format!("[composio] list_connections failed: {error}")
            })?
    };

    let active = resp.connections.iter().filter(|c| c.is_active()).count();
    let total = resp.connections.len();
    if sync_cache_with_connections(&resp.connections) {
        // The Settings poll also catches OAuth completions that outlive the
        // connection-created subscriber's readiness window. Re-warm here so
        // the next chat turn does not perform the three discovery requests.
        let config = config.clone();
        ctx.spawn(async move {
            let _ = fetch_connected_integrations_status(&config).await;
        });
    }
    let resp = enrich_connections_with_identity(ctx, config, resp).await;
    Ok(Outcome::new(
        resp,
        vec![format!(
            "composio: {total} connection(s) listed ({active} active)"
        )],
    ))
}

pub async fn composio_authorize(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    toolkit: &str,
    extra_params: Option<serde_json::Value>,
) -> OpResult<Outcome<ComposioAuthorizeResponse>> {
    tracing::debug!(toolkit = %toolkit, has_extra_params = extra_params.is_some(), "[composio] rpc authorize");
    // The module owns the whole handoff: the Meta pre-clean, the 429 backoff,
    // and the guidance message that replaces an unhelpful rate-limit error.
    // It also owns the difference between the proxy's authorize body and v3's
    // link call, so `extra_params` no longer needs a per-route caveat here.
    let resp = connectors::call::<_, ComposioAuthorizeResponse>(
        config,
        methods::AUTHORIZE,
        ComposioAuthorizeRequest {
            toolkit: toolkit.to_string(),
            extra_params,
        },
    )
    .await
    .map_err(|error| {
        report_composio_op_error("authorize", &anyhow::anyhow!("{error}"));
        format!("[composio] authorize failed: {error}")
    })?;

    crate::core::bus::BUS.publish(
        crate::core::events::DomainEvent::ComposioConnectionCreated {
            toolkit: toolkit.to_string(),
            connection_id: resp.connection_id.clone(),
            connect_url: resp.connect_url.clone(),
            tenant: ctx.event_tenant(),
        },
    );

    Ok(Outcome::new(
        resp,
        vec![format!("composio: authorize flow started for {toolkit}")],
    ))
}

/// The `composio.delete_connection` reply.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ComposioDeleteResult {
    /// Whether the backend confirmed the deletion.
    pub deleted: bool,
}

pub async fn composio_delete_connection(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    connection_id: &str,
) -> OpResult<Outcome<ComposioDeleteResult>> {
    tracing::debug!(connection_id = %connection_id, "[composio] rpc delete_connection");
    // The toolkit names the identity facets to drop; an unresolvable toolkit
    // skips that cleanup only.
    let toolkit = resolve_toolkit_for_connection(config, connection_id)
        .await
        .ok();
    // Only the Composio-side removal crosses the bus. The identity facets
    // are this host's own bookkeeping about a connection it no longer has.
    let resp = connectors::call::<_, ComposioDeleteResponse>(
        config,
        methods::DELETE_CONNECTION,
        ComposioDeleteConnectionRequest {
            connection_id: connection_id.to_string(),
        },
    )
    .await
    .map_err(|error| {
        report_composio_op_error("delete_connection", &anyhow::anyhow!("{error}"));
        format!("[composio] delete_connection failed: {error}")
    })?;
    if let Some(toolkit) = toolkit.as_deref() {
        let deleted = delete_connected_identity_facets(ctx, config, toolkit, connection_id)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(
                    toolkit = %toolkit,
                    connection_id = %connection_id,
                    %error,
                    "[composio] delete_connected_identity_facets failed (non-fatal)"
                );
                0
            });
        tracing::debug!(
            toolkit = %toolkit,
            connection_id = %connection_id,
            facets_deleted = deleted,
            "[composio] deleted connected identity facets after connection removal"
        );
    }
    crate::core::bus::BUS.publish(
        crate::core::events::DomainEvent::ComposioConnectionDeleted {
            toolkit: toolkit.unwrap_or_else(|| "unknown".to_string()),
            connection_id: connection_id.to_string(),
        },
    );
    invalidate_connected_integrations_cache();
    match fetch_connected_integrations_status(config).await {
        FetchConnectedIntegrationsStatus::Authoritative(entries) => {
            tracing::debug!(
                connection_id = %connection_id,
                cached_entries = entries.len(),
                "[composio] eagerly warmed integrations cache after connection deletion"
            );
        }
        FetchConnectedIntegrationsStatus::Unavailable => {
            tracing::warn!(
                connection_id = %connection_id,
                "[composio] eager cache warm after connection deletion skipped: backend unavailable"
            );
        }
    }
    Ok(Outcome::new(
        ComposioDeleteResult {
            deleted: resp.deleted,
        },
        vec![format!("composio: connection {connection_id} deleted")],
    ))
}

/// The ids of every active connection of `toolkit` (matched
/// case-insensitively), through the connector module's `ListConnections`.
///
/// # Errors
///
/// The connector call fails.
pub async fn active_connection_ids(config: &Config, toolkit: &str) -> OpResult<Vec<String>> {
    let toolkit = toolkit.trim().to_ascii_lowercase();
    let resp =
        connectors::call_bare::<ComposioConnectionsResponse>(config, methods::LIST_CONNECTIONS)
            .await
            .map_err(|error| {
                report_composio_op_error("active_connection_ids", &anyhow::anyhow!("{error}"));
                format!("[composio] list_connections failed: {error}")
            })?;
    let ids: Vec<String> = resp
        .connections
        .into_iter()
        .filter(|c| c.is_active() && c.normalized_toolkit() == toolkit)
        .map(|c| c.id)
        .collect();
    tracing::debug!(
        toolkit = %toolkit,
        active = ids.len(),
        "[composio] active_connection_ids"
    );
    Ok(ids)
}

/// Look up the toolkit slug for an existing connection.
pub(super) async fn resolve_toolkit_for_connection(
    config: &Config,
    connection_id: &str,
) -> OpResult<String> {
    tracing::debug!(connection_id = %connection_id, "[composio] resolve_toolkit_for_connection");
    let resp =
        connectors::call_bare::<ComposioConnectionsResponse>(config, methods::LIST_CONNECTIONS)
            .await
            .map_err(|error| {
                report_composio_op_error(
                    "resolve_toolkit_for_connection",
                    &anyhow::anyhow!("{error}"),
                );
                format!("[composio] list_connections failed: {error}")
            })?;
    let conn = resp
        .connections
        .into_iter()
        .find(|c| c.id == connection_id)
        .ok_or_else(|| format!("[composio] no connection with id '{connection_id}'"))?;
    Ok(conn.toolkit)
}

/// Enrich each [`ComposioConnectionsResponse`] connection with human-readable
/// identity fields (`account_email`, `workspace`, `username`) from the
/// persisted provider profile cache so the UI picker can show
/// "Gmail · user@example.com" instead of a generic "Account N" label.
///
/// This is best-effort — no live API calls are made (one SQLite read per poll).
pub(crate) async fn enrich_connections_with_identity(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    mut resp: ComposioConnectionsResponse,
) -> ComposioConnectionsResponse {
    let identities = load_connected_identities(ctx, config)
        .await
        .unwrap_or_else(|error| {
            tracing::debug!(
                %error,
                "[composio] enrich_connections_with_identity: load_connected_identities failed"
            );
            Vec::new()
        });
    if identities.is_empty() {
        tracing::debug!(
            "[composio] enrich_connections_with_identity: no cached identities yet \
             — picker will fall back to numbered labels until first sync completes"
        );
        return resp;
    }

    // One module call normalises every identity key and every connection key.
    let mut raw: Vec<String> = identities
        .iter()
        .flat_map(|id| [id.source.clone(), id.identifier.clone()])
        .collect();
    for conn in &resp.connections {
        raw.push(conn.toolkit.clone());
        raw.push(conn.id.clone());
    }
    let normalized = match normalize_connection_identifiers(config, &raw).await {
        Ok(normalized) if normalized.len() == raw.len() => normalized,
        other => {
            tracing::debug!(
                error = ?other.err(),
                "[composio] enrich_connections_with_identity: key normalisation unavailable"
            );
            return resp;
        }
    };
    let connection_keys = normalized[identities.len() * 2..].to_vec();
    let normalized_keys: HashMap<_, _> = raw.into_iter().zip(normalized).collect();
    let lookup = identity_lookup(&identities, &normalized_keys);

    tracing::debug!(
        total = resp.connections.len(),
        cached_identities = identities.len(),
        "[composio] enrich_connections_with_identity: enriching connection labels"
    );

    for (conn, keys) in resp.connections.iter_mut().zip(connection_keys.chunks(2)) {
        if conn.account_email.is_some() || conn.workspace.is_some() || conn.username.is_some() {
            continue;
        }
        if let Some(identity) = lookup.get(&(keys[0].clone(), keys[1].clone())) {
            conn.account_email = identity.email.clone();
            conn.workspace = identity.display_name.clone();
            conn.username = identity.handle.clone();
            tracing::debug!(
                toolkit = %conn.toolkit,
                connection_id = %conn.id,
                has_email = conn.account_email.is_some(),
                has_workspace = conn.workspace.is_some(),
                has_username = conn.username.is_some(),
                "[composio] enrich_connections_with_identity: enriched connection"
            );
        }
    }
    resp
}
