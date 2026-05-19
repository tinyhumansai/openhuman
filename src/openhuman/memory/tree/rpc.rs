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
use crate::openhuman::memory::tree::canonicalize::{
    chat::ChatBatch, document::DocumentInput, email::EmailThread,
};
use crate::openhuman::memory::tree::ingest::{
    ingest_chat as do_ingest_chat, ingest_document as do_ingest_document,
    ingest_email as do_ingest_email, IngestResult,
};
use crate::openhuman::memory::tree::store::{self, ListChunksQuery};
use crate::openhuman::memory::tree::types::{Chunk, SourceKind};
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
        "[memory_tree::rpc] ingest kind={} source_id={}",
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
        move || store::list_chunks(&config, &query)
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
        move || store::get_chunk(&config, &id)
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
    use crate::openhuman::memory::tree::jobs;
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
    log::debug!("[memory_tree::rpc] backfill_status: entry");
    // SQLite I/O off the async runtime thread, matching the sibling
    // DB-backed handlers in this module (`get_chunk_rpc`, etc.).
    let pending_jobs: u64 = tokio::task::spawn_blocking({
        let config = config.clone();
        move || {
            store::with_connection(&config, |conn| {
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
        log::debug!("[memory_tree::rpc] backfill_status: error: {msg}");
        msg
    })?;
    let in_progress =
        crate::openhuman::memory::tree::jobs::backfill_in_progress() || pending_jobs > 0;
    Ok(RpcOutcome::single_log(
        BackfillStatusResponse {
            in_progress,
            pending_jobs,
        },
        format!("memory_tree: backfill_status in_progress={in_progress} pending={pending_jobs}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::jobs::store::count_total;
    use chrono::{Duration as ChronoDuration, Utc};
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
        use crate::openhuman::memory::tree::jobs;
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
}
