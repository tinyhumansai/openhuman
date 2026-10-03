//! Syncing a connected account into memory.
//!
//! Reading the account is the connector module's job — it holds the
//! credential, the provider registry and the paging cursors. Storing what it
//! read is memory's: each pass hands the batch to
//! [`crate::memory::sources::composio::store_records`], which turns every
//! record into a document on the bound engine. Neither half knows about the
//! other: the module never reaches memory, and memory never sees a Composio
//! credential.
//!
//! Two callers drive passes: `openhuman.composio_sync` (one connection, on
//! demand, in the background — see [`composio_sync`]) and memory's own
//! scheduled Composio sources (`memory::sources::composio::sync_toolkit`),
//! which call [`run_sync_pass`] directly.

use crate::config::Config;
use crate::core::Outcome;
use crate::memory::engine::BoundEngine;
use crate::memory::sources::composio::{source_id_for_toolkit, store_records};

use super::super::module_client::{self as connectors, methods};
use super::super::providers::{SyncOutcome, SyncReason};
use super::connections::resolve_toolkit_for_connection;
use super::error_utils::{report_composio_op_error, OpResult};
use super::pass_failure::{pass_failure, retry_delay};
use tinyconnectors_bus::records::{ConnectorSyncRequest, ConnectorSyncResponse};

/// Per-pass item budget handed to the connector's `Sync` member.
///
/// One pass is one budgeted slice of the account; [`MAX_PASSES`] bounds a
/// `composio_sync` run at 10k items per click.
pub const SYNC_PASS_MAX_ITEMS: usize = 200;

/// Most passes one `composio_sync` run makes before it stops; the next run
/// resumes from the module's cursor.
const MAX_PASSES: usize = 50;

/// What one [`run_sync_pass`] call did.
#[derive(Debug, Clone, Default)]
pub struct SyncPassOutcome {
    /// Records the module returned in this pass.
    pub records_read: usize,
    /// Of those, how many memory stored (empty records are skipped).
    pub written: u64,
    /// Whether the module has more to read — the caller decides whether to
    /// call again.
    pub more_pending: bool,
    /// Why the connector stopped this pass on an error (openhuman#6255). The
    /// counts above still stand; the caller ends or retries the run.
    pub failure: Option<String>,
}

/// `openhuman.composio_sync` — read one connected account into memory.
///
/// Returns as soon as the run is *started*: a full sync is minutes of paging,
/// and the RPC caller is a UI button. The engine is resolved up front so
/// memory being off is an error the caller sees, not a log line in a detached
/// task. Items are filed under the toolkit's configured Composio source, or
/// `composio:<toolkit>` when none is configured.
pub async fn composio_sync(
    config: &Config,
    connection_id: &str,
    reason: Option<String>,
) -> OpResult<Outcome<SyncOutcome>> {
    let reason = parse_sync_reason(reason.as_deref())?;
    tracing::debug!(
        connection_id = %connection_id,
        reason = reason.as_str(),
        "[composio] rpc sync (spawned)"
    );
    let toolkit = resolve_toolkit_for_connection(config, connection_id).await?;
    let bound = crate::memory::engine::resolve(config)
        .engine()
        .map_err(String::from)?;
    let source_id = source_id_for_toolkit(config, &toolkit);

    let started_at_ms = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
    )
    .unwrap_or(u64::MAX);

    let run = SyncRun {
        config: config.clone(),
        bound,
        toolkit: toolkit.clone(),
        connection_id: connection_id.to_string(),
        source_id,
        reason: reason.as_str().to_string(),
    };
    tokio::spawn(run.drive());

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
    Ok(Outcome::new(outcome, vec![summary]))
}

/// One background `composio_sync` run.
struct SyncRun {
    config: Config,
    bound: BoundEngine,
    toolkit: String,
    connection_id: String,
    source_id: String,
    reason: String,
}

impl SyncRun {
    /// Runs passes until the connector reports the end, a pass reads nothing,
    /// or [`MAX_PASSES`] is reached. A pass the connector reports as failed is
    /// retried on [`retry_delay`]'s schedule, then ends the run.
    async fn drive(self) {
        let started = std::time::Instant::now();
        let mut total_written: u64 = 0;
        let mut passes = 0usize;
        let mut failed_attempts = 0u32;
        let outcome: Result<bool, String> = loop {
            passes += 1;
            let pass = match run_sync_pass(
                &self.config,
                &self.bound,
                &self.toolkit,
                &self.connection_id,
                &self.source_id,
                &self.reason,
                SYNC_PASS_MAX_ITEMS,
            )
            .await
            {
                Ok(pass) => pass,
                Err(error) => break Err(error),
            };
            total_written = total_written.saturating_add(pass.written);
            if let Some(reason) = pass.failure {
                failed_attempts += 1;
                let Some(delay) = retry_delay(failed_attempts) else {
                    break Err(reason);
                };
                tracing::info!(
                    toolkit = %self.toolkit,
                    connection_id = %self.connection_id,
                    attempt = failed_attempts,
                    retry_in_secs = delay.as_secs(),
                    "[composio] connector sync pass failed; retrying"
                );
                tokio::time::sleep(delay).await;
                continue;
            }
            failed_attempts = 0;
            tracing::debug!(
                toolkit = %self.toolkit,
                connection_id = %self.connection_id,
                pass = passes,
                records_read = pass.records_read,
                written = pass.written,
                more_pending = pass.more_pending,
                "[composio] background sync pass ok"
            );
            if !pass.more_pending || pass.records_read == 0 || passes >= MAX_PASSES {
                break Ok(pass.more_pending);
            }
        };
        match outcome {
            Ok(more_pending) => tracing::info!(
                toolkit = %self.toolkit,
                connection_id = %self.connection_id,
                passes,
                written = total_written,
                more_pending,
                elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                "[composio] background sync complete"
            ),
            Err(error) => {
                report_composio_op_error("sync", &anyhow::anyhow!("{error}"));
                tracing::warn!(
                    toolkit = %self.toolkit,
                    connection_id = %self.connection_id,
                    written = total_written,
                    error = %error,
                    "[composio] background sync failed"
                );
            }
        }
    }
}

/// Read one pass of a connection through the module and store what it
/// returns under `source_id`.
///
/// A pass the connector reports as failed still stores the records it read
/// and comes back as `Ok`, with [`SyncPassOutcome::failure`] set and nothing
/// pending: whether to ask again is the caller's decision (openhuman#6255).
///
/// # Errors
///
/// The connector call fails, or memory cannot store the batch.
pub async fn run_sync_pass(
    config: &Config,
    bound: &BoundEngine,
    toolkit: &str,
    connection_id: &str,
    source_id: &str,
    reason: &str,
    pass_budget: usize,
) -> Result<SyncPassOutcome, String> {
    // Sync pages inside the call; the default 30s bus deadline reported
    // failure on runs the module then finished successfully.
    let response = connectors::call_slow::<_, ConnectorSyncResponse>(
        config,
        methods::SYNC,
        ConnectorSyncRequest {
            toolkit: toolkit.to_string(),
            connection_id: Some(connection_id.to_string()),
            reason: Some(reason.to_string()),
            // One pass is one budgeted slice: complete=false at the budget →
            // more_pending → the caller resumes from the module's cursor.
            max_items: Some(pass_budget),
            ..ConnectorSyncRequest::default()
        },
    )
    .await?;

    // A failed pass keeps what it read but is not "more pending" (#6255).
    let failure = pass_failure(&response, toolkit, connection_id);
    let more_pending = failure.is_none() && !response.batch.complete;

    let records_read = response.batch.records.len();
    let written = if records_read == 0 {
        0
    } else {
        store_records(
            bound,
            toolkit,
            connection_id,
            source_id,
            &response.batch.records,
        )
        .await
        .map_err(|error| format!("storing {toolkit} records failed: {}", String::from(error)))?
    };

    if more_pending {
        // The module's note says why (today's request budget, typically).
        tracing::info!(
            toolkit = %toolkit,
            note = response.message.as_deref().unwrap_or(""),
            "[composio] sync pass stopped short of the end; the next run resumes"
        );
    }
    tracing::debug!(
        toolkit = %toolkit,
        stage = ?response.stage,
        pages_read = response.pages_read,
        records_skipped = response.records_skipped,
        records_read,
        written,
        "[composio] sync pass stored"
    );
    Ok(SyncPassOutcome {
        records_read,
        written,
        more_pending,
        failure,
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
