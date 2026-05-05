//! Memory-sync RPC handlers and ingestion-status reporting.
//!
//! Sync RPCs publish `DomainEvent::MemorySyncRequested` on the global event
//! bus — they are fire-and-forget hooks for future ingestion subscribers.

use crate::rpc::RpcOutcome;

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

/// Result returned by `memory_ingestion_status`. Mirrors
/// [`crate::openhuman::memory::IngestionStatusSnapshot`] but is the public RPC
/// shape — the indirection keeps internal renames from breaking the wire
/// contract.
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
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::MemorySyncRequested {
            channel_id: Some(params.channel_id.clone()),
        },
    );
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
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::MemorySyncRequested { channel_id: None },
    );
    tracing::debug!("[memory.sync] memory_sync_all: MemorySyncRequested(all) published");
    Ok(RpcOutcome::new(SyncAllResult { requested: true }, vec![]))
}

/// Returns the current memory-ingestion status: whether a job is running, the
/// in-flight document, queue depth, and the most recent completion. Read-only,
/// safe to poll.
pub async fn memory_ingestion_status() -> Result<RpcOutcome<IngestionStatusResult>, String> {
    let snapshot = match crate::openhuman::memory::global::client_if_ready() {
        Some(c) => c.ingestion_state().snapshot(),
        // Memory not yet initialised — report idle, no in-flight job.
        None => Default::default(),
    };
    Ok(RpcOutcome::new(
        IngestionStatusResult {
            running: snapshot.running,
            current_document_id: snapshot.current_document_id,
            current_title: snapshot.current_title,
            current_namespace: snapshot.current_namespace,
            queue_depth: snapshot.queue_depth,
            last_completed_at: snapshot.last_completed_at,
            last_document_id: snapshot.last_document_id,
            last_success: snapshot.last_success,
        },
        vec![],
    ))
}
