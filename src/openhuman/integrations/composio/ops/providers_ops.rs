//! Provider-backed ops: profile fetch, identity refresh, and sync.
//!
//! # The division of labour
//!
//! Reading a connected account is the module's job — it holds the credential,
//! the provider registry, and the paging cursors. Writing what it read into
//! memory is this crate's, because the memory driver is bound here and the
//! guard that redacts and taints a batch sits in front of it.
//!
//! So a sync is two calls: `Sync` returns records, and the bound driver's
//! `accept_source_items` ingests them. Neither half knows about the other,
//! which is the point — the module cannot reach the user's memory, and the
//! memory driver never sees a Composio credential.

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::super::module_client::{self as connectors, methods};
use super::super::providers::{ProviderUserProfile, SyncOutcome, SyncReason};
use super::super::types::{
    reencode, ComposioRefreshIdentitiesResponse, ComposioUserProfile, ComposioUserProfileRequest,
};
use super::connections::resolve_toolkit_for_connection;
use super::error_utils::{report_composio_op_error, OpResult};
use crate::openhuman::memory::api::provider::types::SourceItem;
use crate::openhuman::memory::api::types::MemoryTaint;
use tinyconnectors_bus::records::{ConnectorSyncRequest, ConnectorSyncResponse};

/// The source kind every connector record is ingested under.
///
/// The memory driver parses this — it is `SourceKind::Composio`'s wire string —
/// and answers `Invalid` for a kind it does not know, so it is a literal here
/// rather than something derived from the toolkit. Records from Gmail and from
/// Slack are both Composio records; the *toolkit* lives in the source id.
const SOURCE_KIND: &str = "composio";

/// Aggregate result of [`composio_refresh_all_identities`].
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct RefreshIdentitiesReport {
    pub refreshed: usize,
    pub failed: usize,
    pub skipped_no_provider: usize,
    pub skipped_inactive: usize,
    pub rows_written: usize,
}

/// Persist one profile's identity facets and report how many rows it wrote.
///
/// Was routed through the engine's provider registry (`get_provider(toolkit)
/// .identity_set(profile)`), deleted by tinymemory v1.13.4 with no
/// replacement. `identity_store::persist_provider_profile` is this host's own
/// port of what `identity_set`'s default impl did — see its module docs for
/// what carried over (the facet write) and what did not (the deleted
/// engine's `LearningCandidate` emission for stability scoring).
async fn persist_identity(config: &Config, profile: &ComposioUserProfile) -> OpResult<usize> {
    let native: ProviderUserProfile = reencode(profile)?;
    super::super::identity_store::persist_provider_profile(config, &native).await
}

/// `openhuman.composio_get_user_profile` — fetch a normalized user profile for
/// a connected account.
pub async fn composio_get_user_profile(
    config: &Config,
    connection_id: &str,
) -> OpResult<RpcOutcome<ProviderUserProfile>> {
    tracing::debug!(connection_id = %connection_id, "[composio] rpc get_user_profile");
    let toolkit = resolve_toolkit_for_connection(config, connection_id).await?;

    let profile = connectors::call::<_, ComposioUserProfile>(
        config,
        methods::GET_USER_PROFILE,
        ComposioUserProfileRequest {
            toolkit: toolkit.clone(),
            connection_id: Some(connection_id.to_string()),
        },
    )
    .await
    .map_err(|error| {
        report_composio_op_error("get_user_profile", &anyhow::anyhow!("{error}"));
        format!("[composio] get_user_profile({toolkit}) failed: {error}")
    })?;

    let facets = persist_identity(config, &profile).await?;
    tracing::debug!(
        toolkit = %toolkit,
        facets_written = facets,
        "[composio] identity_set persisted profile facets from get_user_profile"
    );

    Ok(RpcOutcome::new(
        reencode(&profile)?,
        vec![format!(
            "composio: fetched {toolkit} profile for connection {connection_id}"
        )],
    ))
}

/// `openhuman.composio_refresh_all_identities` — re-fetch the user profile for
/// every active connection and persist via `identity_set`.
///
/// Best-effort per connection: the module reports the ones it could not read as
/// failures alongside the profiles it could, because a refresh exists precisely
/// to find the broken ones.
pub async fn composio_refresh_all_identities(
    config: &Config,
) -> OpResult<RpcOutcome<RefreshIdentitiesReport>> {
    tracing::info!("[composio] rpc refresh_all_identities");
    let response = connectors::call_bare::<ComposioRefreshIdentitiesResponse>(
        config,
        methods::REFRESH_ALL_IDENTITIES,
    )
    .await
    .map_err(|error| {
        report_composio_op_error("refresh_all_identities", &anyhow::anyhow!("{error}"));
        format!("[composio] refresh_all_identities failed: {error}")
    })?;

    let mut report = RefreshIdentitiesReport::default();
    let mut messages: Vec<String> =
        Vec::with_capacity(response.profiles.len() + response.failures.len() + 1);

    for profile in &response.profiles {
        let connection_id = profile.connection_id.as_deref().unwrap_or("-");
        let toolkit = &profile.toolkit;

        // A toolkit the module read but this build has no facet schema for is
        // not a failure — it is the same "no native provider" case the loop
        // used to skip before fetching, now discovered one step later.
        // `has_native_provider` replaces the deleted engine registry's
        // `get_provider(toolkit).is_none()` — see `providers`'s module docs.
        if !super::super::providers::has_native_provider(toolkit) {
            report.skipped_no_provider += 1;
            messages.push(format!(
                "{toolkit}/{connection_id}: skipped (no native provider)"
            ));
            continue;
        }

        let rows = persist_identity(config, profile).await?;
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

    for failure in &response.failures {
        report.failed += 1;
        tracing::warn!(
            toolkit = %failure.toolkit,
            connection_id = %failure.connection_id,
            error = %failure.message,
            "[composio] refresh_all_identities: fetch_user_profile failed"
        );
        messages.push(format!(
            "{}/{}: ERROR — {}",
            failure.toolkit, failure.connection_id, failure.message
        ));
    }

    let summary = format!(
        "composio: refreshed {ok}/{tried} active conn(s) — {rows} rows; \
         {fail} failed, {nopv} skipped (no provider)",
        ok = report.refreshed,
        tried = report.refreshed + report.failed + report.skipped_no_provider,
        rows = report.rows_written,
        fail = report.failed,
        nopv = report.skipped_no_provider,
    );
    let mut envelope = vec![summary];
    envelope.extend(messages);
    Ok(RpcOutcome::new(report, envelope))
}

/// `openhuman.composio_sync` — read a connected account and write what it
/// returns into memory.
///
/// Returns as soon as the run is *started*: a full sync is minutes of paging,
/// and the RPC caller is a UI button. Progress is reported in the log, and the
/// records land in memory as each page is ingested.
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
    let toolkit = resolve_toolkit_for_connection(config, connection_id).await?;

    // Resolved before the spawn so a driver that serves no ingestion is an
    // error the caller sees rather than a log line in a detached task.
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    if binding.provider().as_sources().is_none() {
        return Err(format!(
            "the bound memory driver '{}' does not accept source items",
            binding.driver_id()
        ));
    }

    let started_at_ms = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
    )
    .unwrap_or(u64::MAX);

    let config_for_task = config.clone();
    let toolkit_for_task = toolkit.clone();
    let connection_for_task = connection_id.to_string();
    let reason_for_task = reason.as_str().to_string();

    tokio::spawn(async move {
        let outcome = run_sync_pass(
            &config_for_task,
            &toolkit_for_task,
            &connection_for_task,
            &reason_for_task,
        )
        .await;
        match outcome {
            Ok(pass) => tracing::info!(
                toolkit = %toolkit_for_task,
                connection_id = %connection_for_task,
                items_ingested = pass.records_read,
                written = pass.written,
                already_ingested = pass.already_ingested,
                more_pending = pass.more_pending,
                "[composio] background sync ok"
            ),
            Err(error) => {
                report_composio_op_error("sync", &anyhow::anyhow!("{error}"));
                tracing::warn!(
                    toolkit = %toolkit_for_task,
                    connection_id = %connection_for_task,
                    error = %error,
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

/// What one [`run_sync_pass`] call did.
///
/// A superset of the `usize` the caller inside this file needs, so
/// `memory::sync::composio::providers::slack::rpc` — the other caller — can
/// build a [`SyncOutcome`] without a second round trip through the module.
#[derive(Debug, Clone, Default)]
pub(crate) struct SyncPassOutcome {
    /// Records the module returned in this page.
    pub records_read: usize,
    /// Of those, how many the driver actually wrote (the rest were already
    /// ingested and unchanged).
    pub written: u32,
    /// Whether the driver treated this whole batch as already ingested and
    /// unchanged (a no-op call) — `IngestOutcome::already_ingested` is a
    /// batch-level flag, not a per-record count.
    pub already_ingested: bool,
    /// Whether the module has more to read — the caller decides whether to
    /// call again.
    pub more_pending: bool,
}

/// Read one connection through the module and ingest what it returns.
///
/// The two halves are deliberately not interleaved with retries or partial
/// commits: the module already decides what a page is and where the cursor
/// stands, and re-deciding that here would give the run two opinions about
/// what has been read.
///
/// `pub(crate)` — also called from
/// `memory::sync::composio::providers::slack::rpc`, which needs the same
/// tinyconnectors-mediated sync pass `composio_sync` runs here, but awaited
/// synchronously rather than fired into a background task (its RPC contract
/// is "return the outcome", not "return that a run started").
pub(crate) async fn run_sync_pass(
    config: &Config,
    toolkit: &str,
    connection_id: &str,
    reason: &str,
) -> Result<SyncPassOutcome, String> {
    // Sync pages the whole account inside the call; the default 30s bus
    // deadline reported failure on runs the module then finished successfully.
    let response = connectors::call_slow::<_, ConnectorSyncResponse>(
        config,
        methods::SYNC,
        ConnectorSyncRequest {
            toolkit: toolkit.to_string(),
            connection_id: Some(connection_id.to_string()),
            reason: Some(reason.to_string()),
            ..ConnectorSyncRequest::default()
        },
    )
    .await?;

    let count = response.batch.records.len();
    if count == 0 {
        return Ok(SyncPassOutcome {
            records_read: 0,
            written: 0,
            already_ingested: false,
            more_pending: !response.batch.complete,
        });
    }

    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let sink = binding.provider().as_sources().ok_or_else(|| {
        format!(
            "the bound memory driver '{}' does not accept source items",
            binding.driver_id()
        )
    })?;

    // `ConnectorRecord` and memory's `SourceItem` carry the same seven keys —
    // the contract crate asserts that against a literal list, so a drift is a
    // failing test there rather than a decode error here.
    let items = reencode::<_, Vec<SourceItem>>(&response.batch.records)?;

    // `ExternalSync`: everything here came from a third-party account over the
    // network, and the taint is what stops it being treated as the user's own
    // words later.
    let outcome = sink
        .accept_source_items(
            &response.batch.source_id,
            SOURCE_KIND,
            items,
            MemoryTaint::ExternalSync,
        )
        .await
        .map_err(|error| format!("ingesting {toolkit} records failed: {error}"))?;

    if !response.batch.complete {
        // The module keeps its own cursor, so the next call resumes where this
        // one stopped. Saying so is worth a line: a partial run that looked
        // complete is how a user concludes half their mail is missing.
        tracing::info!(
            toolkit = %toolkit,
            "[composio] sync pass stopped short of the end; the next run resumes"
        );
    }

    tracing::debug!(
        toolkit = %toolkit,
        stage = ?response.stage,
        pages_read = response.pages_read,
        records_skipped = response.records_skipped,
        written = outcome.written,
        already_ingested = outcome.already_ingested,
        "[composio] sync pass ingested"
    );
    Ok(SyncPassOutcome {
        records_read: count,
        written: outcome.written,
        already_ingested: outcome.already_ingested,
        more_pending: !response.batch.complete,
    })
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
