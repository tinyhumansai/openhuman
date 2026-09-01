use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::memory::api::provider::sessions::{
    CodingSessionIngestReport, CodingSessionSource,
};
use crate::openhuman::memory::api::provider::sync::SyncAuditEntry;
use crate::openhuman::memory::binding::MemoryBinding;
use crate::openhuman::memory::sources::apply_kind_defaults;
use crate::openhuman::memory::sources::readers;
use crate::openhuman::memory::sources::registry::{self, MemorySourcePatch};
use crate::openhuman::memory::sources::types::{MemorySourceEntry, SourceKind};
use crate::rpc::RpcOutcome;

/// The coding-session ingest request, under this domain's own name.
///
/// Re-exported here so `schemas.rs` can name it as
/// `rpc::CodingSessionIngestRequest`, the way every other handler adapter in
/// that file names its request type. The path behind the alias is now the
/// contract's (`tinymemory_api::provider::sessions`) rather than the engine's,
/// which is what lets `ingest_coding_sessions_rpc` hand the value straight to
/// the driver with no conversion in between — the request that crosses the bus
/// is the request `schemas.rs` deserialised.
pub use crate::openhuman::memory::api::provider::sessions::CodingSessionIngestRequest;

/// The refusal a handler returns when the bound driver serves no such family.
///
/// One place, so the message and the log line cannot drift between the six
/// call sites, and so the driver id is always in both. See the module docs for
/// why these handlers refuse rather than degrade to an empty answer.
fn unserved(binding: &MemoryBinding, family: &str, call: &str) -> String {
    tracing::warn!(
        driver = %binding.driver_id(),
        family = %family,
        call = %call,
        "[memory_sources] refusing: bound driver does not serve this capability family"
    );
    format!(
        "the bound memory driver '{}' does not serve {family}",
        binding.driver_id()
    )
}

#[derive(Debug, serde::Serialize)]
pub struct CodingSessionStatusResponse {
    pub sources: Vec<CodingSessionSource>,
}

/// Discover what each supported coding agent's session store holds.
///
/// The scan happens driver-side and is bounded by the driver's own caps, which
/// is why this no longer needs a blocking worker: the walk that used to run on
/// this process's pool now runs behind the bus, and what comes back is counts.
/// `CodingSessionSource::scan_truncated` is how a caller learns the counts are
/// a floor — the same field the engine's `CodingSessionSourceStatus` carried,
/// under the same name.
pub async fn coding_session_status_rpc() -> Result<RpcOutcome<CodingSessionStatusResponse>, String>
{
    tracing::debug!("[memory_sources] coding_session_status_rpc: entry");
    let config = config_rpc::load_config_with_timeout().await?;
    let binding = crate::openhuman::memory::binding::for_config(&config)?;
    let Some(sessions) = binding.provider().as_coding_sessions() else {
        return Err(unserved(
            &binding,
            "coding sessions",
            "coding_session_status",
        ));
    };

    let sources = sessions
        .coding_session_status()
        .await
        .map_err(|error| format!("coding session status: {error}"))?;

    tracing::debug!(
        driver = %binding.driver_id(),
        sources = sources.len(),
        files = sources
            .iter()
            .map(|source| source.session_files)
            .sum::<usize>(),
        truncated = sources.iter().any(|source| source.scan_truncated),
        "[memory_sources] coding_session_status_rpc: exit"
    );
    Ok(RpcOutcome::new(
        CodingSessionStatusResponse { sources },
        vec![],
    ))
}

/// Wall-clock ceiling for one `ingest_coding_sessions` RPC, sized to the number
/// of sessions the caller asked to backfill and hard-capped at the ceiling the
/// frontend can actually wait for.
///
/// The original formula (`120 + N*30`) assumed **one LLM call per session**.
/// That premise is false: TinyCortex's persona pipeline splits an oversized
/// session into windows (`WINDOW_CHARS`-sized chunks of evidence) and issues one
/// LLM call *per window*, so a multi-window session drives several sequential
/// calls. A dense backfill therefore blew the old budget — 15 sessions hit the
/// exact 570 s ceiling (`120 + 15*30`) and were killed mid-flight.
///
/// The per-session allowance is therefore sized for *multiple* windows, not one
/// call: `PER_SESSION_SECS` budgets ~3 sequential per-window LLM calls at the
/// windows' observed 20–45 s span (#5509). It is a deliberate flat estimate, not
/// a per-session window count.
///
/// The result is hard-capped at `HARD_CAP_SECS` for two reasons that are really
/// one. First, this is the true reachable ceiling: the frontend RPC client
/// clamps every per-call timeout to `PER_CALL_TIMEOUT_MAX_MS = 600 s`
/// (`app/src/services/coreRpcClient.ts`), so a server budget above that can never
/// be observed — the client aborts first. Second, that cap also bounds the
/// blocking-pool worker this budget guards: `max_sessions` is untrusted (an
/// advertised programmatic RPC, `platform/about_app/catalog_data.rs`), and
/// without the cap a caller passing 1000 would pin a thread for ~33 h. Capping
/// the resulting `Duration` — not the multiplier — makes both true at once.
///
/// Because a single pass is bounded, large histories drain across repeated passes
/// (client `drainCodingSessions`); the per-pass batch is sized so `BASE + N*PER`
/// stays under the cap for the UI's `CODING_SESSION_BATCH_MAX`, keeping the
/// server budget the *tighter* of the two so it returns a clean structured
/// timeout before the client's fetch aborts. This is a *ceiling to catch a wedged
/// run*, not a latency target.
///
/// `pub(crate)` so the module driver can size its own bus deadline from the
/// same formula (`modules::memory`). That call sits *inside* this one, and
/// tinybus gives every call a 30 s default deadline if nobody sets one — 19×
/// tighter than the smallest budget computed here, which is how a completed
/// import came to be reported as a failure (#5802). One formula, two layers.
pub(crate) fn ingest_budget(max_sessions: usize) -> std::time::Duration {
    /// Fixed overhead allowance (config load, discovery, process warm-up) added
    /// on top of the per-session budget.
    const BASE_SECS: u64 = 120;
    /// Per-session allowance, sized for ~3 sequential per-window LLM calls at the
    /// 20–45 s/window span observed in #5509 rather than the single call the old
    /// formula assumed.
    const PER_SESSION_SECS: u64 = 90;
    /// Hard ceiling on the whole budget. Mirrors the frontend's
    /// `PER_CALL_TIMEOUT_MAX_MS` (600 s) — a larger budget is unreachable because
    /// the client aborts first — and bounds the blocking worker against an
    /// untrusted `max_sessions`.
    const HARD_CAP_SECS: u64 = 600;

    let scaled = BASE_SECS.saturating_add((max_sessions as u64).saturating_mul(PER_SESSION_SECS));
    std::time::Duration::from_secs(scaled.min(HARD_CAP_SECS))
}

/// Distil local coding-agent transcripts into observations.
///
/// The pipeline runs driver-side now, which removes the blocking-worker hop
/// this handler used to need: the engine's persona pass carried borrowed path
/// state and was not `Send`, so it had to be driven from `spawn_blocking` with
/// a `block_on` inside. The contract member is an ordinary `Send` future, so
/// the RPC simply awaits it.
///
/// **The deadline stays here on purpose.** `MemoryCodingSessions` documents
/// that it cannot bound the wall-clock cost — each session is one or more
/// sequential model calls — and that a caller needing a deadline enforces it on
/// its own side. [`ingest_budget`] is that deadline, unchanged; a timeout is
/// still reported as a structured error rather than as a short report, because
/// a report the run never finished writing is not progress the caller can keep.
pub async fn ingest_coding_sessions_rpc(
    req: CodingSessionIngestRequest,
) -> Result<RpcOutcome<CodingSessionIngestReport>, String> {
    tracing::info!(
        backfill = req.backfill,
        max_sessions = req.max_sessions,
        "[memory_sources] ingest_coding_sessions_rpc: entry"
    );
    let config = Config::load_or_init()
        .await
        .map_err(|error| format!("load config for coding-session ingestion: {error}"))?;
    let binding = crate::openhuman::memory::binding::for_config(&config)?;
    let Some(sessions) = binding.provider().as_coding_sessions() else {
        return Err(unserved(
            &binding,
            "coding sessions",
            "ingest_coding_sessions",
        ));
    };

    // Wall-clock ceiling so a stalled provider call or a wedged session step
    // can't keep the RPC waiting indefinitely (#4863 review), sized to the
    // requested backfill so a legitimate large run isn't killed mid-flight
    // while a genuine infinite hang still terminates. Read before `req` moves.
    let ingest_timeout = ingest_budget(req.max_sessions);
    let report = tokio::time::timeout(ingest_timeout, sessions.ingest_coding_sessions(req))
        .await
        .map_err(|_elapsed| {
            tracing::error!(
                driver = %binding.driver_id(),
                timeout_secs = ingest_timeout.as_secs(),
                "[memory_sources] ingest_coding_sessions_rpc: timed out"
            );
            format!(
                "ingest coding sessions: timed out after {}s",
                ingest_timeout.as_secs()
            )
        })?
        .map_err(|error| format!("ingest coding sessions: {error}"))?;

    tracing::info!(
        driver = %binding.driver_id(),
        mode = %report.mode,
        processed = report.sessions_processed,
        failed = report.sessions_failed,
        budget_hit = report.budget_hit,
        "[memory_sources] ingest_coding_sessions_rpc: exit"
    );
    Ok(RpcOutcome::new(report, vec![]))
}

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
    //
    // The reconcile also hands back the live active-connection set it just
    // scanned, which we reuse to hide Composio rows whose connection is no
    // longer active (re-auth / token expiry leaves a stale row behind) and to
    // collapse identical same-id duplicates from any reconcile race. This is a
    // display-layer filter only — no row, setting, or ingested memory is
    // removed; an inactive connection's row simply reappears once it re-activates.
    let active = crate::openhuman::memory::sources::reconcile::ensure_composio_sources().await;
    let sources = registry::list_sources().await?;
    let filtered = filter_to_active_composio_sources(sources, active.as_ref());
    tracing::debug!(
        active_known = active.is_some(),
        active = active.as_ref().map(|a| a.len()).unwrap_or(0),
        returned = filtered.len(),
        "[memory_sources] list_rpc: filtered listing to active connections"
    );
    Ok(RpcOutcome::new(ListResponse { sources: filtered }, vec![]))
}

/// Filter the registry listing down to the live, deduplicated set of sources.
///
/// Composio sources are kept only when their `connection_id` is in `active`
/// (the live active-connection set scanned by `ensure_composio_sources` this
/// poll), collapsed to one row per `connection_id` so a non-atomic
/// `upsert_composio_source` race can't surface identical duplicate rows.
/// Non-Composio sources (folder / git / …) have no connection and are always
/// shown.
///
/// `active == None` means the live scan was unavailable (config / network /
/// auth failure). We must NOT read that as "everything is inactive" and hide
/// every Composio source — so on `None` the list passes through untouched. This
/// is hide-not-delete: the worst case is a stale row showing briefly until the
/// next good scan, fully reversible. Pure (no I/O) so it is unit-tested directly.
fn filter_to_active_composio_sources(
    mut sources: Vec<MemorySourceEntry>,
    active: Option<&std::collections::HashSet<String>>,
) -> Vec<MemorySourceEntry> {
    let Some(active) = active else {
        // Scan unavailable — show everything rather than hiding all Composio rows.
        return sources;
    };
    let mut seen = std::collections::HashSet::new();
    sources.retain(|s| {
        if s.kind != SourceKind::Composio {
            return true; // no connection to reconcile against — always show
        }
        match s.connection_id.as_deref() {
            // Active connection, first occurrence of this id → keep.
            // Inactive (`!contains`) → hidden (RC-A); later duplicate of the
            // same id (`!seen.insert`) → collapsed (RC-B).
            Some(id) => active.contains(id) && seen.insert(id.to_string()),
            // Malformed Composio row with no connection_id — keep it visible
            // rather than silently dropping a user's source.
            None => true,
        }
    });
    sources
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
    pub items: Vec<crate::openhuman::memory::sources::types::SourceItem>,
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
    pub content: crate::openhuman::memory::sources::types::SourceContent,
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

    let config = config_rpc::load_config_with_timeout().await?;

    // The existence check is the driver's now: `run_source_sync` resolves the id
    // against the registry it already reads for per-source budgets, and answers
    // `NotFound` for an id nobody registered. Looking it up here as well would
    // be a second read of the same file that can disagree with the one the
    // pipeline actually applies.
    let binding = crate::openhuman::memory::binding::for_config(&config)?;
    let sync = binding.provider().as_source_sync().ok_or_else(|| {
        format!(
            "the bound memory driver '{}' does not serve source sync",
            binding.driver_id()
        )
    })?;
    // The enabled gate is not the driver's. `run_source_sync` runs whatever id
    // it is handed; it is `sources::sync::sync_source` and the periodic loop
    // that refuse a disabled entry, and this RPC is the third caller, the one
    // behind the user's Sync button. Same words as `sync_source` so the UI
    // reads one message (#5820).
    if let Some(entry) = super::registry::get_source_in(&config, &req.source_id)? {
        if !entry.enabled {
            return Err(format!("source '{}' is disabled", entry.id));
        }
        // Composio rows never reach the memory driver's pipeline: v1.13.4
        // removed the engine's in-process Composio sync, and the driver now
        // answers this id with "synced through the connector module, not this
        // pipeline". The connector-backed run IS the sync for this kind, so
        // the one Sync button dispatches there — same entry point the
        // `openhuman.composio_sync` RPC uses, which reads the connected
        // account through the module and ingests through this same binding.
        if entry.kind == tinymemory_sources::types::SourceKind::Composio {
            let connection_id = entry.connection_id.as_deref().ok_or_else(|| {
                format!(
                    "composio source '{}' has no connection_id; remove and re-add the source",
                    entry.id
                )
            })?;
            crate::openhuman::integrations::composio::ops::composio_sync(
                &config,
                connection_id,
                Some("manual".to_string()),
            )
            .await?;
            return Ok(RpcOutcome::new(
                SyncResponse {
                    requested: true,
                    source_id: req.source_id,
                },
                vec![],
            ));
        }
    }
    sync.run_source_sync(&req.source_id)
        .await
        .map_err(|error| error.to_string())?;

    Ok(RpcOutcome::new(
        SyncResponse {
            requested: true,
            source_id: req.source_id,
        },
        vec![],
    ))
}

// ── Reconcile ──

#[derive(Debug, Default, serde::Deserialize)]
pub struct ReconcileRequest {
    /// Restrict to one source; omit to inspect every enabled source.
    #[serde(default)]
    pub source_id: Option<String>,
    /// When true, kick off background summarise+ingest for every scope
    /// with pending files. When false (default), report-only.
    #[serde(default)]
    pub execute: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct ReconcileScopeReport {
    pub source_id: String,
    pub tree_scope: String,
    /// Raw `.md` files on disk for this scope.
    pub total_raw_files: u64,
    /// Files already covered by a tree summary.
    pub covered: u64,
    /// Files awaiting summarisation into the tree.
    ///
    /// A count the driver computed, not one this handler derived from a list.
    /// The engine's coverage scan returned each pending file's absolute path
    /// inside the content vault and this handler called `.len()` on it;
    /// `RawArchiveCoverage` deliberately reports the count alone, because a
    /// path describes the driver's storage layout and nothing here ever read
    /// one. The number is the same number.
    pub pending: u64,
    /// True when `execute` was set and a background reconcile was started.
    pub started: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct ReconcileResponse {
    pub scopes: Vec<ReconcileScopeReport>,
}
