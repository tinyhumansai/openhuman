//! RPC handler implementations for memory sources.

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory_sources::readers;
use crate::openhuman::memory_sources::registry::{self, MemorySourcePatch};
use crate::openhuman::memory_sources::types::{MemorySourceEntry, SourceKind};
use crate::rpc::RpcOutcome;

// ── List ──

#[derive(Debug, serde::Serialize)]
pub struct ListResponse {
    pub sources: Vec<MemorySourceEntry>,
}

pub async fn list_rpc() -> Result<RpcOutcome<ListResponse>, String> {
    tracing::debug!("[memory_sources] list_rpc: entry");
    // Lazily reconcile Composio connections into the registry so users
    // see freshly-connected integrations as memory sources immediately,
    // without waiting for a restart or for the connection_created hook
    // to fire (which only triggers on OAuth handoff, not on first launch
    // after the user previously connected something).
    crate::openhuman::memory_sources::reconcile::ensure_composio_sources().await;
    let sources = registry::list_sources().await?;
    Ok(RpcOutcome::new(ListResponse { sources }, vec![]))
}

// ── Get ──

#[derive(Debug, serde::Deserialize)]
pub struct GetRequest {
    pub id: String,
}

#[derive(Debug, serde::Serialize)]
pub struct GetResponse {
    pub source: Option<MemorySourceEntry>,
}

pub async fn get_rpc(req: GetRequest) -> Result<RpcOutcome<GetResponse>, String> {
    tracing::debug!(id = %req.id, "[memory_sources] get_rpc: entry");
    let source = registry::get_source(&req.id).await?;
    Ok(RpcOutcome::new(GetResponse { source }, vec![]))
}

// ── Add ──

#[derive(Debug, serde::Deserialize)]
pub struct AddRequest {
    pub kind: SourceKind,
    pub label: String,
    #[serde(default = "default_true")]
    pub enabled: bool,

    // Kind-specific fields (flat)
    #[serde(default)]
    pub toolkit: Option<String>,
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub glob: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub since_days: Option<u32>,
    #[serde(default)]
    pub max_items: Option<u32>,
    #[serde(default)]
    pub selector: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, serde::Serialize)]
pub struct AddResponse {
    pub source: MemorySourceEntry,
}

pub async fn add_rpc(req: AddRequest) -> Result<RpcOutcome<AddResponse>, String> {
    tracing::info!(
        kind = %req.kind.as_str(),
        label = %req.label,
        "[memory_sources] add_rpc: entry"
    );

    let entry = MemorySourceEntry {
        id: format!("src_{}", uuid::Uuid::new_v4().as_simple()),
        kind: req.kind,
        label: req.label,
        enabled: req.enabled,
        toolkit: req.toolkit,
        connection_id: req.connection_id,
        path: req.path,
        glob: req.glob,
        url: req.url,
        branch: req.branch,
        paths: req.paths,
        query: req.query,
        since_days: req.since_days,
        max_items: req.max_items,
        selector: req.selector,
    };

    let source = registry::add_source(entry).await?;
    Ok(RpcOutcome::new(AddResponse { source }, vec![]))
}

// ── Update ──

#[derive(Debug, serde::Deserialize)]
pub struct UpdateRequest {
    pub id: String,
    #[serde(flatten)]
    pub patch: MemorySourcePatch,
}

#[derive(Debug, serde::Serialize)]
pub struct UpdateResponse {
    pub source: MemorySourceEntry,
}

pub async fn update_rpc(req: UpdateRequest) -> Result<RpcOutcome<UpdateResponse>, String> {
    tracing::info!(id = %req.id, "[memory_sources] update_rpc: entry");
    let source = registry::update_source(&req.id, req.patch).await?;
    Ok(RpcOutcome::new(UpdateResponse { source }, vec![]))
}

// ── Remove ──

#[derive(Debug, serde::Deserialize)]
pub struct RemoveRequest {
    pub id: String,
}

#[derive(Debug, serde::Serialize)]
pub struct RemoveResponse {
    pub removed: bool,
}

pub async fn remove_rpc(req: RemoveRequest) -> Result<RpcOutcome<RemoveResponse>, String> {
    tracing::info!(id = %req.id, "[memory_sources] remove_rpc: entry");
    let removed = registry::remove_source(&req.id).await?;
    Ok(RpcOutcome::new(RemoveResponse { removed }, vec![]))
}

// ── List Items ──

#[derive(Debug, serde::Deserialize)]
pub struct ListItemsRequest {
    pub source_id: String,
}

#[derive(Debug, serde::Serialize)]
pub struct ListItemsResponse {
    pub items: Vec<crate::openhuman::memory_sources::types::SourceItem>,
}

pub async fn list_items_rpc(
    req: ListItemsRequest,
) -> Result<RpcOutcome<ListItemsResponse>, String> {
    tracing::debug!(source_id = %req.source_id, "[memory_sources] list_items_rpc: entry");

    let source = registry::get_source(&req.source_id)
        .await?
        .ok_or_else(|| format!("source '{}' not found", req.source_id))?;

    let config = config_rpc::load_config_with_timeout().await?;
    let reader = readers::reader_for(&source.kind);
    let items = reader.list_items(&source, &config).await?;

    Ok(RpcOutcome::new(ListItemsResponse { items }, vec![]))
}

// ── Read Item ──

#[derive(Debug, serde::Deserialize)]
pub struct ReadItemRequest {
    pub source_id: String,
    pub item_id: String,
}

#[derive(Debug, serde::Serialize)]
pub struct ReadItemResponse {
    pub content: crate::openhuman::memory_sources::types::SourceContent,
}

pub async fn read_item_rpc(req: ReadItemRequest) -> Result<RpcOutcome<ReadItemResponse>, String> {
    tracing::debug!(
        source_id = %req.source_id,
        item_id = %req.item_id,
        "[memory_sources] read_item_rpc: entry"
    );

    let source = registry::get_source(&req.source_id)
        .await?
        .ok_or_else(|| format!("source '{}' not found", req.source_id))?;

    let config = config_rpc::load_config_with_timeout().await?;
    let reader = readers::reader_for(&source.kind);
    let content = reader.read_item(&source, &req.item_id, &config).await?;

    Ok(RpcOutcome::new(ReadItemResponse { content }, vec![]))
}

// ── Sync ──

#[derive(Debug, serde::Deserialize)]
pub struct SyncRequest {
    pub source_id: String,
}

#[derive(Debug, serde::Serialize)]
pub struct SyncResponse {
    pub requested: bool,
    pub source_id: String,
}

pub async fn sync_rpc(req: SyncRequest) -> Result<RpcOutcome<SyncResponse>, String> {
    tracing::info!(source_id = %req.source_id, "[memory_sources] sync_rpc: entry");

    let source = registry::get_source(&req.source_id)
        .await?
        .ok_or_else(|| format!("source '{}' not found", req.source_id))?;

    let config = config_rpc::load_config_with_timeout().await?;
    crate::openhuman::memory_sources::sync::sync_source(source, config).await?;

    Ok(RpcOutcome::new(
        SyncResponse {
            requested: true,
            source_id: req.source_id,
        },
        vec![],
    ))
}

// ── Status List ──

#[derive(Debug, serde::Serialize)]
pub struct StatusListResponse {
    pub statuses: Vec<crate::openhuman::memory_sources::status::SourceStatus>,
}

pub async fn status_list_rpc() -> Result<RpcOutcome<StatusListResponse>, String> {
    tracing::debug!("[memory_sources] status_list_rpc: entry");
    let config = config_rpc::load_config_with_timeout().await?;
    let statuses = crate::openhuman::memory_sources::status::status_list(&config).await?;
    Ok(RpcOutcome::new(StatusListResponse { statuses }, vec![]))
}
