//! Connection listing, authorization, deletion, and identity enrichment ops.

use std::collections::HashMap;

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::super::client::{
    create_composio_client, direct_list_connections, ComposioClient, ComposioClientKind,
};
use super::super::connected_integrations::{
    fetch_connected_integrations_status, invalidate_connected_integrations_cache,
    sync_cache_with_connections, FetchConnectedIntegrationsStatus,
};
use super::super::providers::profile::{
    load_connected_identities, normalize_connection_identifier,
};
use super::super::types::{
    ComposioAuthorizeResponse, ComposioConnectionsResponse, ComposioDeleteResponse,
};
use super::error_utils::{
    direct_mode_without_key, report_composio_op_error, resolve_client, OpResult,
};
use super::memory_cleanup::composio_memory_targets_for_connection;

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
    let kind =
        create_composio_client(config).map_err(|e| format!("[composio] list_connections: {e}"))?;
    let client = match kind {
        ComposioClientKind::Backend(client) => {
            tracing::debug!("[composio] list_connections: backend variant");
            client
        }
        ComposioClientKind::Direct(direct) => {
            tracing::info!(
                "[composio-direct] list_connections: fetching v3 \
                 /connected_accounts for the user's personal Composio tenant"
            );
            let resp = direct_list_connections(&direct).await.map_err(|e| {
                let rendered = format!("[composio-direct] list_connections failed: {e:#}");
                report_composio_op_error("list_connections", &rendered);
                rendered
            })?;
            let active = resp.connections.iter().filter(|c| c.is_active()).count();
            let total = resp.connections.len();
            sync_cache_with_connections(&resp.connections);
            let resp = enrich_connections_with_identity(resp);
            return Ok(RpcOutcome::new(
                resp,
                vec![format!(
                    "composio: direct mode — {total} connection(s) listed ({active} active)"
                )],
            ));
        }
    };
    let resp = client.list_connections().await.map_err(|e| {
        report_composio_op_error("list_connections", &e);
        format!("[composio] list_connections failed: {e:#}")
    })?;
    let active = resp.connections.iter().filter(|c| c.is_active()).count();
    let total = resp.connections.len();
    sync_cache_with_connections(&resp.connections);
    let resp = enrich_connections_with_identity(resp);
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
    let kind = create_composio_client(config).map_err(|e| format!("[composio] authorize: {e}"))?;
    let resp = match kind {
        ComposioClientKind::Backend(client) => {
            tracing::debug!(toolkit = %toolkit, "[composio] authorize: backend variant");
            super::super::oauth_handoff::authorize_with_meta_guard(&client, toolkit, extra_params)
                .await
                .map_err(|e| {
                    report_composio_op_error("authorize", &e);
                    let wrapped =
                        super::super::oauth_handoff::wrap_authorize_rate_limit_error(toolkit, e);
                    format!("[composio] authorize failed: {wrapped:#}")
                })?
        }
        ComposioClientKind::Direct(direct) => {
            tracing::info!(
                toolkit = %toolkit,
                "[composio-direct] authorize: routing to user's personal Composio tenant"
            );
            if extra_params.is_some() {
                tracing::warn!(
                    toolkit = %toolkit,
                    "[composio-direct] authorize: extra_params is set but direct mode does \
                     not propagate it — configure toolkit-specific fields via \
                     app.composio.dev for your auth config"
                );
            }
            super::super::oauth_handoff::direct_authorize_with_meta_guard(
                &direct,
                toolkit,
                &config.composio.entity_id,
            )
            .await
            .map_err(|e| {
                let wrapped =
                    super::super::oauth_handoff::wrap_authorize_rate_limit_error(toolkit, e);
                let rendered = format!("[composio-direct] authorize failed: {wrapped:#}");
                report_composio_op_error("authorize", &rendered);
                rendered
            })?
        }
    };

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
    clear_memory: bool,
) -> OpResult<RpcOutcome<ComposioDeleteResponse>> {
    tracing::debug!(connection_id = %connection_id, "[composio] rpc delete_connection");
    let client = resolve_client(config)?;
    let toolkit = match resolve_toolkit_for_connection(&client, connection_id).await {
        Ok(toolkit) => Some(toolkit),
        Err(error) if clear_memory => {
            return Err(format!(
                "[composio] delete_connection cannot clear memory without resolving toolkit: {error}"
            ));
        }
        Err(_) => None,
    };
    let memory_targets = if clear_memory {
        composio_memory_targets_for_connection(config, toolkit.as_deref(), connection_id)
            .await
            .map_err(|error| {
                format!("[composio] delete_connection cannot enumerate memory targets: {error:#}")
            })?
    } else {
        Vec::new()
    };
    let mut resp = client.delete_connection(connection_id).await.map_err(|e| {
        report_composio_op_error("delete_connection", &e);
        format!("[composio] delete_connection failed: {e:#}")
    })?;
    let mut memory_chunks_deleted = 0;
    let mut memory_clear_errors = Vec::new();
    for target in &memory_targets {
        match target.delete(config) {
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
        let deleted = super::super::providers::profile::delete_connected_identity_facets(
            toolkit,
            connection_id,
        );
        tracing::debug!(
            toolkit = %toolkit,
            connection_id = %connection_id,
            facets_deleted = deleted,
            "[composio] deleted connected identity facets after connection removal"
        );
        if let Err(e) = super::super::providers::profile_md::remove_provider_from_profile_md(
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
    match crate::openhuman::memory_sources::registry::remove_composio_source_by_connection_id(
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
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::ComposioConnectionDeleted {
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
    client: &ComposioClient,
    connection_id: &str,
) -> OpResult<String> {
    tracing::debug!(connection_id = %connection_id, "[composio] resolve_toolkit_for_connection");
    let resp = client.list_connections().await.map_err(|e| {
        report_composio_op_error("resolve_toolkit_for_connection", &e);
        format!("[composio] list_connections failed: {e:#}")
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
pub(crate) fn enrich_connections_with_identity(
    mut resp: ComposioConnectionsResponse,
) -> ComposioConnectionsResponse {
    let identities = load_connected_identities();
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
