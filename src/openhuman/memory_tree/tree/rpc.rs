//! RPC handler functions for the memory tree layer.
//!
//! Public JSON-RPC surface:
//! - `openhuman.memory_tree_ingest` — one unified ingest. Caller supplies
//!   `source_kind` + generic JSON `payload` (adapter-specific). Internally
//!   dispatches to chat / email / document canonicalisers.
//! - `openhuman.memory_tree_list_chunks` — listing with filters.
//! - `openhuman.memory_tree_get_chunk` — single chunk fetch.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::openhuman::config::Config;
use crate::openhuman::memory::ingest_pipeline::{
    ingest_chat as do_ingest_chat, ingest_document as do_ingest_document,
    ingest_email as do_ingest_email, IngestResult,
};
use crate::openhuman::memory_store::chunks::store::{self as chunk_store, ListChunksQuery};
use crate::openhuman::memory_store::chunks::types::{Chunk, SourceKind};
use crate::openhuman::memory_sync::canonicalize::{
    chat::ChatBatch, document::DocumentInput, email::EmailThread,
};
use crate::rpc::RpcOutcome;

/// Unified ingest request. The `payload` shape is adapter-specific and is
/// validated inside the dispatch based on `source_kind`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IngestRequest {
    /// Which kind of source the payload represents.
    pub source_kind: SourceKind,
    /// Logical source id (channel/group for chat, thread for email, doc id).
    pub source_id: String,
    /// Account/user this content belongs to.
    #[serde(default)]
    pub owner: String,
    /// Optional labels/tags carried through.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Adapter-specific payload — shape matches the canonicaliser for
    /// `source_kind`:
    /// - `chat`     → [`ChatBatch`]
    /// - `email`    → [`EmailThread`]
    /// - `document` → [`DocumentInput`]
    pub payload: Value,
}

/// Unified ingest RPC handler. Dispatches on `source_kind`.
pub async fn ingest_rpc(
    config: &Config,
    req: IngestRequest,
) -> Result<RpcOutcome<IngestResult>, String> {
    let IngestRequest {
        source_kind,
        source_id,
        owner,
        tags,
        payload,
    } = req;

    log::debug!(
        "[memory::rpc] ingest kind={} source_id={}",
        source_kind.as_str(),
        source_id
    );

    // Phase 2: ingest functions are async. Their scoring stage awaits the
    // extractor (cheap for regex, not-cheap for future GLiNER/LLM impls)
    // and the DB work is isolated on `spawn_blocking` inside `persist`.
    let result = match source_kind {
        SourceKind::Chat => {
            let batch: ChatBatch = serde_json::from_value(payload)
                .map_err(|e| format!("invalid chat payload: {e}"))?;
            do_ingest_chat(config, &source_id, &owner, tags, batch)
                .await
                .map_err(|e| format!("ingest: {e}"))?
        }
        SourceKind::Email => {
            let thread: EmailThread = serde_json::from_value(payload)
                .map_err(|e| format!("invalid email payload: {e}"))?;
            do_ingest_email(config, &source_id, &owner, tags, thread)
                .await
                .map_err(|e| format!("ingest: {e}"))?
        }
        SourceKind::Document => {
            let doc: DocumentInput = serde_json::from_value(payload)
                .map_err(|e| format!("invalid document payload: {e}"))?;
            do_ingest_document(config, &source_id, &owner, tags, doc)
                .await
                .map_err(|e| format!("ingest: {e}"))?
        }
    };

    Ok(RpcOutcome::single_log(
        result,
        format!(
            "memory_tree: ingest kind={} source_id={source_id}",
            source_kind.as_str()
        ),
    ))
}

/// Query shape for the `list_chunks` RPC.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ListChunksRequest {
    #[serde(default)]
    pub source_kind: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub since_ms: Option<i64>,
    #[serde(default)]
    pub until_ms: Option<i64>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Response shape for the `list_chunks` RPC.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListChunksResponse {
    pub chunks: Vec<Chunk>,
}

/// `list_chunks` RPC handler. Filters and returns persisted chunks ordered by
/// timestamp DESC.
pub async fn list_chunks_rpc(
    config: &Config,
    req: ListChunksRequest,
) -> Result<RpcOutcome<ListChunksResponse>, String> {
    let query = ListChunksQuery {
        source_kind: match req.source_kind.as_deref() {
            None => None,
            Some(s) => Some(SourceKind::parse(s)?),
        },
        source_id: req.source_id,
        owner: req.owner,
        since_ms: req.since_ms,
        until_ms: req.until_ms,
        limit: req.limit,
    };
    let rows = tokio::task::spawn_blocking({
        let config = config.clone();
        move || chunk_store::list_chunks(&config, &query)
    })
    .await
    .map_err(|e| format!("list_chunks join error: {e}"))?
    .map_err(|e| format!("list_chunks: {e}"))?;

    let n = rows.len();
    Ok(RpcOutcome::single_log(
        ListChunksResponse { chunks: rows },
        format!("memory_tree: list_chunks n={n}"),
    ))
}

/// Request shape for the `get_chunk` RPC.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetChunkRequest {
    pub id: String,
}

/// Response shape for the `get_chunk` RPC.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GetChunkResponse {
    pub chunk: Option<Chunk>,
}

/// `get_chunk` RPC handler. Returns the chunk identified by `id`, or `None`.
pub async fn get_chunk_rpc(
    config: &Config,
    req: GetChunkRequest,
) -> Result<RpcOutcome<GetChunkResponse>, String> {
    let id = req.id.clone();
    let chunk = tokio::task::spawn_blocking({
        let config = config.clone();
        move || chunk_store::get_chunk(&config, &id)
    })
    .await
    .map_err(|e| format!("get_chunk join error: {e}"))?
    .map_err(|e| format!("get_chunk: {e}"))?;
    Ok(RpcOutcome::single_log(
        GetChunkResponse { chunk },
        format!("memory_tree: get_chunk id={}", req.id),
    ))
}

/// Manual-trigger surface for the global tree's daily digest. Default
/// behavior (no `date_iso`) targets yesterday in UTC, matching the
/// scheduler's autonomous behavior. Pass an explicit `YYYY-MM-DD` to
/// re-run a specific date (idempotent — the handler skips if a daily
/// node already exists for that day).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TriggerDigestRequest {
    /// UTC calendar date in `YYYY-MM-DD` form. When omitted, defaults to
    /// `yesterday` (today minus one day, UTC).
    #[serde(default)]
    pub date_iso: Option<String>,
}

/// Response from the `trigger_digest` RPC.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TriggerDigestResponse {
    /// True when the job was newly enqueued; false when an active job for
    /// the same date was suppressed by the dedupe partial unique index.
    pub enqueued: bool,
    /// ID of the freshly-inserted job row (None when dedupe-suppressed).
    pub job_id: Option<String>,
    /// The actual date the digest will run for, echoed back as
    /// `YYYY-MM-DD`. Useful when the caller didn't pass `date_iso` and
    /// wants to know what default got chosen.
    pub date_iso: String,
}

/// `trigger_digest` RPC handler. Manually enqueues the global tree's daily
/// digest job for `date_iso` (defaults to yesterday in UTC); idempotent via the
/// jobs-queue dedupe index.
pub async fn trigger_digest_rpc(
    config: &Config,
    req: TriggerDigestRequest,
) -> Result<RpcOutcome<TriggerDigestResponse>, String> {
    use crate::openhuman::memory_queue as jobs;
    use chrono::{Duration as ChronoDuration, NaiveDate, Utc};

    let date = match req
        .date_iso
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map_err(|e| format!("invalid date_iso (expected YYYY-MM-DD): {e}"))?,
        None => Utc::now().date_naive() - ChronoDuration::days(1),
    };
    let date_iso = date.format("%Y-%m-%d").to_string();

    // Run the synchronous enqueue on a blocking thread — `trigger_digest`
    // touches SQLite and we don't want to block the async runtime even
    // for the few-microsecond INSERT.
    let cfg_clone = config.clone();
    let date_for_blocking = date;
    let job_id =
        tokio::task::spawn_blocking(move || jobs::trigger_digest(&cfg_clone, date_for_blocking))
            .await
            .map_err(|e| format!("trigger_digest join error: {e}"))?
            .map_err(|e| format!("trigger_digest: {e}"))?;

    let enqueued = job_id.is_some();
    Ok(RpcOutcome::single_log(
        TriggerDigestResponse {
            enqueued,
            job_id,
            date_iso: date_iso.clone(),
        },
        format!("memory_tree: trigger_digest date={date_iso} enqueued={enqueued}"),
    ))
}

/// Response from the `memory_backfill_status` RPC (#1574 §4b). The frontend
/// polls this while the re-embed modal is open to surface progress and to
/// dismiss the modal once the new embedding space is fully covered.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackfillStatusResponse {
    /// True while a re-embed backfill chain still has work pending — the
    /// #1365 flag OR a queued/running `reembed_backfill` job.
    pub in_progress: bool,
    /// Count of `reembed_backfill` jobs in `ready` or `running` state. `0`
    /// with `in_progress=false` means the active embedding space is fully
    /// covered (modal can close).
    pub pending_jobs: u64,
}

/// `memory_backfill_status` RPC handler (#1574 §4b). No inputs — reports
/// whether a per-model re-embed backfill is in flight so the UI can warn
/// the user that semantic recall is reduced until it drains.
pub async fn backfill_status_rpc(
    config: &Config,
) -> Result<RpcOutcome<BackfillStatusResponse>, String> {
    log::debug!("[memory::rpc] backfill_status: entry");
    // SQLite I/O off the async runtime thread, matching the sibling
    // DB-backed handlers in this module (`get_chunk_rpc`, etc.).
    let pending_jobs: u64 = tokio::task::spawn_blocking({
        let config = config.clone();
        move || {
            chunk_store::with_connection(&config, |conn| {
                let n: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM mem_tree_jobs
                      WHERE kind = 'reembed_backfill' AND status IN ('ready', 'running')",
                    [],
                    |r| r.get(0),
                )?;
                Ok(n.max(0) as u64)
            })
        }
    })
    .await
    .map_err(|e| format!("memory_backfill_status join error: {e}"))?
    .map_err(|e| {
        let msg = format!("memory_backfill_status: {e}");
        log::debug!("[memory::rpc] backfill_status: error: {msg}");
        msg
    })?;
    let in_progress = crate::openhuman::memory_queue::backfill_in_progress() || pending_jobs > 0;
    Ok(RpcOutcome::single_log(
        BackfillStatusResponse {
            in_progress,
            pending_jobs,
        },
        format!("memory_tree: backfill_status in_progress={in_progress} pending={pending_jobs}"),
    ))
}

// ── pipeline_status / set_enabled (#1856 Part 1) ─────────────────────────

/// Per-status counters for the `mem_tree_jobs` table — snapshot returned by
/// the `memory_tree_pipeline_status` RPC. Only the three states the status
/// panel surfaces are exposed; `done` / `cancelled` are intentionally
/// omitted to keep the wire payload small.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PipelineJobCounts {
    /// Jobs queued and waiting for a worker (`status = 'ready'`).
    pub ready: u64,
    /// Jobs currently being processed by a worker (`status = 'running'`).
    pub running: u64,
    /// Jobs that exhausted retries and remain in the table for diagnosis
    /// (`status = 'failed'`).
    pub failed: u64,
}

/// Response from the `memory_tree_pipeline_status` RPC (#1856 Part 1).
///
/// Aggregates "is the Memory Tree healthy?" signals into a single payload
/// the UI status panel can render without secondary fetches:
///
/// - `status` is a coarse, UI-shaped string (`running`/`paused`/`syncing`/
///   `error`/`idle`) derived from the other fields so the frontend stays
///   purely presentational.
/// - `wiki_size_bytes` is a recursive walk of the on-disk `wiki/` sub-tree
///   under the memory-tree content root; recomputed every call (cheap for
///   typical workspaces). The walk is scoped to `wiki/` so the figure
///   reflects the user-visible wiki only — not the sibling `raw/`,
///   `email/`, `chat/`, `document/` staging directories.
/// - `pipeline_jobs` is a snapshot of the queue — running > 0 implies
///   active sync, failed > 0 implies degraded.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PipelineStatusResponse {
    /// Aggregated status string: `running` | `paused` | `syncing` | `error`
    /// | `idle`. Derivation:
    /// 1. `is_paused` (scheduler-gate `off`) wins → `paused`.
    /// 2. otherwise failed > 0 → `error`.
    /// 3. otherwise running > 0 → `syncing`.
    /// 4. otherwise total_chunks > 0 → `running`.
    /// 5. otherwise → `idle`.
    pub status: String,
    /// Optional human-readable reason — populated when status is
    /// `paused` or `error`. `None` otherwise.
    pub reason: Option<String>,
    /// Epoch milliseconds of the most-recent chunk timestamp across all
    /// sources. Zero when the store is empty.
    pub last_sync_ms: i64,
    /// Total `mem_tree_chunks` rows across all sources.
    pub total_chunks: u64,
    /// Recursive byte size of the on-disk `wiki/` sub-tree under the
    /// memory-tree content root. Zero when the `wiki/` directory does not
    /// exist yet or cannot be read. Scoped to `wiki/` so the value matches
    /// the user-visible "Wiki size" tile (#1856 follow-up).
    pub wiki_size_bytes: u64,
    /// Snapshot counts from `mem_tree_jobs`.
    pub pipeline_jobs: PipelineJobCounts,
    /// Convenience flag: at least one job is currently `running`.
    pub is_syncing: bool,
    /// Convenience flag: scheduler-gate is in `off` mode, so all LLM-bound
    /// background work is paused cooperatively.
    pub is_paused: bool,
}

/// `memory_tree_pipeline_status` RPC handler (#1856 Part 1).
///
/// Aggregates `list_sources` + `count_by_status` + a recursive disk-size
/// probe into the [`PipelineStatusResponse`] the UI status panel renders.
/// All blocking work is dispatched onto `spawn_blocking` so the async
/// runtime isn't held during SQLite or filesystem I/O.
pub async fn pipeline_status_rpc(
    config: &Config,
) -> Result<RpcOutcome<PipelineStatusResponse>, String> {
    use crate::openhuman::config::SchedulerGateMode;
    use crate::openhuman::memory_queue::store as queue_store;
    use crate::openhuman::memory_queue::types::JobStatus;

    log::debug!("[memory-tree][rpc] pipeline_status: entry");

    // Chunk aggregates — total count + latest timestamp from
    // `mem_tree_chunks` in a single SQL round-trip so we don't materialise
    // the full source list just to sum two columns.
    let cfg_for_sources = config.clone();
    let (total_chunks, last_sync_ms) =
        tokio::task::spawn_blocking(move || -> Result<(u64, i64), String> {
            chunk_store::with_connection(&cfg_for_sources, |conn| {
                let (count, max_ts): (i64, Option<i64>) = conn.query_row(
                    "SELECT COUNT(*), MAX(timestamp_ms) FROM mem_tree_chunks",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                Ok((count.max(0) as u64, max_ts.unwrap_or(0).max(0)))
            })
            .map_err(|e| format!("chunk aggregates: {e:#}"))
        })
        .await
        .map_err(|e| {
            let msg = format!("pipeline_status join error: {e}");
            log::warn!("[memory-tree][rpc] pipeline_status: {msg}");
            msg
        })??;

    // Job counters — three parallel-safe blocking calls.
    let cfg_for_jobs = config.clone();
    let pipeline_jobs =
        tokio::task::spawn_blocking(move || -> Result<PipelineJobCounts, String> {
            let ready = queue_store::count_by_status(&cfg_for_jobs, JobStatus::Ready)
                .map_err(|e| format!("count_by_status(ready): {e:#}"))?;
            let running = queue_store::count_by_status(&cfg_for_jobs, JobStatus::Running)
                .map_err(|e| format!("count_by_status(running): {e:#}"))?;
            let failed = queue_store::count_by_status(&cfg_for_jobs, JobStatus::Failed)
                .map_err(|e| format!("count_by_status(failed): {e:#}"))?;
            Ok(PipelineJobCounts {
                ready,
                running,
                failed,
            })
        })
        .await
        .map_err(|e| {
            let msg = format!("pipeline_status job-count join error: {e}");
            log::warn!("[memory-tree][rpc] pipeline_status: {msg}");
            msg
        })??;

    // Disk size — best-effort. Permission errors etc. degrade to 0 with a
    // warn log rather than failing the whole RPC. Scoped to the `wiki/`
    // sub-directory so the tile lives up to its "Wiki size" label — the
    // sibling `raw/` / `email/` / `chat/` / `document/` staging directories
    // hold pre-canonicalised content and should not roll into the figure
    // surfaced to the user (#1856 CodeRabbit feedback).
    let wiki_root = config.memory_tree_content_root().join("wiki");
    let wiki_size_bytes = tokio::task::spawn_blocking(move || compute_dir_size_bytes(&wiki_root))
        .await
        .map_err(|e| {
            let msg = format!("pipeline_status size-walk join error: {e}");
            log::warn!("[memory-tree][rpc] pipeline_status: {msg}");
            msg
        })?;

    let is_paused = config.scheduler_gate.mode == SchedulerGateMode::Off;
    let is_syncing = pipeline_jobs.running > 0;

    let (status, reason) = derive_pipeline_status(
        is_paused,
        config.scheduler_gate.mode,
        is_syncing,
        pipeline_jobs.failed,
        total_chunks,
    );

    let payload = PipelineStatusResponse {
        status: status.clone(),
        reason: reason.clone(),
        last_sync_ms,
        total_chunks,
        wiki_size_bytes,
        pipeline_jobs,
        is_syncing,
        is_paused,
    };

    log::debug!(
        "[memory-tree][rpc] pipeline_status: ok status={status} total_chunks={total_chunks} wiki_size_bytes={wiki_size_bytes} ready={r} running={n} failed={f} reason={reason:?}",
        r = payload.pipeline_jobs.ready,
        n = payload.pipeline_jobs.running,
        f = payload.pipeline_jobs.failed,
    );

    Ok(RpcOutcome::single_log(
        payload,
        format!(
            "memory_tree: pipeline_status status={status} total_chunks={total_chunks} is_paused={is_paused} is_syncing={is_syncing}",
        ),
    ))
}

/// Recursive byte-count of files under `root`. Returns `0` when the root
/// does not exist or any traversal error occurs (best-effort; the status
/// panel is a UI convenience, not an audit surface).
fn compute_dir_size_bytes(root: &std::path::Path) -> u64 {
    if !root.exists() {
        return 0;
    }
    let mut total: u64 = 0;
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        match entry {
            Ok(e) if e.file_type().is_file() => {
                if let Ok(meta) = e.metadata() {
                    total = total.saturating_add(meta.len());
                }
            }
            Ok(_) => {}
            Err(err) => {
                // Both `err.path()` and `walkdir::Error`'s `Display` impl
                // embed the absolute on-disk path (which lives under the
                // user's home directory), so we redact: log only whether a
                // path was attached and the underlying `io::ErrorKind`.
                // That's enough for diagnosis while keeping the user's
                // workspace layout out of the log file.
                log::warn!(
                    "[memory-tree][rpc] pipeline_status: dir walk error has_path={} kind={:?}",
                    err.path().is_some(),
                    err.io_error().map(|e| e.kind())
                );
            }
        }
    }
    total
}

/// Pure derivation of `(status, reason)` from raw signals. Split out so the
/// unit tests can exercise the precedence rules without spinning up a
/// store.
fn derive_pipeline_status(
    is_paused: bool,
    mode: crate::openhuman::config::SchedulerGateMode,
    is_syncing: bool,
    failed: u64,
    total_chunks: u64,
) -> (String, Option<String>) {
    if is_paused {
        return (
            "paused".to_string(),
            Some(format!("scheduler gate mode = {}", mode.as_str())),
        );
    }
    if failed > 0 {
        return (
            "error".to_string(),
            Some(format!("{failed} failed job(s) in pipeline")),
        );
    }
    if is_syncing {
        return ("syncing".to_string(), None);
    }
    if total_chunks > 0 {
        return ("running".to_string(), None);
    }
    ("idle".to_string(), None)
}

/// Request shape for `memory_tree_set_enabled`. Single field — the caller
/// asks to enable (auto-mode) or pause (off-mode) all LLM-bound background
/// work.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetEnabledRequest {
    /// `true` ⇒ scheduler-gate mode becomes `auto`. `false` ⇒ `off`.
    pub enabled: bool,
}

/// Response shape for `memory_tree_set_enabled`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetEnabledResponse {
    /// Echo of the requested `enabled` state (post-write).
    pub enabled: bool,
    /// `true` when the saved mode actually flipped; `false` for no-ops.
    pub changed: bool,
    /// New scheduler-gate mode as wire string (`auto` / `off`).
    pub mode: String,
}

/// `memory_tree_set_enabled` RPC handler (#1856 Part 1).
///
/// Flips `config.scheduler_gate.mode` to either `Auto` (enabled) or `Off`
/// (paused), persists to disk via `config.save()`, and hot-reloads the
/// live scheduler-gate state so any in-flight workers immediately observe
/// the new policy at their next `wait_for_capacity()` await.
///
/// Notes:
/// - This is intentionally a single-field RPC (no batched
///   `MemoryTreeSettingsPatch`) — keeps the surface tight while #1856
///   Part 2 work lands the broader settings story.
/// - The 20-min Composio fetch loop is *not* paused by this toggle yet —
///   that requires a separate `Notify` signal and is queued for Part 2.
pub async fn set_enabled_rpc(
    config: &mut Config,
    req: SetEnabledRequest,
) -> Result<RpcOutcome<SetEnabledResponse>, String> {
    use crate::openhuman::config::SchedulerGateMode;

    let prev_mode = config.scheduler_gate.mode;
    let new_mode = if req.enabled {
        SchedulerGateMode::Auto
    } else {
        SchedulerGateMode::Off
    };

    log::debug!(
        "[memory-tree][rpc] set_enabled: requested enabled={} prev_mode={} new_mode={}",
        req.enabled,
        prev_mode.as_str(),
        new_mode.as_str(),
    );

    if prev_mode == new_mode {
        log::info!(
            "[memory-tree][rpc] set_enabled: no-op (mode already {})",
            new_mode.as_str()
        );
        return Ok(RpcOutcome::single_log(
            SetEnabledResponse {
                enabled: req.enabled,
                changed: false,
                mode: new_mode.as_str().to_string(),
            },
            format!(
                "memory_tree: set_enabled no-op enabled={} mode={}",
                req.enabled,
                new_mode.as_str()
            ),
        ));
    }

    config.scheduler_gate.mode = new_mode;
    config.save().await.map_err(|e| {
        let msg = format!("set_enabled: config.save failed: {e}");
        log::warn!("[memory-tree][rpc] {msg}");
        msg
    })?;

    // Hot-reload the live gate state — workers re-poll inside
    // `wait_for_capacity` and pick up the new policy without a restart.
    crate::openhuman::scheduler_gate::gate::update_config(config.scheduler_gate.clone());

    log::info!(
        "[memory-tree][rpc] set_enabled: scheduler_gate.mode {} -> {} (enabled={})",
        prev_mode.as_str(),
        new_mode.as_str(),
        req.enabled,
    );

    Ok(RpcOutcome::single_log(
        SetEnabledResponse {
            enabled: req.enabled,
            changed: true,
            mode: new_mode.as_str().to_string(),
        },
        format!(
            "memory_tree: set_enabled enabled={} mode={} changed=true",
            req.enabled,
            new_mode.as_str()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory_queue as jobs;
    use crate::openhuman::memory_queue::store::count_total;
    use crate::openhuman::memory_store::chunks::types::SourceKind;
    use crate::openhuman::memory_sync::canonicalize::document::DocumentInput;
    use chrono::{Duration as ChronoDuration, Utc};
    use serde_json::json;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    fn sample_document(title: &str, body: &str) -> DocumentInput {
        DocumentInput {
            provider: "notion".into(),
            title: title.into(),
            body: body.into(),
            modified_at: Utc::now(),
            source_ref: Some("notion://page/launch".into()),
        }
    }

    #[tokio::test]
    async fn ingest_document_roundtrip_lists_and_gets_chunks() {
        let (_tmp, cfg) = test_config();
        let outcome = ingest_rpc(
            &cfg,
            IngestRequest {
                source_kind: SourceKind::Document,
                source_id: "doc-launch".into(),
                owner: "alice".into(),
                tags: vec!["launch".into()],
                payload: serde_json::to_value(sample_document(
                    "Launch Plan",
                    "Phoenix launch canary checklist with rollback steps.",
                ))
                .unwrap(),
            },
        )
        .await
        .unwrap();
        assert_eq!(outcome.value.source_id, "doc-launch");
        assert_eq!(outcome.value.chunks_dropped, 0);
        assert!(!outcome.value.chunk_ids.is_empty());

        let listed = list_chunks_rpc(
            &cfg,
            ListChunksRequest {
                source_kind: Some("document".into()),
                source_id: Some("doc-launch".into()),
                owner: Some("alice".into()),
                limit: Some(10),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .chunks;
        assert_eq!(listed.len(), outcome.value.chunks_written);
        assert!(listed
            .iter()
            .all(|chunk| chunk.metadata.source_kind == SourceKind::Document));
        assert!(listed
            .iter()
            .any(|chunk| chunk.content.contains("Phoenix launch canary checklist")));

        let fetched = get_chunk_rpc(
            &cfg,
            GetChunkRequest {
                id: outcome.value.chunk_ids[0].clone(),
            },
        )
        .await
        .unwrap()
        .value
        .chunk
        .expect("chunk should exist");
        assert_eq!(fetched.id, outcome.value.chunk_ids[0]);
        assert_eq!(fetched.metadata.source_id, "doc-launch");
        assert_eq!(fetched.metadata.owner, "alice");
    }

    #[tokio::test]
    async fn ingest_document_is_idempotent_for_duplicate_source_id() {
        let (_tmp, cfg) = test_config();
        let req = IngestRequest {
            source_kind: SourceKind::Document,
            source_id: "doc-dup".into(),
            owner: "alice".into(),
            tags: vec![],
            payload: serde_json::to_value(sample_document("Launch Plan", "First body")).unwrap(),
        };

        let first = ingest_rpc(&cfg, req.clone()).await.unwrap().value;
        let second = ingest_rpc(&cfg, req).await.unwrap().value;
        assert!(first.chunks_written > 0);
        assert!(!first.already_ingested);
        assert_eq!(second.chunks_written, 0);
        assert!(second.already_ingested);

        let listed = list_chunks_rpc(
            &cfg,
            ListChunksRequest {
                source_id: Some("doc-dup".into()),
                limit: Some(10),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .chunks;
        assert_eq!(listed.len(), first.chunks_written);
    }

    #[tokio::test]
    async fn ingest_rpc_rejects_invalid_document_payload() {
        let (_tmp, cfg) = test_config();
        let err = ingest_rpc(
            &cfg,
            IngestRequest {
                source_kind: SourceKind::Document,
                source_id: "doc-invalid".into(),
                owner: String::new(),
                tags: vec![],
                payload: json!({"title": "Missing body"}),
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("invalid document payload"));
    }

    #[tokio::test]
    async fn list_chunks_rejects_unknown_source_kind() {
        let (_tmp, cfg) = test_config();
        let err = list_chunks_rpc(
            &cfg,
            ListChunksRequest {
                source_kind: Some("nonsense".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("unknown source kind: nonsense"));
    }

    #[tokio::test]
    async fn get_chunk_returns_none_for_missing_id() {
        let (_tmp, cfg) = test_config();
        let outcome = get_chunk_rpc(
            &cfg,
            GetChunkRequest {
                id: "missing-chunk".into(),
            },
        )
        .await
        .unwrap();
        assert!(outcome.value.chunk.is_none());
    }

    #[tokio::test]
    async fn trigger_digest_with_explicit_date_enqueues() {
        let (_tmp, cfg) = test_config();
        let req = TriggerDigestRequest {
            date_iso: Some("2026-04-27".into()),
        };
        let outcome = trigger_digest_rpc(&cfg, req).await.unwrap();
        let resp = outcome.value;
        assert!(resp.enqueued);
        assert!(resp.job_id.is_some());
        assert_eq!(resp.date_iso, "2026-04-27");
        assert_eq!(count_total(&cfg).unwrap(), 1);
    }

    #[tokio::test]
    async fn trigger_digest_with_no_date_defaults_to_yesterday() {
        let (_tmp, cfg) = test_config();
        let req = TriggerDigestRequest::default();
        let outcome = trigger_digest_rpc(&cfg, req).await.unwrap();
        let resp = outcome.value;
        assert!(resp.enqueued);
        let expected = (Utc::now().date_naive() - ChronoDuration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        assert_eq!(resp.date_iso, expected);
    }

    #[tokio::test]
    async fn trigger_digest_rejects_malformed_date() {
        let (_tmp, cfg) = test_config();
        let req = TriggerDigestRequest {
            date_iso: Some("not-a-date".into()),
        };
        let err = trigger_digest_rpc(&cfg, req).await.unwrap_err();
        assert!(
            err.contains("invalid date_iso"),
            "expected schema-shaped error message, got: {err}"
        );
        assert_eq!(count_total(&cfg).unwrap(), 0);
    }

    #[tokio::test]
    async fn trigger_digest_dedupes_active_jobs() {
        let (_tmp, cfg) = test_config();
        let req = TriggerDigestRequest {
            date_iso: Some("2026-04-27".into()),
        };
        let first = trigger_digest_rpc(&cfg, req.clone()).await.unwrap().value;
        let second = trigger_digest_rpc(&cfg, req).await.unwrap().value;
        assert!(first.enqueued);
        assert!(!second.enqueued, "duplicate must be dedupe-suppressed");
        assert!(second.job_id.is_none());
        assert_eq!(count_total(&cfg).unwrap(), 1);
    }

    /// #1574 §4b: `backfill_status_rpc` reports 0 pending on an idle space
    /// and reflects a queued `reembed_backfill` job (forcing `in_progress`).
    /// `in_progress` for the empty case is intentionally not asserted — the
    /// underlying flag is a process-global shared across parallel tests.
    #[tokio::test]
    async fn backfill_status_reports_pending_jobs() {
        let (_tmp, cfg) = test_config();

        let s0 = backfill_status_rpc(&cfg).await.unwrap().value;
        assert_eq!(s0.pending_jobs, 0, "idle space has no pending backfill");

        let job = jobs::types::NewJob::reembed_backfill(&jobs::types::ReembedBackfillPayload {
            signature: "provider=test;model=x;dims=1".into(),
        })
        .unwrap();
        jobs::enqueue(&cfg, &job).unwrap();

        let s1 = backfill_status_rpc(&cfg).await.unwrap().value;
        assert_eq!(
            s1.pending_jobs, 1,
            "a ready reembed_backfill job must count"
        );
        assert!(s1.in_progress, "pending>0 forces in_progress=true");
    }

    // ── pipeline_status / set_enabled (#1856 Part 1) ─────────────────────

    /// `derive_pipeline_status` precedence is locked in here so the UI can
    /// rely on the wire status string without re-deriving it from the raw
    /// counters.
    #[test]
    fn derive_pipeline_status_precedence_matches_spec() {
        use crate::openhuman::config::SchedulerGateMode;

        // paused beats everything else
        let (s, reason) = derive_pipeline_status(true, SchedulerGateMode::Off, true, 5, 100);
        assert_eq!(s, "paused");
        assert!(reason.unwrap().contains("off"));

        // error beats syncing / running / idle
        let (s, reason) = derive_pipeline_status(false, SchedulerGateMode::Auto, true, 2, 100);
        assert_eq!(s, "error");
        assert!(reason.unwrap().contains("2 failed"));

        // syncing beats running / idle
        let (s, reason) = derive_pipeline_status(false, SchedulerGateMode::Auto, true, 0, 100);
        assert_eq!(s, "syncing");
        assert!(reason.is_none());

        // running when chunks exist but nothing in flight
        let (s, _) = derive_pipeline_status(false, SchedulerGateMode::Auto, false, 0, 100);
        assert_eq!(s, "running");

        // idle when the store is empty and nothing is in flight
        let (s, _) = derive_pipeline_status(false, SchedulerGateMode::Auto, false, 0, 0);
        assert_eq!(s, "idle");
    }

    /// On a fresh workspace the panel must report `idle` with zero
    /// counters — the UI uses this to swap the loading skeleton for a
    /// "no memory yet" state.
    #[tokio::test]
    async fn pipeline_status_returns_idle_for_empty_store() {
        let (_tmp, cfg) = test_config();
        let out = pipeline_status_rpc(&cfg).await.unwrap().value;
        assert_eq!(out.status, "idle");
        assert_eq!(out.total_chunks, 0);
        assert_eq!(out.last_sync_ms, 0);
        assert_eq!(out.pipeline_jobs.ready, 0);
        assert_eq!(out.pipeline_jobs.running, 0);
        assert_eq!(out.pipeline_jobs.failed, 0);
        assert!(!out.is_syncing);
        assert!(!out.is_paused);
        assert_eq!(out.wiki_size_bytes, 0, "no content dir yet");
        assert!(out.reason.is_none());
    }

    /// When the scheduler gate is `off`, the aggregated status flips to
    /// `paused` regardless of the rest of the signals. This is the
    /// invariant the toggle relies on.
    #[tokio::test]
    async fn pipeline_status_reflects_paused_when_scheduler_off() {
        use crate::openhuman::config::SchedulerGateMode;

        let (_tmp, mut cfg) = test_config();
        cfg.scheduler_gate.mode = SchedulerGateMode::Off;
        let out = pipeline_status_rpc(&cfg).await.unwrap().value;
        assert_eq!(out.status, "paused");
        assert!(out.is_paused);
        let reason = out.reason.expect("paused must carry a reason");
        assert!(reason.contains("off"), "reason should name the mode");
    }

    /// `pipeline_status` reflects chunks that have been ingested — total
    /// count rolls up and `last_sync_ms` picks up the most-recent
    /// timestamp from `mem_tree_chunks`.
    #[tokio::test]
    async fn pipeline_status_reports_chunk_aggregates_after_ingest() {
        let (_tmp, cfg) = test_config();

        // Seed one document so `mem_tree_chunks` is non-empty.
        ingest_rpc(
            &cfg,
            IngestRequest {
                source_kind: SourceKind::Document,
                source_id: "doc-status".into(),
                owner: "alice".into(),
                tags: vec![],
                payload: serde_json::to_value(sample_document(
                    "Status",
                    "Pipeline status smoke document.",
                ))
                .unwrap(),
            },
        )
        .await
        .unwrap();

        let out = pipeline_status_rpc(&cfg).await.unwrap().value;
        assert!(out.total_chunks > 0, "ingest must populate chunk count");
        assert!(
            out.last_sync_ms > 0,
            "ingest must populate last_sync_ms (got {})",
            out.last_sync_ms
        );
        // No jobs running ⇒ running status, not syncing/error.
        assert_eq!(out.status, "running");
        assert!(!out.is_syncing);
    }

    /// `set_enabled` flips the persisted scheduler-gate mode and reports
    /// `changed=true`; calling it again with the same value is a no-op
    /// reporting `changed=false`. Uses an isolated `config_path` under
    /// the workspace tempdir so `config.save()` doesn't touch the
    /// host's real ~/.openhuman directory.
    #[tokio::test]
    async fn set_enabled_toggles_scheduler_gate_mode() {
        use crate::openhuman::config::SchedulerGateMode;

        let (tmp, mut cfg) = test_config();
        // Pin config_path inside the tempdir so `save()` stays sandboxed.
        cfg.config_path = tmp.path().join("config.toml");

        assert_eq!(cfg.scheduler_gate.mode, SchedulerGateMode::Auto);

        let off = set_enabled_rpc(&mut cfg, SetEnabledRequest { enabled: false })
            .await
            .unwrap()
            .value;
        assert!(!off.enabled);
        assert!(off.changed);
        assert_eq!(off.mode, "off");
        assert_eq!(cfg.scheduler_gate.mode, SchedulerGateMode::Off);

        // Calling with the same value must report no-op.
        let again = set_enabled_rpc(&mut cfg, SetEnabledRequest { enabled: false })
            .await
            .unwrap()
            .value;
        assert!(!again.changed, "duplicate toggle must be a no-op");

        // Flip back.
        let on = set_enabled_rpc(&mut cfg, SetEnabledRequest { enabled: true })
            .await
            .unwrap()
            .value;
        assert!(on.enabled);
        assert!(on.changed);
        assert_eq!(on.mode, "auto");
        assert_eq!(cfg.scheduler_gate.mode, SchedulerGateMode::Auto);
    }
}
