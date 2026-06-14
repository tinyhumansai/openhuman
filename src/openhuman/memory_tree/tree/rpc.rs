//! RPC handler functions for the memory tree layer.
//!
//! Public JSON-RPC surface:
//! - `openhuman.memory_tree_ingest` — one unified ingest. Caller supplies
//!   `source_kind` + generic JSON `payload` (adapter-specific). Internally
//!   dispatches to chat / email / document canonicalisers.
//! - `openhuman.memory_tree_list_chunks` — listing with filters.
//! - `openhuman.memory_tree_get_chunk` — single chunk fetch.

use rusqlite::OptionalExtension;
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
        source_scope: None,
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
    /// Aggregated status string: `running` | `paused` | `syncing` |
    /// `degraded` | `error` | `idle`. Derivation:
    /// 1. `is_paused` (scheduler-gate `off`) wins → `paused`.
    /// 2. otherwise failed > 0 → `error`.
    /// 3. otherwise degraded (#002, recall/structure reduced) → `degraded`.
    /// 4. otherwise running > 0 → `syncing`.
    /// 5. otherwise total_chunks > 0 → `running`.
    /// 6. otherwise → `idle`.
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
    /// #002 (FR-002/FR-004): "the pipeline ran but output quality is reduced"
    /// — `semantic_recall` true when embeddings were skipped (no usable
    /// provider, so recall falls back to recency), `structure` true when
    /// extraction yielded nothing across the board (empty wiki). Carries the
    /// typed `cause` so the UI can render an actionable remediation. Additive:
    /// `#[serde(default)]` keeps older clients deserialising the response.
    #[serde(default)]
    pub degraded: crate::openhuman::memory_tree::health::DegradedState,
    /// #002 (FR-004): the single first blocking/most-significant cause, as a
    /// typed failure with an i18n remediation key. Populated from a failed
    /// job's classified reason or the active degradation cause; `None` when
    /// the pipeline is healthy. The frontend renders this verbatim (resolving
    /// `remediation_key`) instead of re-deriving a cause from raw counters.
    #[serde(default)]
    pub first_blocking_cause: Option<crate::openhuman::memory_tree::health::PipelineFailure>,
    /// #002 (FR-010 / US5): fraction of chunks with ≥1 indexed entity, in
    /// `[0.0, 1.0]`. Near 0 with `total_chunks > 0` means extraction is
    /// producing no structure (the "empty-but-built wiki"). `None` when the
    /// metric could not be measured (DB read error) — deliberately distinct
    /// from a genuine `Some(0.0)` so the status surface never misreports a
    /// broken measurement path as a structure failure. Additive
    /// (`#[serde(default)]` → `None` for older clients).
    #[serde(default)]
    pub extraction_coverage: Option<f32>,
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

    // Job counters — parallel-safe blocking calls. `failed_unrecoverable` is the
    // #3365 left-right split: of the failed jobs, how many are the hard,
    // user-actionable kind (`failure_class = 'unrecoverable'`) vs transient ones
    // that self-heal via auto-requeue. Only the former escalates to `error`.
    let cfg_for_jobs = config.clone();
    let (pipeline_jobs, failed_unrecoverable) =
        tokio::task::spawn_blocking(move || -> Result<(PipelineJobCounts, u64), String> {
            let ready = queue_store::count_by_status(&cfg_for_jobs, JobStatus::Ready)
                .map_err(|e| format!("count_by_status(ready): {e:#}"))?;
            let running = queue_store::count_by_status(&cfg_for_jobs, JobStatus::Running)
                .map_err(|e| format!("count_by_status(running): {e:#}"))?;
            let failed = queue_store::count_by_status(&cfg_for_jobs, JobStatus::Failed)
                .map_err(|e| format!("count_by_status(failed): {e:#}"))?;
            let failed_unrecoverable = queue_store::count_failed_unrecoverable(&cfg_for_jobs)
                .map_err(|e| format!("count_failed_unrecoverable: {e:#}"))?;
            Ok((
                PipelineJobCounts {
                    ready,
                    running,
                    failed,
                },
                failed_unrecoverable,
            ))
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

    // #002: read the process-global degradation snapshot (set by the embed /
    // extract stages) so a half-working sync surfaces as `degraded` with a
    // cause rather than a misleading `running`. The structure-degraded latch is
    // a liveness signal ("the extraction model is timing out") kept honest at
    // its source in `extract::llm` — it self-clears on the next *completed*
    // extraction (#3365), so the status surface never consults the unrelated
    // `extraction_coverage` metric to second-guess it here.
    let degraded = crate::openhuman::memory_tree::health::current_degraded_state();

    let (status, reason) = derive_pipeline_status(
        is_paused,
        config.scheduler_gate.mode,
        is_syncing,
        pipeline_jobs.failed,
        failed_unrecoverable,
        total_chunks,
        &degraded,
    );

    // #002: both of these touch SQLite, so run them off the async runtime
    // thread in a single blocking task (a contended DB could otherwise pin a
    // Tokio worker for the busy-timeout window). Best-effort — failures degrade
    // to `None` rather than failing the polled status RPC.
    //   - first_blocking_cause (FR-004): the most-recent failed job's typed
    //     reason, surfaced verbatim by the UI.
    //   - extraction_coverage (FR-010/US5): fraction of chunks with structure,
    //     surfaced as its own display metric — deliberately NOT folded into the
    //     status pill (#3365: coverage is a cumulative measure, unrelated to the
    //     live structure-degraded liveness signal).
    //     `None` (not `0.0`) on a read error, so a broken measurement path is
    //     never mistaken for a genuine 0% extraction rate.
    let (latest_failure, extraction_coverage) = {
        let cfg = config.clone();
        tokio::task::spawn_blocking(move || {
            // Log-then-drop: keep the None fallback (these reads must not fail
            // the polled status RPC) but emit a grep-friendly diagnostic so a
            // DB/query failure is distinguishable from "no blocking cause" /
            // "metric unavailable by design".
            let failure = latest_failed_job_failure(&cfg).unwrap_or_else(|e| {
                log::warn!(
                    "[memory-tree][rpc] pipeline_status: latest_failed_job_failure read failed: {e:#}"
                );
                None
            });
            let coverage = crate::openhuman::memory_store::chunks::store::extraction_coverage(&cfg)
                .map_err(|e| {
                    log::warn!(
                        "[memory-tree][rpc] pipeline_status: extraction_coverage read failed: {e:#}"
                    );
                })
                .ok();
            (failure, coverage)
        })
        .await
        .unwrap_or_else(|e| {
            log::warn!("[memory-tree][rpc] pipeline_status: ancillary metrics join error: {e:#}");
            (None, None)
        })
    };

    // A hard failed-job reason is more urgent than a soft degradation; fall
    // back to the active degradation cause, then `None` when healthy.
    let first_blocking_cause = latest_failure.or_else(|| degraded.cause.clone());

    let payload = PipelineStatusResponse {
        status: status.clone(),
        reason: reason.clone(),
        last_sync_ms,
        total_chunks,
        wiki_size_bytes,
        pipeline_jobs,
        is_syncing,
        is_paused,
        degraded,
        first_blocking_cause,
        extraction_coverage,
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

/// `memory_tree_doctor` RPC handler (#002 FR-009). Runs the one-shot
/// pipeline diagnostic and returns the [`DoctorReport`] — per-stage health,
/// the first blocking cause, the degraded snapshot, and counters. Exposed for
/// the agent tool + CLI so the agent can self-diagnose an empty/stalled wiki.
/// Synchronous + cheap (config + queue counters + degraded flags), so no
/// blocking-pool dispatch is needed.
pub async fn doctor_rpc(
    config: &Config,
) -> Result<RpcOutcome<crate::openhuman::memory_tree::health::DoctorReport>, String> {
    // Offload the doctor's blocking SQLite reads off the async runtime thread.
    let report = crate::openhuman::memory_tree::health::async_run_doctor(config).await;
    let summary = if report.healthy {
        "memory_tree: doctor — healthy".to_string()
    } else {
        format!(
            "memory_tree: doctor — first_blocking_cause={}",
            report
                .first_blocking_cause
                .as_ref()
                .map(|f| f.code.as_str())
                .unwrap_or("unknown")
        )
    };
    Ok(RpcOutcome::single_log(report, summary))
}

/// Response from `memory_tree_retry_failed` (#002 FR-011).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetryFailedResponse {
    /// Number of `failed` jobs flipped back to `ready` for retry.
    pub requeued: u64,
}

/// `memory_tree_retry_failed` RPC handler (#002 FR-011). Flips every
/// terminally-`failed` `mem_tree_jobs` row back to `ready` (fresh attempt
/// budget, typed reason cleared) so jobs that failed under a now-fixed config
/// re-run without re-ingesting source data. Backs the "Retry failed" button.
pub async fn retry_failed_rpc(config: &Config) -> Result<RpcOutcome<RetryFailedResponse>, String> {
    let cfg = config.clone();
    let requeued = tokio::task::spawn_blocking(move || {
        crate::openhuman::memory_queue::store::requeue_failed(&cfg)
    })
    .await
    .map_err(|e| format!("retry_failed join error: {e}"))?
    .map_err(|e| format!("retry_failed: {e:#}"))?;
    // Wake the worker pool so the requeued jobs are picked up promptly.
    crate::openhuman::memory_queue::wake_workers();
    Ok(RpcOutcome::single_log(
        RetryFailedResponse { requeued },
        format!("memory_tree: retry_failed requeued={requeued}"),
    ))
}

/// #002 (FR-004): the typed [`PipelineFailure`] of the most-recently-failed
/// `mem_tree_jobs` row, when it carries a classified `failure_reason`. Returns
/// `Ok(None)` when there is no failed job with a typed reason (older failures
/// predating the typed-failure columns, or none at all). Best-effort: the
/// status panel is a UI convenience, so a DB error degrades to `Ok(None)`
/// rather than failing the whole status RPC.
fn latest_failed_job_failure(
    config: &Config,
) -> Result<Option<crate::openhuman::memory_tree::health::PipelineFailure>, String> {
    use crate::openhuman::memory_tree::health::{FailureClass, FailureCode, PipelineFailure};

    let row: Option<(Option<String>, Option<String>)> =
        chunk_store::with_connection(config, |conn| {
            conn.query_row(
                "SELECT failure_reason, failure_class FROM mem_tree_jobs
              WHERE status = 'failed' AND failure_reason IS NOT NULL
              ORDER BY completed_at_ms DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(Into::into)
        })
        .map_err(|e| format!("latest_failed_job_failure: {e:#}"))?;

    let Some((Some(reason), class)) = row else {
        return Ok(None);
    };
    let Some(code) = FailureCode::from_str(&reason) else {
        return Ok(None);
    };
    // Trust the persisted class when present and parseable; otherwise derive
    // from the code (keeps a forward-compatible default if the column is NULL
    // on an older row).
    let mut failure = PipelineFailure::new(code);
    if let Some(c) = class.as_deref() {
        if c == "transient" {
            failure.class = FailureClass::Transient;
        } else if c == "unrecoverable" {
            failure.class = FailureClass::Unrecoverable;
        }
    }
    Ok(Some(failure))
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
    failed_unrecoverable: u64,
    total_chunks: u64,
    degraded: &crate::openhuman::memory_tree::health::DegradedState,
) -> (String, Option<String>) {
    if is_paused {
        return (
            "paused".to_string(),
            Some(format!("scheduler gate mode = {}", mode.as_str())),
        );
    }
    // #3365: split the failed bucket by class. Only an UNRECOVERABLE failure
    // (budget / auth / dim-mismatch) is a hard `error` the user must act on —
    // it stays parked and can't self-heal. Transient failures are auto-requeued
    // by `requeue_transient_failed`, so they must NOT escalate to `error`; they
    // fall through to `degraded` ("failed, retrying") below. This fixes the prior
    // `failed > 0 → error` that flashed a scary error for a job about to retry.
    if failed_unrecoverable > 0 {
        return (
            "error".to_string(),
            Some(format!(
                "{failed_unrecoverable} unrecoverable failure(s) need action"
            )),
        );
    }
    // #002 (FR-005): "degraded" sits below error but above syncing/running —
    // the pipeline is making progress, but recall/structure is reduced (or some
    // jobs failed transiently and are retrying) and the user should be told why.
    // Beats syncing/running so a half-working sync isn't reported as plain
    // "running"/"syncing".
    //
    // Only fires when there are chunks: degraded recall/structure is only
    // meaningful when there's actual content affected. An empty workspace with
    // a misconfigured embedder should show "idle" (nothing to recall) rather
    // than "degraded" (recall is broken for existing content).
    //
    // `failed` here is transient-only — any unrecoverable failure returned
    // `error` above, so a non-zero `failed` at this point means jobs that will
    // be auto-requeued.
    if (degraded.is_degraded() || failed > 0) && total_chunks > 0 {
        let mut parts: Vec<String> = Vec::new();
        if degraded.semantic_recall {
            parts.push("semantic recall disabled".to_string());
        }
        if degraded.structure {
            parts.push("wiki structure incomplete".to_string());
        }
        if failed > 0 {
            parts.push(format!("{failed} job(s) failed, retrying"));
        }
        return ("degraded".to_string(), Some(parts.join("; ")));
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

    /// Regression #3568 / CORE-2K: chat payloads with RFC-3339 timestamps must
    /// be accepted — not rejected with "expected unix timestamp in milliseconds".
    #[tokio::test]
    async fn ingest_chat_accepts_rfc3339_timestamps() {
        let (_tmp, cfg) = test_config();
        let outcome = ingest_rpc(
            &cfg,
            IngestRequest {
                source_kind: SourceKind::Chat,
                source_id: "slack:#rfc3339-test".into(),
                owner: "alice".into(),
                tags: vec![],
                payload: json!({
                    "platform": "slack",
                    "channel_label": "#eng",
                    "messages": [
                        {
                            "author": "alice",
                            "timestamp": "2026-05-17T19:30:00Z",
                            "text": "planning the launch"
                        },
                        {
                            "author": "bob",
                            "timestamp": 1779046260000_i64,
                            "text": "confirmed"
                        }
                    ]
                }),
            },
        )
        .await
        .unwrap();
        assert!(!outcome.value.chunk_ids.is_empty());
    }

    /// Regression #3568 / CORE-2K: email payloads with RFC-3339 timestamps must
    /// be accepted.
    #[tokio::test]
    async fn ingest_email_accepts_rfc3339_timestamps() {
        let (_tmp, cfg) = test_config();
        let outcome = ingest_rpc(
            &cfg,
            IngestRequest {
                source_kind: SourceKind::Email,
                source_id: "gmail:rfc3339-test".into(),
                owner: "alice@example.com".into(),
                tags: vec![],
                payload: json!({
                    "provider": "gmail",
                    "thread_subject": "Launch",
                    "messages": [
                        {
                            "from": "bob@example.com",
                            "to": ["alice@example.com"],
                            "subject": "Launch",
                            "sent_at": "2026-05-17T19:30:00Z",
                            "body": "Let's ship this."
                        }
                    ]
                }),
            },
        )
        .await
        .unwrap();
        assert!(!outcome.value.chunk_ids.is_empty());
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
        use crate::openhuman::memory_tree::health::{DegradedState, FailureCode, PipelineFailure};

        let healthy = DegradedState::default();
        let recall_degraded = DegradedState {
            semantic_recall: true,
            structure: false,
            cause: Some(PipelineFailure::new(FailureCode::EmbeddingsUnconfigured)),
        };
        let structure_degraded = DegradedState {
            semantic_recall: false,
            structure: true,
            cause: Some(PipelineFailure::new(FailureCode::ExtractionTimeout)),
        };

        // Args: (is_paused, mode, is_syncing, failed, failed_unrecoverable,
        //        total_chunks, &degraded).

        // paused beats everything else (even degradation)
        let (s, reason) = derive_pipeline_status(
            true,
            SchedulerGateMode::Off,
            true,
            5,
            5,
            100,
            &recall_degraded,
        );
        assert_eq!(s, "paused");
        assert!(reason.unwrap().contains("off"));

        // error beats degraded / syncing / running / idle — but ONLY for
        // unrecoverable failures (#3365).
        let (s, reason) = derive_pipeline_status(
            false,
            SchedulerGateMode::Auto,
            true,
            2,
            2, // both failures unrecoverable
            100,
            &recall_degraded,
        );
        assert_eq!(s, "error");
        assert!(reason.unwrap().contains("unrecoverable"));

        // #3365: transient-only failures (failed > 0, none unrecoverable) do NOT
        // escalate to error — they self-heal via auto-requeue, so they surface
        // as `degraded` ("retrying"), beating syncing/running.
        let (s, reason) =
            derive_pipeline_status(false, SchedulerGateMode::Auto, true, 3, 0, 100, &healthy);
        assert_eq!(s, "degraded", "transient failures must not read as error");
        assert!(reason.unwrap().contains("3 job(s) failed, retrying"));

        // #002: degraded beats syncing / running / idle (but loses to paused/error)
        let (s, reason) = derive_pipeline_status(
            false,
            SchedulerGateMode::Auto,
            true, // syncing
            0,
            0,
            100,
            &recall_degraded,
        );
        assert_eq!(s, "degraded", "degraded must beat syncing");
        assert!(reason.unwrap().contains("semantic recall disabled"));

        let (s, reason) = derive_pipeline_status(
            false,
            SchedulerGateMode::Auto,
            false,
            0,
            0,
            100,
            &structure_degraded,
        );
        assert_eq!(s, "degraded");
        assert!(reason.unwrap().contains("wiki structure incomplete"));

        // syncing beats running / idle (when healthy)
        let (s, reason) =
            derive_pipeline_status(false, SchedulerGateMode::Auto, true, 0, 0, 100, &healthy);
        assert_eq!(s, "syncing");
        assert!(reason.is_none());

        // running when chunks exist but nothing in flight
        let (s, _) =
            derive_pipeline_status(false, SchedulerGateMode::Auto, false, 0, 0, 100, &healthy);
        assert_eq!(s, "running");

        // idle when the store is empty and nothing is in flight (transient
        // failures with no content don't manufacture a `degraded`).
        let (s, _) =
            derive_pipeline_status(false, SchedulerGateMode::Auto, false, 2, 0, 0, &healthy);
        assert_eq!(s, "idle");
    }

    /// On a fresh workspace the panel must report `idle` with zero
    /// counters — the UI uses this to swap the loading skeleton for a
    /// "no memory yet" state.
    #[tokio::test]
    async fn pipeline_status_returns_idle_for_empty_store() {
        // #002: the degraded flags are process-global; reset+serialise so a
        // parallel test (factory None-path, extract transport-fail) can't leak
        // a "degraded" signal into this fresh-workspace assertion.
        let _g = crate::openhuman::memory_tree::health::test_guard();
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
    /// timestamp from `mem_tree_chunks`. Depending on test environment provider
    /// availability, ingest may also mark semantic recall degraded; either way,
    /// the status must be terminally healthy/degraded rather than syncing/error.
    #[tokio::test]
    async fn pipeline_status_reports_chunk_aggregates_after_ingest() {
        // #002: reset+serialise the process-global degraded flags so this
        // "running" assertion isn't flipped to "degraded" by a parallel test.
        let _g = crate::openhuman::memory_tree::health::test_guard();
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
        // No jobs running. Provider availability differs between local and CI
        // harnesses, so a completed ingest may be fully running or degraded
        // because semantic recall or wiki structure was skipped. Both are
        // terminal, non-syncing states and both preserve the aggregate counters
        // asserted above.
        match out.status.as_str() {
            "running" => assert!(out.reason.is_none()),
            "degraded" => {
                let reason = out.reason.as_deref().unwrap_or_default();
                assert!(
                    reason.contains("semantic recall disabled")
                        || reason.contains("wiki structure incomplete"),
                    "degraded status should explain recall or structure loss: {:?}",
                    out.reason
                );
            }
            other => panic!("expected running or degraded after ingest, got {other}"),
        }
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
