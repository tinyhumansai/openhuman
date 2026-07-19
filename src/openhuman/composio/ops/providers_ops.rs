//! Provider-backed ops: profile fetch, identity refresh, and sync.

use std::sync::Arc;

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::super::providers::{
    get_provider, ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason,
};
use super::connections::resolve_toolkit_for_connection;
use super::error_utils::{report_composio_op_error, resolve_client, OpResult};

/// Aggregate result of [`composio_refresh_all_identities`].
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct RefreshIdentitiesReport {
    pub refreshed: usize,
    pub failed: usize,
    pub skipped_no_provider: usize,
    pub skipped_inactive: usize,
    pub rows_written: usize,
}

/// `openhuman.composio_get_user_profile` — fetch a normalized user
/// profile for a connected account by dispatching to the toolkit's
/// registered [`super::super::providers::ComposioProvider`].
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

    let _ = client;
    let ctx = ProviderContext {
        config: Arc::new(config.clone()),
        toolkit: toolkit.clone(),
        connection_id: Some(connection_id.to_string()),
        usage: Default::default(),
        max_items: None,
        sync_depth_days: None,
    };

    let profile = provider.fetch_user_profile(&ctx).await.map_err(|e| {
        report_composio_op_error("get_user_profile", &e);
        format!("[composio] get_user_profile({toolkit}) failed: {e}")
    })?;

    let facets = provider.identity_set(&profile);
    tracing::debug!(
        toolkit = %toolkit,
        facets_written = facets,
        "[composio] identity_set persisted profile facets from get_user_profile"
    );

    Ok(RpcOutcome::new(
        profile,
        vec![format!(
            "composio: fetched {toolkit} profile for connection {connection_id}"
        )],
    ))
}

/// `openhuman.composio_refresh_all_identities` — re-fetch the user
/// profile for every active connection and persist via `identity_set`.
/// Used to populate kind-tagged `user_profile` rows on existing
/// connections after the #1365 schema rewrite without waiting for the
/// next periodic sync tick.
///
/// Best-effort per connection: a failure on one toolkit does not abort
/// the others. Returns aggregate counts plus a per-connection trail in
/// the envelope messages.
pub async fn composio_refresh_all_identities(
    config: &Config,
) -> OpResult<RpcOutcome<RefreshIdentitiesReport>> {
    tracing::info!("[composio] rpc refresh_all_identities");
    let client = resolve_client(config)?;
    let conns = client.list_connections().await.map_err(|e| {
        report_composio_op_error("refresh_all_identities", &e);
        format!("[composio] list_connections failed: {e:#}")
    })?;

    let mut report = RefreshIdentitiesReport::default();
    let mut messages: Vec<String> = Vec::with_capacity(conns.connections.len() + 1);

    for conn in conns.connections {
        if !conn.is_active() {
            report.skipped_inactive += 1;
            continue;
        }
        let toolkit = conn.toolkit.clone();
        let connection_id = conn.id.clone();

        let Some(provider) = get_provider(&toolkit) else {
            tracing::debug!(
                toolkit = %toolkit,
                connection_id = %connection_id,
                "[composio] refresh_all_identities: no native provider — skipping"
            );
            report.skipped_no_provider += 1;
            messages.push(format!(
                "{toolkit}/{connection_id}: skipped (no native provider)"
            ));
            continue;
        };

        let ctx = ProviderContext {
            config: Arc::new(config.clone()),
            toolkit: toolkit.clone(),
            connection_id: Some(connection_id.clone()),
            usage: Default::default(),
            max_items: None,
            sync_depth_days: None,
        };

        match provider.fetch_user_profile(&ctx).await {
            Ok(profile) => {
                let rows = provider.identity_set(&profile);
                report.refreshed += 1;
                report.rows_written += rows;
                tracing::debug!(
                    toolkit = %toolkit,
                    connection_id = %connection_id,
                    rows_written = rows,
                    "[composio] refresh_all_identities: identity_set ok"
                );
                messages.push(format!("{toolkit}/{connection_id}: {rows} row(s)"));
            }
            Err(e) => {
                report.failed += 1;
                tracing::warn!(
                    toolkit = %toolkit,
                    connection_id = %connection_id,
                    error = %e,
                    "[composio] refresh_all_identities: fetch_user_profile failed"
                );
                messages.push(format!("{toolkit}/{connection_id}: ERROR — {e}"));
            }
        }
    }

    let summary = format!(
        "composio: refreshed {ok}/{tried} active conn(s) — {rows} rows; \
         {fail} failed, {nopv} skipped (no provider), {inact} inactive",
        ok = report.refreshed,
        tried = report.refreshed + report.failed + report.skipped_no_provider,
        rows = report.rows_written,
        fail = report.failed,
        nopv = report.skipped_no_provider,
        inact = report.skipped_inactive,
    );
    let mut envelope = vec![summary];
    envelope.extend(messages);
    Ok(RpcOutcome::new(report, envelope))
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
        "[composio] rpc sync (spawned)"
    );
    let client = resolve_client(config)?;
    let toolkit = resolve_toolkit_for_connection(&client, connection_id).await?;
    get_provider(&toolkit).ok_or_else(|| {
        format!("[composio] no native provider registered for toolkit '{toolkit}'")
    })?;
    let _ = client;
    let config_for_task = config.clone();
    let started_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let toolkit_for_outcome = toolkit.clone();
    let connection_id_for_log = connection_id.to_string();

    tokio::spawn(async move {
        match crate::openhuman::tinycortex::run_composio_connection(
            &toolkit_for_outcome,
            &connection_id_for_log,
            &config_for_task,
        )
        .await
        {
            Ok(out) => {
                tracing::info!(
                    toolkit = %toolkit_for_outcome,
                    connection_id = %connection_id_for_log,
                    items_ingested = out.records_ingested,
                    actions_called = out.actions_called,
                    "[composio] background sync ok"
                );
            }
            Err(e) => {
                report_composio_op_error("sync", &e);
                tracing::warn!(
                    toolkit = %toolkit_for_outcome,
                    connection_id = %connection_id_for_log,
                    error = %e,
                    "[composio] background sync failed"
                );
            }
        }
    });

    let summary = format!("composio: {toolkit} sync started (background)");
    let outcome = SyncOutcome {
        toolkit,
        connection_id: Some(connection_id.to_string()),
        reason: reason.as_str().to_string(),
        items_ingested: 0,
        started_at_ms,
        finished_at_ms: 0,
        summary: summary.clone(),
        details: serde_json::json!({ "status": "started" }),
    };
    Ok(RpcOutcome::new(outcome, vec![summary]))
}

/// Parse the optional `reason` parameter into a [`SyncReason`].
///
/// `None` and the explicit `"manual"` value both map to
/// [`SyncReason::Manual`]. Any other unrecognized string is rejected
/// with a clear error so a typo in a caller surfaces at the RPC boundary.
pub(crate) fn parse_sync_reason(raw: Option<&str>) -> OpResult<SyncReason> {
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
