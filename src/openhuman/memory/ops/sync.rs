//! Memory-sync RPC handlers and ingestion-status reporting.
//!
//! Sync RPCs publish `DomainEvent::MemorySyncRequested` on the global event
//! bus — they are fire-and-forget hooks for future ingestion subscribers.
//!
//! # The engine call that was here is gone (openhuman#5560)
//!
//! This section used to name one blocker: `spawn_manual_sync` reached
//! `tinycortex::run_composio_connection`, whose
//! `run_composio_connection_with_caps` opens with
//! `global::client_if_ready().ok_or(… "memory client is not ready")` — so with
//! the in-process engine gone, every target failed and the handler emitted
//! `MemorySyncStage::Failed` per connection. Loud, but wrong.
//!
//! [`spawn_manual_sync`] runs the pass through
//! [`integrations::composio::ops::run_sync_pass`](crate::openhuman::integrations::composio::ops::run_sync_pass)
//! now — the tinyconnectors module for the fetch, the bound driver's
//! `MemorySourceSink` for the write. The one thing this handler still does for
//! itself is resolve the binding *before* the spawn, so a driver that accepts
//! no source items is an error the caller sees rather than a status line a
//! detached task emits into a channel nobody is reading yet.
//!
//! # `memory_ingestion_status` was the quiet one, and it is fixed
//!
//! It used to read the in-process engine's live `IngestionState` through
//! `global::client_if_ready()`, whose `None` arm answered "idle, queue empty" —
//! indistinguishable from a healthy store with nothing to do. Once the second
//! engine stopped booting, that arm became the *only* arm: the RPC reported
//! permanent idle regardless of reality, and the Memory panel's ingestion
//! indicator never lit up again. The frontend polls this every 1.5–4s
//! (`useMemoryIngestionStatus`, `useBackgroundActivity`), so the wrong answer
//! was on screen continuously.
//!
//! It now reads `MemoryMaintenance::queue_stats` off the bound driver, which is
//! the contract's own queue telemetry and needs no new bus member. See
//! `ingestion_status_for_config` for exactly which fields survived that move
//! and which the contract does not carry.

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::memory::sync::composio;
use crate::rpc::RpcOutcome;
use tinymemory_api::sync_events::{emit_sync_stage, MemorySyncStage, MemorySyncTrigger};

/// Parameters for `memory_sync_channel`.
#[derive(Debug, serde::Deserialize)]
pub struct SyncChannelParams {
    pub channel_id: String,
}

/// Result returned by `memory_sync_channel`.
#[derive(Debug, serde::Serialize)]
pub struct SyncChannelResult {
    pub requested: bool,
    pub channel_id: String,
}

/// Result returned by `memory_sync_all`.
#[derive(Debug, serde::Serialize)]
pub struct SyncAllResult {
    pub requested: bool,
}

/// Result returned by `memory_ingestion_status` — the public RPC shape, kept
/// deliberately separate from whatever the driver answers so an internal rename
/// cannot break the wire contract.
///
/// **Five fields are permanently `None` since the move onto the contract**
/// (`current_document_id`, `current_title`, `current_namespace`,
/// `last_document_id`, `last_success`) — see `ingestion_status_for_config`,
/// which documents the reduction field by field. They stay on the struct because
/// `MemoryControls`, `OverviewPanel` and `useBackgroundActivity` decode this
/// shape and every one of them is `skip_serializing_if = "Option::is_none"`
/// already, so an absent field is a shape the frontend has always handled.
/// Removing them would be a wire change; leaving them is not a promise that
/// they are filled.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct IngestionStatusResult {
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_document_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_namespace: Option<String>,
    pub queue_depth: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_document_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success: Option<bool>,
}

/// Request a memory sync for a specific channel.
///
/// Ingestion in OpenHuman is listener/webhook-driven — there is no per-provider
/// pull mechanism yet. This RPC publishes `DomainEvent::MemorySyncRequested` so
/// that future ingestion subscribers can react to an explicit pull request.
/// The event is fire-and-forget; the caller receives confirmation that the
/// request was published, not that ingestion ran.
pub async fn memory_sync_channel(
    params: SyncChannelParams,
) -> Result<RpcOutcome<SyncChannelResult>, String> {
    // `channel_id` is a user/context identifier — keep it out of normal logs.
    tracing::info!("[memory.sync] memory_sync_channel: entry");
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::MemorySyncRequested {
        channel_id: Some(params.channel_id.clone()),
    });
    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Requested,
        None,
        Some(&params.channel_id),
        Some("channel-targeted sync requested".to_string()),
        None, // channel-level sync — not a memory-source row
    );
    let channel_id_for_spawn = params.channel_id.clone();
    tokio::spawn(async move {
        if let Err(e) = spawn_manual_sync(Some(channel_id_for_spawn)).await {
            tracing::warn!(error = %e, "[memory.sync] background channel sync failed");
        }
    });
    tracing::debug!("[memory.sync] memory_sync_channel: MemorySyncRequested published");
    Ok(RpcOutcome::new(
        SyncChannelResult {
            requested: true,
            channel_id: params.channel_id,
        },
        vec![],
    ))
}

/// Request a memory sync for all channels.
///
/// Publishes `DomainEvent::MemorySyncRequested { channel_id: None }` on the
/// global event bus. No consumers exist yet — this is a hook for future
/// ingestion subscribers.
pub async fn memory_sync_all() -> Result<RpcOutcome<SyncAllResult>, String> {
    tracing::info!("[memory.sync] memory_sync_all: entry");
    crate::core::bus::BUS
        .publish(crate::core::events::DomainEvent::MemorySyncRequested { channel_id: None });
    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Requested,
        None,
        None,
        Some("global sync requested".to_string()),
        None, // global sync — not a memory-source row
    );
    tokio::spawn(async move {
        if let Err(e) = spawn_manual_sync(None).await {
            tracing::warn!(error = %e, "[memory.sync] background global sync failed");
        }
    });
    tracing::debug!("[memory.sync] memory_sync_all: MemorySyncRequested(all) published");
    Ok(RpcOutcome::new(SyncAllResult { requested: true }, vec![]))
}

async fn spawn_manual_sync(requested_connection: Option<String>) -> Result<(), String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let targets = match composio::list_sync_targets(&config).await {
        Ok(t) => t,
        Err(e) => {
            tracing::debug!(
                error = %e,
                "[memory.sync] no composio sync targets available — proceeding with empty list"
            );
            Vec::new()
        }
    };

    let targets: Vec<composio::SyncTarget> = match requested_connection.as_deref() {
        Some(requested) => targets
            .into_iter()
            .filter(|target| target.connection_id == requested || target.toolkit == requested)
            .collect(),
        None => targets,
    };

    if let Some(requested) = requested_connection.as_deref() {
        if targets.is_empty() {
            emit_sync_stage(
                MemorySyncTrigger::Manual,
                MemorySyncStage::Failed,
                None,
                Some(requested),
                Some("no active provider-backed sync target matched request".to_string()),
                None, // channel-level sync — not a memory-source row
            );
            return Err(format!(
                "memory sync: no active provider-backed target matched `{requested}`"
            ));
        }
    }

    // Resolved BEFORE the spawn so a missing driver is an error the caller
    // sees, not a status line the spawned task emits into a channel nobody is
    // reading yet. `run_sync_pass` re-resolves its own binding per target
    // (it takes `&Config`, not a binding), so this check exists purely to
    // fail fast on the same "does the bound driver accept source items"
    // question it would otherwise only discover after the spawn.
    let binding = crate::openhuman::memory::binding::for_config(&config)?;
    if binding.provider().as_sources().is_none() {
        return Err(format!(
            "the bound memory driver '{}' does not accept source items",
            binding.driver_id()
        ));
    }

    tokio::spawn(async move {
        for target in targets {
            emit_sync_stage(
                MemorySyncTrigger::Manual,
                MemorySyncStage::Fetching,
                Some(&target.toolkit),
                Some(&target.connection_id),
                Some("provider sync started".to_string()),
                None, // provider-level composio sync — not a memory-source row
            );

            // Through the tinyconnectors module and the bound driver's
            // `MemorySourceSink`, not the (now permanently refusing) engine
            // seam — see `memory::sync::composio`'s module docs.
            let outcome = crate::openhuman::integrations::composio::ops::run_sync_pass(
                &config,
                &target.toolkit,
                &target.connection_id,
                "manual",
            )
            .await;
            match outcome {
                Ok(pass) => {
                    emit_sync_stage(
                        MemorySyncTrigger::Manual,
                        MemorySyncStage::Completed,
                        Some(&target.toolkit),
                        Some(&target.connection_id),
                        Some(format!(
                            "provider sync completed items_ingested={} written={} \
                             already_ingested={}",
                            pass.records_read, pass.written, pass.already_ingested
                        )),
                        None, // provider-level composio sync — not a memory-source row
                    );
                }
                Err(error) => {
                    emit_sync_stage(
                        MemorySyncTrigger::Manual,
                        MemorySyncStage::Failed,
                        Some(&target.toolkit),
                        Some(&target.connection_id),
                        Some(error.clone()),
                        None, // provider-level composio sync — not a memory-source row
                    );
                    tracing::warn!(
                        toolkit = %target.toolkit,
                        connection_id = %target.connection_id,
                        error = %error,
                        "[memory.sync] provider sync failed"
                    );
                }
            }
        }
    });

    Ok(())
}

/// Returns the current memory-ingestion status: whether the driver's queue is
/// working, how much is waiting, and when it last settled a job. Read-only,
/// safe to poll.
pub async fn memory_ingestion_status() -> Result<RpcOutcome<IngestionStatusResult>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let status = ingestion_status_for_config(&config).await?;
    Ok(RpcOutcome::new(status, vec![]))
}

/// The queue half of [`memory_ingestion_status`], against an explicit config.
///
/// Split out so the mapping below is testable against a bound driver without
/// the handler's ambient `Config::load_or_init` — the same shape
/// `read_rpc::chunks` uses for its own paged reads.
///
/// # What moved, and what the contract does not carry
///
/// This was `global::client_if_ready().ingestion_state().snapshot()` — an
/// in-process counter owned by the engine this host no longer boots. The
/// replacement is
/// [`MemoryMaintenance::queue_stats`](crate::openhuman::memory::api::provider::MemoryMaintenance::queue_stats),
/// the contract's own queue
/// telemetry, so it works against *whichever* driver is bound rather than only
/// against an engine living in this address space.
///
/// | RPC field | Source | Note |
/// | --- | --- | --- |
/// | `running` | `QueueStats::running > 0` | jobs a worker currently holds |
/// | `queue_depth` | `QueueStats::ready` | jobs waiting; the running one is counted by `running`, exactly as the engine counter split them |
/// | `last_completed_at` | `QueueStats::last_completed_ms` | when the queue last settled a job |
///
/// **The reduction, stated rather than hidden.** `current_document_id`,
/// `current_title`, `current_namespace`, `last_document_id` and `last_success`
/// have no contract equivalent and are left `None`. `QueueStats` is counts, not
/// job identity: it can say a worker is busy, not *what* it is busy with. That
/// is a narrower answer than the engine's snapshot gave — and a strictly better
/// one than what shipped, because since the second engine stopped booting those
/// fields were `None` **and** `running`/`queue_depth` were falsely zero. Nothing
/// is substituted for them: `latest_queue_failure()` could be squinted at to
/// synthesise a `last_success`, and it would be a different question's answer
/// (the newest *terminal failure*, not the newest job's outcome).
///
/// Widening this back out is a contract member and a `tinymemory` release, not
/// a host change.
///
/// # Degrade
///
/// A driver error propagates — reporting "idle" because the driver failed is
/// the exact bug this replaced. A driver that does not serve `Maintenance` at
/// all reports zeros with the driver named in the log, matching its siblings in
/// `memory::tree::tree::rpc`: it has no queue to be behind on.
async fn ingestion_status_for_config(config: &Config) -> Result<IngestionStatusResult, String> {
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let Some(maintenance) = binding.provider().as_maintenance() else {
        log::debug!(
            "[memory.sync] ingestion_status: driver '{}' does not serve Maintenance; reporting idle",
            binding.driver_id()
        );
        return Ok(IngestionStatusResult::default());
    };

    // `None` counts every job kind. The engine counter this replaces was not
    // narrowed to the ingest kind either — it was bumped on every submit — so
    // narrowing here would silently shrink a number the Memory panel already
    // displays.
    let queue = maintenance
        .queue_stats(None)
        .await
        .map_err(|e| format!("queue_stats: {e}"))?;

    log::debug!(
        "[memory.sync] ingestion_status: driver='{}' running={} ready={} eligible_now={}",
        binding.driver_id(),
        queue.running,
        queue.ready,
        queue.eligible_now
    );

    Ok(IngestionStatusResult {
        running: queue.running > 0,
        current_document_id: None,
        current_title: None,
        current_namespace: None,
        queue_depth: usize::try_from(queue.ready).unwrap_or(usize::MAX),
        last_completed_at: queue.last_completed_ms,
        last_document_id: None,
        last_success: None,
    })
}

#[cfg(test)]
#[path = "sync_tests.rs"]
mod tests;
