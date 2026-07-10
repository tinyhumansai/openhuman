//! RPC request/response types and handler implementations.

use log::debug;
use serde::{Deserialize, Serialize};

use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

use super::git_store::Ledger;
use super::ops;
use super::types::*;

// ── Request / Response types ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TakeSnapshotRequest {
    pub source_id: String,
}

#[derive(Debug, Serialize)]
pub struct TakeSnapshotResponse {
    pub snapshot: Snapshot,
}

#[derive(Debug, Deserialize)]
pub struct ListSnapshotsRequest {
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ListSnapshotsResponse {
    pub snapshots: Vec<Snapshot>,
}

#[derive(Debug, Deserialize)]
pub struct DiffRequest {
    #[serde(default)]
    pub from_snapshot_id: Option<String>,
    pub to_snapshot_id: String,
    #[serde(default)]
    pub include_text_diff: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct DiffResponse {
    pub diff: DiffResult,
}

#[derive(Debug, Deserialize)]
pub struct DiffSinceLastRequest {
    pub source_id: String,
    #[serde(default)]
    pub include_text_diff: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct DiffSinceLastResponse {
    pub diff: DiffResult,
}

#[derive(Debug, Deserialize)]
pub struct DiffSinceReadRequest {
    pub source_id: String,
    #[serde(default)]
    pub include_text_diff: Option<bool>,
    /// Advance the read marker to the head snapshot after computing the diff.
    /// Defaults to true so reading acknowledges the changes as consumed.
    #[serde(default)]
    pub commit: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct DiffSinceReadResponse {
    pub diff: DiffResult,
}

#[derive(Debug, Deserialize)]
pub struct MarkReadRequest {
    /// Sources to mark read. Omit to mark all enabled sources with a snapshot.
    #[serde(default)]
    pub source_ids: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct MarkReadResponse {
    pub marked: u64,
}

#[derive(Debug, Deserialize)]
pub struct CreateCheckpointRequest {
    pub label: String,
}

#[derive(Debug, Serialize)]
pub struct CreateCheckpointResponse {
    pub checkpoint: Checkpoint,
}

#[derive(Debug, Deserialize)]
pub struct ListCheckpointsRequest {
    #[serde(default)]
    pub limit: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ListCheckpointsResponse {
    pub checkpoints: Vec<Checkpoint>,
}

#[derive(Debug, Deserialize)]
pub struct DiffSinceCheckpointRequest {
    pub checkpoint_id: String,
    #[serde(default)]
    pub include_text_diff: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct DiffSinceCheckpointResponse {
    pub diff: CrossSourceDiff,
}

#[derive(Debug, Deserialize)]
pub struct CleanupRequest {
    pub older_than_days: u64,
}

#[derive(Debug, Serialize)]
pub struct CleanupResponse {
    pub deleted_snapshots: u64,
}

// ── Handlers ──────────────────────────────────────────────────────────

pub async fn take_snapshot_rpc(
    req: TakeSnapshotRequest,
) -> Result<RpcOutcome<TakeSnapshotResponse>, String> {
    debug!(
        "[memory_diff][rpc] take_snapshot source_id={}",
        req.source_id
    );
    let config = config_rpc::load_config_with_timeout().await?;
    let source = crate::openhuman::memory_sources::get_source(&req.source_id)
        .await?
        .ok_or_else(|| format!("source not found: {}", req.source_id))?;

    let snapshot = ops::take_snapshot(&source, &config, SnapshotTrigger::Manual).await?;
    debug!(
        "[memory_diff][rpc] take_snapshot done snapshot_id={} item_count={}",
        snapshot.id, snapshot.item_count
    );
    Ok(RpcOutcome::new(TakeSnapshotResponse { snapshot }, vec![]))
}

pub async fn list_snapshots_rpc(
    req: ListSnapshotsRequest,
) -> Result<RpcOutcome<ListSnapshotsResponse>, String> {
    debug!(
        "[memory_diff][rpc] list_snapshots source_id={:?} limit={:?}",
        req.source_id, req.limit
    );
    let config = config_rpc::load_config_with_timeout().await?;
    let workspace_dir = config.workspace_dir.clone();
    let limit = req.limit.unwrap_or(50) as u32;
    let source_id = req.source_id;

    let snapshots = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Snapshot>> {
        let ledger = Ledger::open(&workspace_dir)?;
        ledger.list_snapshots(source_id.as_deref(), limit)
    })
    .await
    .map_err(|e| format!("list_snapshots join: {e}"))?
    .map_err(|e: anyhow::Error| format!("list_snapshots: {e:#}"))?;

    debug!(
        "[memory_diff][rpc] list_snapshots returned {} snapshots",
        snapshots.len()
    );
    Ok(RpcOutcome::new(ListSnapshotsResponse { snapshots }, vec![]))
}

pub async fn diff_rpc(req: DiffRequest) -> Result<RpcOutcome<DiffResponse>, String> {
    debug!(
        "[memory_diff][rpc] diff from={:?} to={}",
        req.from_snapshot_id, req.to_snapshot_id
    );
    let config = config_rpc::load_config_with_timeout().await?;
    let diff = ops::compute_diff(
        &config,
        req.from_snapshot_id.as_deref(),
        &req.to_snapshot_id,
        req.include_text_diff.unwrap_or(false),
    )
    .await?;
    debug!(
        "[memory_diff][rpc] diff done added={} removed={} modified={}",
        diff.summary.added, diff.summary.removed, diff.summary.modified
    );
    Ok(RpcOutcome::new(DiffResponse { diff }, vec![]))
}

pub async fn diff_since_last_rpc(
    req: DiffSinceLastRequest,
) -> Result<RpcOutcome<DiffSinceLastResponse>, String> {
    debug!(
        "[memory_diff][rpc] diff_since_last source_id={}",
        req.source_id
    );
    let config = config_rpc::load_config_with_timeout().await?;
    let source = crate::openhuman::memory_sources::get_source(&req.source_id)
        .await?
        .ok_or_else(|| format!("source not found: {}", req.source_id))?;

    let diff =
        ops::diff_since_last(&source, &config, req.include_text_diff.unwrap_or(false)).await?;
    debug!(
        "[memory_diff][rpc] diff_since_last done added={} removed={} modified={}",
        diff.summary.added, diff.summary.removed, diff.summary.modified
    );
    Ok(RpcOutcome::new(DiffSinceLastResponse { diff }, vec![]))
}

pub async fn diff_since_read_rpc(
    req: DiffSinceReadRequest,
) -> Result<RpcOutcome<DiffSinceReadResponse>, String> {
    let commit = req.commit.unwrap_or(true);
    debug!(
        "[memory_diff][rpc] diff_since_read source_id={} commit={}",
        req.source_id, commit
    );
    let config = config_rpc::load_config_with_timeout().await?;
    let source = crate::openhuman::memory_sources::get_source(&req.source_id)
        .await?
        .ok_or_else(|| format!("source not found: {}", req.source_id))?;

    let diff = ops::diff_since_read(
        &source,
        &config,
        req.include_text_diff.unwrap_or(false),
        commit,
    )
    .await?;
    debug!(
        "[memory_diff][rpc] diff_since_read done added={} removed={} modified={}",
        diff.summary.added, diff.summary.removed, diff.summary.modified
    );
    Ok(RpcOutcome::new(DiffSinceReadResponse { diff }, vec![]))
}

pub async fn mark_read_rpc(req: MarkReadRequest) -> Result<RpcOutcome<MarkReadResponse>, String> {
    debug!(
        "[memory_diff][rpc] mark_read source_ids={:?}",
        req.source_ids
    );
    let config = config_rpc::load_config_with_timeout().await?;
    let marked = ops::mark_read(&config, req.source_ids).await?;
    debug!("[memory_diff][rpc] mark_read done marked={}", marked);
    Ok(RpcOutcome::new(MarkReadResponse { marked }, vec![]))
}

pub async fn create_checkpoint_rpc(
    req: CreateCheckpointRequest,
) -> Result<RpcOutcome<CreateCheckpointResponse>, String> {
    debug!("[memory_diff][rpc] create_checkpoint label={}", req.label);
    let config = config_rpc::load_config_with_timeout().await?;
    let checkpoint = ops::create_checkpoint(&req.label, &config).await?;
    debug!(
        "[memory_diff][rpc] create_checkpoint done id={} snapshots={}",
        checkpoint.id,
        checkpoint.snapshot_ids.len()
    );
    Ok(RpcOutcome::new(
        CreateCheckpointResponse { checkpoint },
        vec![],
    ))
}

pub async fn list_checkpoints_rpc(
    req: ListCheckpointsRequest,
) -> Result<RpcOutcome<ListCheckpointsResponse>, String> {
    debug!("[memory_diff][rpc] list_checkpoints limit={:?}", req.limit);
    let config = config_rpc::load_config_with_timeout().await?;
    let workspace_dir = config.workspace_dir.clone();
    let limit = req.limit.unwrap_or(20) as u32;

    let checkpoints = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Checkpoint>> {
        let ledger = Ledger::open(&workspace_dir)?;
        ledger.list_checkpoints(limit)
    })
    .await
    .map_err(|e| format!("list_checkpoints join: {e}"))?
    .map_err(|e: anyhow::Error| format!("list_checkpoints: {e:#}"))?;

    debug!(
        "[memory_diff][rpc] list_checkpoints returned {} checkpoints",
        checkpoints.len()
    );
    Ok(RpcOutcome::new(
        ListCheckpointsResponse { checkpoints },
        vec![],
    ))
}

pub async fn diff_since_checkpoint_rpc(
    req: DiffSinceCheckpointRequest,
) -> Result<RpcOutcome<DiffSinceCheckpointResponse>, String> {
    debug!(
        "[memory_diff][rpc] diff_since_checkpoint checkpoint_id={}",
        req.checkpoint_id
    );
    let config = config_rpc::load_config_with_timeout().await?;
    let diff = ops::diff_since_checkpoint(
        &req.checkpoint_id,
        &config,
        req.include_text_diff.unwrap_or(false),
    )
    .await?;
    debug!(
        "[memory_diff][rpc] diff_since_checkpoint done sources={}",
        diff.per_source.len()
    );
    Ok(RpcOutcome::new(
        DiffSinceCheckpointResponse { diff },
        vec![],
    ))
}

pub async fn cleanup_rpc(req: CleanupRequest) -> Result<RpcOutcome<CleanupResponse>, String> {
    debug!(
        "[memory_diff][rpc] cleanup older_than_days={}",
        req.older_than_days
    );
    let config = config_rpc::load_config_with_timeout().await?;
    let deleted = ops::cleanup(&config, req.older_than_days as u32).await?;
    debug!("[memory_diff][rpc] cleanup done deleted={}", deleted);
    Ok(RpcOutcome::new(
        CleanupResponse {
            deleted_snapshots: deleted,
        },
        vec![],
    ))
}
