//! Connection listing, authorization, deletion, and identity enrichment ops.

use std::collections::HashMap;

use super::super::client::{create_composio_client, direct_list_connections, ComposioClientKind};
use super::super::module_client::{self as connectors, methods};
use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::super::connected_integrations::{
    fetch_connected_integrations_status, invalidate_connected_integrations_cache,
    sync_cache_with_connections, FetchConnectedIntegrationsStatus,
};
use super::super::identity_store::{delete_connected_identity_facets, load_connected_identities};
use super::super::types::{
    ComposioAuthorizeRequest, ComposioAuthorizeResponse, ComposioConnectionsResponse,
    ComposioDeleteConnectionRequest, ComposioDeleteResponse,
};
use super::error_utils::{direct_mode_without_key, report_composio_op_error, OpResult};
use super::memory_cleanup::composio_memory_targets_for_connection;
use tinymemory_api::composio::normalize_connection_identifier;

pub async fn composio_list_connections(
    config: &Config,
) -> OpResult<RpcOutcome<ComposioConnectionsResponse>> {
    tracing::debug!("[composio] rpc list_connections");
    if direct_mode_without_key(config)? {
        tracing::debug!(
            "[composio] list_connections: direct mode selected, no api key configured yet \
             — returning empty connection list (valid setup state, not an error)"
        );
        return Ok(RpcOutcome::new(
            ComposioConnectionsResponse {
                connections: Vec::new(),
            },
            vec!["composio: direct mode — no api key configured yet, 0 connection(s)".to_string()],
        ));
    }
    // The connector module owns the backend-proxied route. Direct mode stays
    // host-side because its client accepts the local loopback overrides used
    // by desktop development and its v3 response mapper lives here.
    let resp =
        if config.composio.mode.trim() == crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT {
            let ComposioClientKind::Direct(direct) = create_composio_client(config)
                .map_err(|error| format!("[composio-direct] list_connections: {error:#}"))?
            else {
                unreachable!("direct Composio mode must construct a direct client")
            };
            direct_list_connections(&direct).await.map_err(|error| {
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
    sync_cache_with_connections(&resp.connections);
    let resp = enrich_connections_with_identity(config, resp).await;
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
    extra_params: Option<serde_json::Value>,
) -> OpResult<RpcOutcome<ComposioAuthorizeResponse>> {
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
    clear_memory: bool,
) -> OpResult<RpcOutcome<ComposioDeleteResponse>> {
    tracing::debug!(connection_id = %connection_id, "[composio] rpc delete_connection");
    let toolkit = match resolve_toolkit_for_connection(config, connection_id).await {
        Ok(toolkit) => Some(toolkit),
        Err(error) if clear_memory => {
            return Err(format!(
                "[composio] delete_connection cannot clear memory without resolving toolkit: {error}"
            ));
        }
        Err(_) => None,
    };
    let memory_targets = if clear_memory {
        // Target discovery takes the config and resolves the bound driver
        // itself — the notion arm reads sync state through the driver's `Graph`
        // family. This used to resolve the LIVE in-process client here
        // (`memory::ops::helpers::active_memory_client`) and hand it down;
        // openhuman#5560 deleted that engine, and the binding is what replaced
        // it. Discovery still refuses before the connection is deleted rather
        // than after, so a memory store this host cannot reach aborts the
        // delete instead of orphaning the user's synced pages.
        composio_memory_targets_for_connection(config, toolkit.as_deref(), connection_id)
            .await
            .map_err(|error| {
                format!("[composio] delete_connection cannot enumerate memory targets: {error:#}")
            })?
    } else {
        Vec::new()
    };
    // Only the Composio-side removal crosses the bus. Everything around it —
    // the memory targets, the identity facets, PROFILE.md, the memory_sources
    // row — is this host's own bookkeeping about a connection it no longer has,
    // and the module knows nothing about any of it.
    let mut resp = connectors::call::<_, ComposioDeleteResponse>(
        config,
        methods::DELETE_CONNECTION,
        ComposioDeleteConnectionRequest {
            connection_id: connection_id.to_string(),
            clear_memory,
        },
    )
    .await
    .map_err(|error| {
        report_composio_op_error("delete_connection", &anyhow::anyhow!("{error}"));
        format!("[composio] delete_connection failed: {error}")
    })?;
    let mut memory_chunks_deleted = 0;
    let mut memory_clear_errors = Vec::new();
    for target in &memory_targets {
        match target.delete(config).await {
            Ok(deleted) => {
                memory_chunks_deleted += deleted;
            }
            Err(error) => {
                memory_clear_errors.push(format!(
                    "[composio] connection deleted, but failed to clear memory chunks for {}: {error:#}",
                    target.label()
                ));
            }
        }
    }
    resp.memory_chunks_deleted = memory_chunks_deleted;
    if let Some(toolkit) = toolkit.as_deref() {
        let deleted = delete_connected_identity_facets(config, toolkit, connection_id)
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
        if let Err(e) = super::super::profile_md::remove_provider_from_profile_md(
            &config.workspace_dir,
            toolkit,
            connection_id,
        ) {
            tracing::warn!(
                toolkit = %toolkit,
                connection_id = %connection_id,
                error = %e,
                "[composio] PROFILE.md bullet removal failed (non-fatal)"
            );
        }
    }
    match crate::openhuman::memory::sources::registry::remove_composio_source_by_connection_id(
        connection_id,
    )
    .await
    {
        Ok(0) => {}
        Ok(removed) => tracing::debug!(
            connection_id = %connection_id,
            removed,
            "[composio] pruned memory_sources entry after connection deletion"
        ),
        Err(e) => tracing::warn!(
            connection_id = %connection_id,
            error = %e,
            "[composio] failed to prune memory_sources entry after connection deletion (non-fatal)"
        ),
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
    if !memory_clear_errors.is_empty() {
        return Err(memory_clear_errors.join("; "));
    }
    Ok(RpcOutcome::new(
        resp,
        vec![format!("composio: connection {connection_id} deleted")],
    ))
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
    config: &Config,
    mut resp: ComposioConnectionsResponse,
) -> ComposioConnectionsResponse {
    let identities = load_connected_identities(config)
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

    let lookup: HashMap<(String, String), _> = identities
        .iter()
        .map(|id| {
            (
                (
                    normalize_connection_identifier(&id.source),
                    normalize_connection_identifier(&id.identifier),
                ),
                id,
            )
        })
        .collect();

    tracing::debug!(
        total = resp.connections.len(),
        cached_identities = identities.len(),
        "[composio] enrich_connections_with_identity: enriching connection labels"
    );

    for conn in &mut resp.connections {
        if conn.account_email.is_some() || conn.workspace.is_some() || conn.username.is_some() {
            continue;
        }
        let toolkit_key = normalize_connection_identifier(&conn.toolkit);
        let conn_id_key = normalize_connection_identifier(&conn.id);
        if let Some(identity) = lookup.get(&(toolkit_key, conn_id_key)) {
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
