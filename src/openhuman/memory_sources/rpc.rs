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
    pub max_commits: Option<u32>,
    #[serde(default)]
    pub max_issues: Option<u32>,
    #[serde(default)]
    pub max_prs: Option<u32>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub since_days: Option<u32>,
    #[serde(default)]
    pub max_items: Option<u32>,
    #[serde(default)]
    pub selector: Option<String>,
    #[serde(default)]
    pub max_tokens_per_sync: Option<u64>,
    #[serde(default)]
    pub max_cost_per_sync_usd: Option<f64>,
    #[serde(default)]
    pub sync_depth_days: Option<u32>,
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

    let mut entry = MemorySourceEntry {
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
        max_commits: req.max_commits,
        max_issues: req.max_issues,
        max_prs: req.max_prs,
        query: req.query,
        since_days: req.since_days,
        max_items: req.max_items,
        selector: req.selector,
        max_tokens_per_sync: req.max_tokens_per_sync,
        max_cost_per_sync_usd: req.max_cost_per_sync_usd,
        sync_depth_days: req.sync_depth_days,
    };

    // Apply conservative per-kind defaults when the caller left caps unset.
    apply_kind_defaults(&mut entry);

    let source = registry::add_source(entry).await?;
    Ok(RpcOutcome::new(AddResponse { source }, vec![]))
}

/// Apply conservative per-kind cap defaults to a new source entry.
///
/// Only fills fields that are still `None` — never overwrites a
/// caller-supplied value. This mirrors the retroactive migration logic in
/// `reconcile::apply_composio_source_caps_migration` so the same defaults
/// are applied consistently at creation time and during migration.
pub fn apply_kind_defaults(entry: &mut MemorySourceEntry) {
    match entry.kind {
        SourceKind::GithubRepo => {
            if entry.max_prs.is_none() {
                entry.max_prs = Some(10);
            }
            if entry.max_issues.is_none() {
                entry.max_issues = Some(10);
            }
            if entry.max_commits.is_none() {
                entry.max_commits = Some(50);
            }
        }
        SourceKind::RssFeed => {
            if entry.max_items.is_none() {
                entry.max_items = Some(20);
            }
        }
        SourceKind::TwitterQuery => {
            if entry.since_days.is_none() {
                entry.since_days = Some(7);
            }
        }
        // Folder / WebPage / Composio: no defaults to apply here.
        // Composio defaults are set at upsert time in registry::upsert_composio_source.
        _ => {}
    }
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

// ── Supported Toolkits ──

#[derive(Debug, serde::Serialize)]
pub struct SupportedToolkitsResponse {
    /// Sorted, de-duplicated toolkit slugs that ship a native memory-sync
    /// provider (e.g. `clickup`, `github`, `gmail`, `linear`, `notion`,
    /// `slack`). Anything outside this set can never sync.
    pub toolkits: Vec<String>,
}

/// Toolkit slugs the memory-sync layer can actually run, sourced from the
/// provider registry (`all_providers()`) — the single source of truth shared
/// with `scan_active_sync_targets`. Exposed so the Add Source picker can
/// disable connections whose toolkit has no provider instead of letting the
/// user add a dead source. See issue #3352.
pub async fn supported_toolkits_rpc() -> Result<RpcOutcome<SupportedToolkitsResponse>, String> {
    tracing::debug!("[memory_sources] supported_toolkits_rpc: entry");
    // Ensure the built-in providers are registered before we snapshot the
    // registry — in CLI / fresh-process contexts the startup hook that calls
    // this may not have run yet.
    crate::openhuman::memory_sync::composio::init_default_composio_sync_providers();

    let mut toolkits: Vec<String> =
        crate::openhuman::memory_sync::composio::all_composio_sync_providers()
            .iter()
            .map(|p| p.toolkit_slug().to_string())
            .collect();
    toolkits.sort();
    toolkits.dedup();

    tracing::debug!(
        count = toolkits.len(),
        toolkits = ?toolkits,
        "[memory_sources] supported_toolkits_rpc: resolved supported toolkit set"
    );
    Ok(RpcOutcome::new(
        SupportedToolkitsResponse { toolkits },
        vec![],
    ))
}

// ── Sync Audit Log ──

#[derive(Debug, serde::Serialize)]
pub struct SyncAuditLogResponse {
    pub entries: Vec<crate::openhuman::memory_sync::sources::audit::SyncAuditEntry>,
}

pub async fn sync_audit_log_rpc() -> Result<RpcOutcome<SyncAuditLogResponse>, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let entries = crate::openhuman::memory_sync::sources::audit::read_audit_log(&config);
    Ok(RpcOutcome::new(SyncAuditLogResponse { entries }, vec![]))
}

// ── Estimate Sync Cost ──

#[derive(Debug, serde::Deserialize)]
pub struct EstimateSyncCostRequest {
    pub source_id: String,
}

#[derive(Debug, serde::Serialize)]
pub struct EstimateSyncCostResponse {
    pub source_id: String,
    pub item_count: u32,
    pub estimated_tokens: u64,
    pub estimated_cost_usd: f64,
    pub budget_max_cost_usd: Option<f64>,
    pub budget_max_tokens: Option<u64>,
}

pub async fn estimate_sync_cost_rpc(
    req: EstimateSyncCostRequest,
) -> Result<RpcOutcome<EstimateSyncCostResponse>, String> {
    tracing::debug!(source_id = %req.source_id, "[memory_sources] estimate_sync_cost_rpc: entry");

    let source = registry::get_source(&req.source_id)
        .await?
        .ok_or_else(|| format!("source '{}' not found", req.source_id))?;

    let config = config_rpc::load_config_with_timeout().await?;
    let reader = readers::reader_for(&source.kind);
    let items = reader.list_items(&source, &config).await?;

    let item_count = items.len() as u32;
    // estimated_tokens includes both input (500/item) and output (100/item)
    // to be consistent with the cost calculation below.
    let estimated_input_tokens = item_count as u64 * 500;
    let estimated_output_tokens = item_count as u64 * 100;
    let estimated_tokens = estimated_input_tokens + estimated_output_tokens;
    let estimated_cost_usd = crate::openhuman::memory_sync::sources::audit::estimate_cost_usd(
        estimated_input_tokens,
        estimated_output_tokens,
    );

    Ok(RpcOutcome::new(
        EstimateSyncCostResponse {
            source_id: req.source_id,
            item_count,
            estimated_tokens,
            estimated_cost_usd,
            budget_max_cost_usd: source.max_cost_per_sync_usd,
            budget_max_tokens: source.max_tokens_per_sync,
        },
        vec![],
    ))
}

// ── Monthly Cost Summary ──

#[derive(Debug, serde::Serialize)]
pub struct MonthlyCostSummaryResponse {
    pub month: String,
    pub total_cost_usd: f64,
    pub total_syncs: u32,
    pub total_items: u32,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

pub async fn monthly_cost_summary_rpc() -> Result<RpcOutcome<MonthlyCostSummaryResponse>, String> {
    tracing::debug!("[memory_sources] monthly_cost_summary_rpc: entry");
    let config = config_rpc::load_config_with_timeout().await?;
    let entries = crate::openhuman::memory_sync::sources::audit::read_audit_log(&config);

    let now = chrono::Utc::now();
    let month_str = now.format("%Y-%m").to_string();

    let mut total_cost_usd = 0.0f64;
    let mut total_syncs = 0u32;
    let mut total_items = 0u32;
    let mut total_input_tokens = 0u64;
    let mut total_output_tokens = 0u64;

    for entry in &entries {
        if entry.timestamp.format("%Y-%m").to_string() == month_str {
            total_cost_usd += entry.effective_cost_usd();
            total_syncs += 1;
            total_items += entry.items_fetched;
            total_input_tokens += entry.input_tokens;
            total_output_tokens += entry.output_tokens;
        }
    }

    Ok(RpcOutcome::new(
        MonthlyCostSummaryResponse {
            month: month_str,
            total_cost_usd,
            total_syncs,
            total_items,
            total_input_tokens,
            total_output_tokens,
        },
        vec![],
    ))
}

// ── Apply All In ──

/// Response returned by `memory_sources_apply_all_in`.
#[derive(Debug, serde::Serialize)]
pub struct AllInResponse {
    /// All memory source entries after the "all in" transformation
    /// (every source enabled, every cap cleared).
    pub sources: Vec<MemorySourceEntry>,
    /// Number of sync tasks spawned (one per enabled source).
    pub sync_triggered: u32,
}

/// Enable ALL memory sources, clear all caps, and trigger a sync for
/// every source.
///
/// Returns immediately with the updated source list and the number of
/// syncs queued. Individual syncs run in the background and publish
/// `MemorySyncStageChanged` events as they progress.
pub async fn apply_all_in_rpc() -> Result<RpcOutcome<AllInResponse>, String> {
    tracing::info!("[memory_sources] apply_all_in_rpc: entry");

    // Enable all sources and clear caps.
    let sources = registry::apply_all_in().await?;

    // Trigger a background sync for every enabled source.
    let config = config_rpc::load_config_with_timeout().await?;
    let mut sync_triggered: u32 = 0;

    for source in &sources {
        if !source.enabled {
            continue;
        }
        tracing::debug!(
            source_id = %source.id,
            kind = %source.kind.as_str(),
            "[memory_sources] apply_all_in_rpc: triggering sync"
        );
        match crate::openhuman::memory_sources::sync::sync_source(source.clone(), config.clone())
            .await
        {
            Ok(()) => {
                sync_triggered += 1;
            }
            Err(e) => {
                // Non-fatal: log and continue — best-effort sync trigger.
                tracing::warn!(
                    source_id = %source.id,
                    error = %e,
                    "[memory_sources] apply_all_in_rpc: sync trigger failed for source"
                );
            }
        }
    }

    tracing::info!(
        sources = sources.len(),
        sync_triggered,
        "[memory_sources] apply_all_in_rpc: complete"
    );

    Ok(RpcOutcome::new(
        AllInResponse {
            sources,
            sync_triggered,
        },
        vec![],
    ))
}

#[cfg(test)]
mod supported_toolkits_tests {
    use super::*;

    /// The supported-toolkit set must include every built-in provider slug.
    /// Asserted via `contains` (not exact equality) because the provider
    /// registry is a process-global shared with other tests in this binary
    /// that may register ad-hoc dummy providers.
    #[tokio::test]
    async fn supported_toolkits_includes_builtin_providers() {
        let outcome = supported_toolkits_rpc()
            .await
            .expect("supported_toolkits_rpc should succeed");
        let toolkits = outcome.value.toolkits;

        for slug in ["clickup", "github", "gmail", "linear", "notion", "slack"] {
            assert!(
                toolkits.iter().any(|t| t == slug),
                "expected supported toolkits to include '{slug}', got {toolkits:?}"
            );
        }
    }

    /// The returned set must be sorted and free of duplicates.
    #[tokio::test]
    async fn supported_toolkits_is_sorted_and_deduped() {
        let outcome = supported_toolkits_rpc()
            .await
            .expect("supported_toolkits_rpc should succeed");
        let toolkits = outcome.value.toolkits;

        let mut sorted = toolkits.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            toolkits, sorted,
            "toolkits should be sorted and de-duplicated"
        );
    }
}
