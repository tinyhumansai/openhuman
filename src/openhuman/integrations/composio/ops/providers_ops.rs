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

/// Per-pass item budget handed to the connector's `Sync` member.
///
/// One pass is one budgeted slice of the account; the pass loop multiplies it
/// by `MAX_PASSES` into a worst case of 10k items per click. 200, not 500: a
/// pass reaches the memory module as ONE `AcceptSourceItems` call that embeds
/// each document in turn (~1.7 s each), and 500 ran ~14 min into the host's
/// 15-minute deadline, logging a false `initial sync failed` (openhuman#6025).
pub const SYNC_PASS_MAX_ITEMS: usize = 200;

/// The next pass's item budget, or `None` when the configured per-run cap is
/// spent and the run should end.
///
/// Pure on purpose: this is the arithmetic the budgeted loop stands on —
/// unlimited slices at the pass ceiling, a cap slices to `min(remaining,
/// ceiling)`, an exhausted cap stops — and a unit test can hold it still.
pub(crate) fn next_pass_budget(source_max_items: Option<u32>, total_written: u64) -> Option<usize> {
    match source_max_items {
        None => Some(SYNC_PASS_MAX_ITEMS),
        Some(cap) => {
            let remaining = u64::from(cap).saturating_sub(total_written);
            if remaining == 0 {
                None
            } else {
                Some(
                    usize::try_from(remaining)
                        .unwrap_or(usize::MAX)
                        .min(SYNC_PASS_MAX_ITEMS),
                )
            }
        }
    }
}

/// The `completed` stage's detail string.
///
/// A parse contract, not prose: the Sources UI extracts the count with
/// `/ingested\s+(\d+)\s+item/i` and falls back to a generic "up to date"
/// when it cannot (#3295). Pinned by a unit test against that exact pattern.
///
/// `note` is what the module said about a run that stopped short — today's
/// request budget being spent, above all. It rides *after* the count, never
/// inside it, so the regex keeps matching and everything past the count is
/// free text the UI can show. Without it a spent budget wrote zero items and
/// read back as "Up to date", the opposite of what happened.
pub(crate) fn completed_sync_detail(
    total_written: u64,
    more_pending: bool,
    note: Option<&str>,
) -> String {
    let mut detail = if more_pending {
        format!("ingested {total_written} item(s), more pending — Sync again to continue")
    } else {
        format!("ingested {total_written} item(s)")
    };
    if let Some(note) = note.map(str::trim).filter(|note| !note.is_empty()) {
        detail.push_str("; ");
        detail.push_str(note);
    }
    detail
}

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
    composio_sync_for_source(config, connection_id, reason, None).await
}

/// [`composio_sync`], carrying the originating memory-source row id so the
/// background task can publish `MemorySyncStageChanged` events the Brain
/// sources row keys its per-row indicator on (#3295). The composio path never
/// emitted them — the driver pipeline's events come from the module host
/// bridge, which this path does not cross — so a successful sync left the row
/// on "Syncing" forever, waiting for a terminal stage that never arrived.
pub async fn composio_sync_for_source(
    config: &Config,
    connection_id: &str,
    reason: Option<String>,
    source_id: Option<String>,
) -> OpResult<RpcOutcome<SyncOutcome>> {
    composio_sync_budgeted(config, connection_id, reason, source_id, None).await
}

/// [`composio_sync_for_source`] with the source's configured per-run ingest
/// cap. `None` preserves unlimited: the loop still slices the account into
/// 200-item passes, but stops only when the connector reports the end.
pub async fn composio_sync_budgeted(
    config: &Config,
    connection_id: &str,
    reason: Option<String>,
    source_id: Option<String>,
    source_max_items: Option<u32>,
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

    let toolkit_for_task = toolkit.clone();
    let connection_for_task = connection_id.to_string();
    let reason_for_task = reason.as_str().to_string();
    let source_for_task = source_id.clone();

    let trigger_for_task = reason.as_str().to_string();
    let publish_stage = move |stage: &str, detail: Option<String>| {
        crate::core::bus::BUS.publish(crate::core::events::DomainEvent::MemorySyncStageChanged {
            trigger: trigger_for_task.clone(),
            stage: stage.to_string(),
            provider: Some(toolkit_for_task.clone()),
            connection_id: Some(connection_for_task.clone()),
            detail,
            source_id: source_for_task.clone(),
        });
    };

    let toolkit_for_log = toolkit.clone();
    let connection_for_log = connection_id.to_string();
    let config_for_run = config.clone();

    // A connector page is one `run_sync_pass`; `more_pending` means the run
    // stopped mid-account and "the next run resumes". The button's terminal
    // stage must describe the whole user intent, not one page — emitting
    // `completed` with pages unfetched cleared the row while records remained
    // (review finding on this PR) — so loop passes until the connector reports
    // the end, bounded so an upstream that never completes cannot pin this
    // task forever. The bound is generous: 50 passes ≈ 10,000 records per click.
    const MAX_PASSES: usize = 50;

    // Published before the spawn: the row's "running" exists before the RPC
    // returns, and the bus preserves publisher order, so completed can never
    // overtake it (review question on ordering).
    publish_stage("running", None);

    tokio::spawn(async move {
        let mut total_written: u64 = 0;
        let mut passes = 0usize;
        let mut more_pending = false;
        // The module's own account of a pass that stopped short (the day's
        // request budget, typically). The last pass's word wins, present or
        // absent: it describes the state the run ended in, and a pass that
        // completes cleanly must not inherit the note of an earlier one.
        let mut stop_note: Option<String> = None;
        let outcome = loop {
            passes += 1;
            // The source's configured per-run cap wins over the pass ceiling:
            // a budget of 200 means one 200-item pass, not 50 passes of 500
            // (review finding). `None` from the arithmetic = cap spent, and a
            // spent cap is a completed run, not a failed one.
            let Some(pass_budget) = next_pass_budget(source_max_items, total_written) else {
                break Ok(());
            };
            match run_sync_pass(
                &config_for_run,
                &toolkit_for_log,
                &connection_for_log,
                &reason_for_task,
                pass_budget,
            )
            .await
            {
                Ok(pass) => {
                    total_written = total_written.saturating_add(u64::from(pass.written));
                    more_pending = pass.more_pending;
                    stop_note = pass
                        .message
                        .as_deref()
                        .map(str::trim)
                        .filter(|message| !message.is_empty())
                        .map(str::to_string);
                    tracing::info!(
                        toolkit = %toolkit_for_log,
                        connection_id = %connection_for_log,
                        pass = passes,
                        items_ingested = pass.records_read,
                        written = pass.written,
                        already_ingested = pass.already_ingested,
                        more_pending = pass.more_pending,
                        "[composio] background sync pass ok"
                    );
                    if !pass.more_pending {
                        break Ok(());
                    }
                    if passes >= MAX_PASSES {
                        // The cap is pacing, not failure: everything fetched is
                        // ingested and the next click resumes from the cursor.
                        // Routing this through the failed stage put an error
                        // toast on a working feature (review finding); the row
                        // settles with the honest count and the remainder named.
                        break Ok(());
                    }
                    // A heartbeat between passes. The module reports only a
                    // run's first stage and its last, and a run of many passes
                    // can outlive the host's memory of it: `memory::sync_activity`
                    // forgets a run after a bounded silence, because the bus can
                    // lose a terminal stage (openhuman#6019). One pass is bounded
                    // by the slow call's deadline, so a stage per pass keeps the
                    // run alive for exactly as long as it runs — and tells the
                    // row where it is.
                    publish_stage(
                        "running",
                        Some(format!(
                            "pass {passes} done, {total_written} item(s) so far"
                        )),
                    );
                }
                Err(error) => break Err(error),
            }
        };
        match outcome {
            Ok(()) => {
                // The detail is a parse contract, not prose: the Sources UI
                // extracts the count with `/ingested\s+(\d+)\s+item/i` and
                // falls back to a generic "up to date" when it cannot (#3295).
                publish_stage(
                    "completed",
                    Some(completed_sync_detail(
                        total_written,
                        more_pending,
                        stop_note.as_deref(),
                    )),
                );
            }
            Err(error) => {
                report_composio_op_error("sync", &anyhow::anyhow!("{error}"));
                tracing::warn!(
                    toolkit = %toolkit_for_log,
                    connection_id = %connection_for_log,
                    error = %error,
                    "[composio] background sync failed"
                );
                publish_stage("failed", Some(error.clone()));
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
/// A superset of the `usize` the caller inside this file needs, so the
/// single-call entry points behind `pass_budget::run_sync_within_budget` can
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
    /// What the module said about a run that stopped short — today's request
    /// budget being spent, above all. Carried so the completed-stage detail
    /// can say *why* zero items arrived instead of reading as "up to date".
    pub message: Option<String>,
}

/// Read one connection through the module and ingest what it returns.
///
/// The two halves are deliberately not interleaved with retries or partial
/// commits: the module already decides what a page is and where the cursor
/// stands, and re-deciding that here would give the run two opinions about
/// what has been read.
///
/// `pub(crate)` — also driven by `pass_budget::run_sync_within_budget` for
/// the entry points that sync once per invocation (periodic tick, manual
/// provider sync, `connection_created`, the Slack ingest RPC), which repeat
/// this pass within one call's item budget and await it synchronously rather
/// than firing it into a background task.
pub(crate) async fn run_sync_pass(
    config: &Config,
    toolkit: &str,
    connection_id: &str,
    reason: &str,
    pass_budget: usize,
) -> Result<SyncPassOutcome, String> {
    // Sync pages the whole account inside the call; the default 30s bus
    // deadline reported failure on runs the module then finished successfully.
    // The per-source "Sync depth (days)" cap. Until contract 1.8 gave the
    // request a field for it, the setting reached the log and nothing else;
    // the module now turns it into Gmail's own `after:` search term, so a
    // bounded first sync costs one page of recent mail rather than a walk
    // through the years. `None` reads unbounded, as every earlier release did.
    let depth_days = source_sync_depth_days(config, toolkit, connection_id);

    let response = connectors::call_slow::<_, ConnectorSyncResponse>(
        config,
        methods::SYNC,
        ConnectorSyncRequest {
            toolkit: toolkit.to_string(),
            connection_id: Some(connection_id.to_string()),
            reason: Some(reason.to_string()),
            // One pass is one budgeted slice, not "the whole account": without
            // a budget the module pages until its own limits and the caller's
            // pass loop bounds nothing (review finding). complete=false at the
            // budget → more_pending → the loop (or the next click) resumes.
            max_items: Some(pass_budget),
            depth_days,
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
            message: response.message,
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

    // openhuman#6007: seal this source's summary tree once the pass has written
    // something.
    //
    // Tree ingest writes its L0 chunk rows synchronously, but the Memory Tree
    // graph and tree-backed recall read *sealed* summaries. A buffer seals on
    // the 50k-token budget or, failing that, on the seven-day
    // `DEFAULT_FLUSH_AGE_SECS` force-flush — and nothing on the sync path ever
    // asked for one. So an incremental run of a handful of messages stayed
    // under the budget and its memories were invisible for up to a week, while
    // the source row reported them ingested the entire time.
    //
    // `flush_source_tree` rather than `flush_pending`, for two reasons that
    // are the whole of tinyhumansai/tinymemory#135:
    //
    // - It **bypasses the job queue**. `flush_pending` enqueues, and that queue
    //   dedupes on `date + hour/3`, so a request landing while an earlier flush
    //   was already running was suppressed — and if that flush had already
    //   walked past this buffer, nothing remained queued for it. The periodic
    //   tick does not rescue those: it enqueues the default seven-day age and
    //   steps straight over a buffer written minutes ago. With no queue there
    //   is no dedupe key and nothing to be suppressed against.
    // - It seals **one scope**. `flush_pending` is workspace-wide, so a Gmail
    //   sync also sealed whatever a folder or `github_repo` source had left
    //   pending — unrelated trees nudged by an unrelated event.
    //
    // The scope is built exactly as the ingest funnel builds `path_scope`
    // (`{toolkit}:{connection_id}`, toolkit lowercased), because it has to name
    // the same tree the items were filed under. A drift here seals nothing and
    // says `Ok(0)` while doing it.
    //
    // Here rather than at the callers: `run_sync_pass` has three of them (the
    // budgeted loop in this file, the Slack trigger RPC, and the sync bus), and
    // one rule spread across call sites is exactly what caused #6007 — two
    // would have got the flush and the third would have been forgotten.
    //
    // Best-effort by construction. The records are committed; an unsealed
    // buffer is a delay, not a loss. Nothing here may turn a successful sync
    // into a failed one. Safe to call unconditionally, too: the contract makes
    // an empty scope `Ok(0)` rather than an error.
    if outcome.written > 0 {
        let scope = format!(
            "{}:{}",
            toolkit.trim().to_ascii_lowercase(),
            connection_id.trim()
        );
        match binding.provider().as_tree() {
            Some(tree) => match tree.flush_source_tree(&scope).await {
                Ok(seals) => tracing::debug!(
                    scope = %scope,
                    seals_fired = seals,
                    "[composio] source tree sealed after sync pass"
                ),
                Err(error) => tracing::warn!(
                    scope = %scope,
                    error = %error,
                    "[composio] source tree could not be sealed; ingested records stay \
                     unsealed until a later seal reaches them"
                ),
            },
            None => tracing::debug!(
                driver = %binding.driver_id(),
                "[composio] bound driver serves no Tree; skipping the post-sync seal"
            ),
        }
    }

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
        message: response.message,
    })
}

/// The per-source "Sync depth (days)" cap for one connection, from the
/// memory-sources registry the pass's own `config` names.
///
/// Resolved here rather than threaded through every caller because there are
/// five of them (the row button, All In, the periodic loop, the connection
/// bootstrap, Slack's own RPC), and `max_items` already showed what happens
/// when each open-codes the same rule: two of the five disagreed
/// (openhuman#6007). Read through `config`, not the process environment: a
/// pass is bound to one workspace, and the global registry path would answer a
/// caller bound to workspace B with workspace A's rows — the cross-workspace
/// leak the registry's `_in` variants exist to prevent. `None` when the row is
/// missing, carries no cap, or the registry cannot be read — each means "no
/// lower bound", which is what every release before the field existed did, so
/// a registry hiccup degrades to the old behaviour rather than to a failed
/// sync.
fn source_sync_depth_days(config: &Config, toolkit: &str, connection_id: &str) -> Option<u32> {
    let sources = match crate::openhuman::memory::sources::registry::list_sources_in(config) {
        Ok(sources) => sources,
        Err(error) => {
            tracing::warn!(
                toolkit = %toolkit,
                connection_id = %connection_id,
                error = %error,
                "[composio] memory-sources registry unreadable for the sync depth; \
                 syncing without a lower bound"
            );
            return None;
        }
    };
    pick_source_sync_depth_days(
        sources
            .iter()
            .filter(|source| source.kind == crate::openhuman::memory::sources::SourceKind::Composio)
            .map(|source| {
                (
                    source.toolkit.as_deref(),
                    source.connection_id.as_deref(),
                    source.sync_depth_days,
                )
            }),
        toolkit,
        connection_id,
    )
}

/// The cap of the row matching `toolkit` and `connection_id`, if any.
///
/// Matched the way the engine keys the rows — toolkit case-insensitively and
/// trimmed, connection trimmed — and a cap of zero reads as none: the settings
/// field stores "unlimited" as an empty value, and a zero typed by hand would
/// otherwise ask Gmail for mail newer than today.
pub(crate) fn pick_source_sync_depth_days<'a>(
    rows: impl IntoIterator<Item = (Option<&'a str>, Option<&'a str>, Option<u32>)>,
    toolkit: &str,
    connection_id: &str,
) -> Option<u32> {
    let toolkit = toolkit.trim();
    let connection_id = connection_id.trim();
    rows.into_iter()
        .find_map(|(row_toolkit, row_connection, depth)| {
            let same_toolkit =
                row_toolkit.is_some_and(|slug| slug.trim().eq_ignore_ascii_case(toolkit));
            let same_connection = row_connection.is_some_and(|id| id.trim() == connection_id);
            (same_toolkit && same_connection)
                .then_some(depth)
                .flatten()
                .filter(|days| *days > 0)
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

/// Test window over the two-arg detail (the one-arg-era test path kept its
/// name; this avoids re-plumbing the cfg(test) re-export).
#[cfg(test)]
pub(crate) fn completed_sync_detail_for_test(total: u64, more: bool) -> String {
    completed_sync_detail(total, more, None)
}
