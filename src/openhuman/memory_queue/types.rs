//! Job types for the async memory-tree pipeline.
//!
//! Each `Job` row in `mem_tree_jobs` stores its discriminator as a string
//! `kind` plus a JSON-encoded `payload`. The strongly-typed payload structs
//! below own (de)serialisation; handlers parse the payload by branching on
//! [`JobKind`] and calling the matching `from_payload_json`.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// Discriminator persisted in `mem_tree_jobs.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobKind {
    /// Run LLM entity extraction over a single chunk and decide admission.
    ExtractChunk,
    /// Push an admitted chunk into a tree's L0 buffer.
    AppendBuffer,
    /// Seal exactly one buffer level; cascades enqueue a follow-up.
    Seal,
    /// Match a chunk's entities against active topic trees and enqueue
    /// per-topic `AppendBuffer` jobs.
    TopicRoute,
    /// Build the global tree's daily digest for a given UTC date.
    DigestDaily,
    /// Walk stale buffers and enqueue `Seal` jobs for any over the age cap.
    FlushStale,
    /// #1574 §6: re-embed a bounded batch of chunks/summaries that lack a
    /// vector at the active embedding signature (post model-switch, or the
    /// §7 dim-mismatch slice), then self-continue until none remain.
    ReembedBackfill,
}

impl JobKind {
    /// Snake-case wire string written to `mem_tree_jobs.kind`.
    pub fn as_str(&self) -> &'static str {
        match self {
            JobKind::ExtractChunk => "extract_chunk",
            JobKind::AppendBuffer => "append_buffer",
            JobKind::Seal => "seal",
            JobKind::TopicRoute => "topic_route",
            JobKind::DigestDaily => "digest_daily",
            JobKind::FlushStale => "flush_stale",
            JobKind::ReembedBackfill => "reembed_backfill",
        }
    }

    /// Inverse of [`Self::as_str`]; returns `Err` for unknown kinds.
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "extract_chunk" => JobKind::ExtractChunk,
            "append_buffer" => JobKind::AppendBuffer,
            "seal" => JobKind::Seal,
            "topic_route" => JobKind::TopicRoute,
            "digest_daily" => JobKind::DigestDaily,
            "flush_stale" => JobKind::FlushStale,
            "reembed_backfill" => JobKind::ReembedBackfill,
            other => return Err(anyhow!("unknown JobKind '{other}'")),
        })
    }

    /// True when handling this kind should hold a slot from the global
    /// LLM concurrency semaphore. `TopicRoute` is bound because
    /// `maybe_spawn_topic_tree → backfill_topic_tree` can transitively
    /// trigger summariser LLM calls when an entity first crosses the
    /// hotness threshold.
    pub fn is_llm_bound(&self) -> bool {
        matches!(
            self,
            JobKind::ExtractChunk
                | JobKind::Seal
                | JobKind::DigestDaily
                | JobKind::TopicRoute
                | JobKind::ReembedBackfill
        )
    }
}

/// Outcome of a successful handler run. Workers translate this into a
/// queue settlement: `Done` finalises the row, while `Defer` puts it back
/// to `ready` with `available_at_ms = until_ms` and **does not** count
/// toward the failure-attempt budget.
///
/// `Defer` exists so a handler that is transiently unable to make
/// progress (cloud rate-limited, dependency unavailable, model warming
/// up) can re-queue its job with a wake-up time without marking it
/// failed. Handlers should still surface real errors via `Err(_)` — that
/// path runs the existing exponential-backoff retry logic which **does**
/// burn the failure budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOutcome {
    /// Handler ran to completion. Row is settled as `done`.
    Done,
    /// Handler chose not to make progress yet. Row is rescheduled to
    /// `available_at_ms = until_ms` (UTC milliseconds) with `attempts`
    /// reverted to its pre-claim value so the failure budget is not
    /// touched. `reason` is recorded in `last_error` for visibility.
    Defer { until_ms: i64, reason: String },
}

/// Lifecycle states persisted on `mem_tree_jobs.status`. Workers transition
/// `ready → running → done|failed`. `Cancelled` is reserved for explicit
/// admin actions (none surfaced yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Ready,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobStatus {
    /// Snake-case wire string written to `mem_tree_jobs.status`.
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Ready => "ready",
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }

    /// Inverse of [`Self::as_str`]; returns `Err` for unknown values.
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "ready" => JobStatus::Ready,
            "running" => JobStatus::Running,
            "done" => JobStatus::Done,
            "failed" => JobStatus::Failed,
            "cancelled" => JobStatus::Cancelled,
            other => return Err(anyhow!("unknown JobStatus '{other}'")),
        })
    }

    /// True for `Done`, `Failed`, `Cancelled` — i.e. no further worker
    /// transitions are expected.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            JobStatus::Done | JobStatus::Failed | JobStatus::Cancelled
        )
    }
}

// ── Payloads ───────────────────────────────────────────────────────────────

/// Reference to either a leaf chunk or a sealed summary node. Used by
/// payloads that route content through the pipeline regardless of which
/// kind of source produced it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeRef {
    Leaf { chunk_id: String },
    Summary { summary_id: String },
}

impl NodeRef {
    /// Stringified id with kind prefix (`leaf:` or `summary:`), suitable
    /// for dedupe-key composition.
    pub fn dedupe_fragment(&self) -> String {
        match self {
            NodeRef::Leaf { chunk_id } => format!("leaf:{chunk_id}"),
            NodeRef::Summary { summary_id } => format!("summary:{summary_id}"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExtractChunkPayload {
    pub chunk_id: String,
}

impl ExtractChunkPayload {
    /// Stable dedupe key written to `mem_tree_jobs.dedupe_key` so a partial
    /// unique index can suppress in-flight duplicates.
    pub fn dedupe_key(&self) -> String {
        format!("extract:{}", self.chunk_id)
    }
}

/// Where an `AppendBuffer` job should land its node. Source-tree appends
/// are keyed by `source_id`; topic-tree appends are keyed by `tree_id`
/// because there can be many topic trees per node.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppendTarget {
    Source { source_id: String },
    Topic { tree_id: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppendBufferPayload {
    pub node: NodeRef,
    pub target: AppendTarget,
}

impl AppendBufferPayload {
    /// Stable dedupe key written to `mem_tree_jobs.dedupe_key` so a partial
    /// unique index can suppress in-flight duplicates.
    pub fn dedupe_key(&self) -> String {
        let node_part = self.node.dedupe_fragment();
        match &self.target {
            AppendTarget::Source { source_id } => {
                format!("append:source:{source_id}:{node_part}")
            }
            AppendTarget::Topic { tree_id } => {
                format!("append:topic:{tree_id}:{node_part}")
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SealPayload {
    pub tree_id: String,
    pub level: u32,
    /// When `Some`, the seal handler bypasses the buffer-budget check and
    /// force-seals — used by the time-based flush path. The wall-clock is
    /// passed through so the seal stamps a deterministic `sealed_at`.
    pub force_now_ms: Option<i64>,
}

impl SealPayload {
    /// Stable dedupe key written to `mem_tree_jobs.dedupe_key` so a partial
    /// unique index can suppress in-flight duplicates.
    pub fn dedupe_key(&self) -> String {
        // Active seal-job uniqueness is enforced per (tree, level): a seal
        // already in flight suppresses duplicate enqueues. Once the job
        // completes the partial index releases the key, so the next time
        // the buffer crosses its gate a fresh seal can be enqueued.
        format!("seal:{}:{}", self.tree_id, self.level)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TopicRoutePayload {
    pub node: NodeRef,
}

impl TopicRoutePayload {
    /// Stable dedupe key written to `mem_tree_jobs.dedupe_key` so a partial
    /// unique index can suppress in-flight duplicates.
    pub fn dedupe_key(&self) -> String {
        format!("topic_route:{}", self.node.dedupe_fragment())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DigestDailyPayload {
    /// UTC calendar date in `YYYY-MM-DD` form. Stored as a string so the
    /// dedupe key doesn't need to know about chrono.
    pub date_iso: String,
}

impl DigestDailyPayload {
    /// Stable dedupe key written to `mem_tree_jobs.dedupe_key` so a partial
    /// unique index can suppress in-flight duplicates.
    pub fn dedupe_key(&self) -> String {
        format!("digest_daily:{}", self.date_iso)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct FlushStalePayload {
    /// Override the configured `DEFAULT_FLUSH_AGE_SECS`. Optional so the
    /// scheduler can enqueue with `None` and let the handler use the
    /// configured default.
    pub max_age_secs: Option<i64>,
}

impl FlushStalePayload {
    /// Dedupe key scoped to a 3-hour UTC block (`hour_block = hour / 3`,
    /// 0..=7) so flush_stale runs up to 8× per day. Without this,
    /// low-volume sources wait a full day between seal opportunities.
    ///
    /// Pure: both `date_iso` and `hour_block` are supplied by the caller
    /// from a single `Utc::now()` reading, which keeps the key
    /// deterministic in tests and avoids a 3-hour-boundary race where
    /// the caller's `today_iso` could disagree with a second
    /// `Utc::now()` taken inside this function.
    pub fn dedupe_key(&self, date_iso: &str, hour_block: u32) -> String {
        format!("flush_stale:{date_iso}-h{hour_block}")
    }
}

/// #1574 §6 re-embed backfill. One chain per embedding signature: the
/// `dedupe_key` is the signature, so re-triggering while a chain is
/// in-flight is correctly suppressed (exactly one backfill per space).
/// The handler self-continues via `JobOutcome::Defer` (reschedules this
/// same row) rather than re-enqueuing, so the fixed dedupe key is safe.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReembedBackfillPayload {
    /// The embedding signature this chain re-embeds under. If the active
    /// signature has since changed, the handler treats this job as stale
    /// and finishes (a fresh chain for the new signature takes over).
    pub signature: String,
}

impl ReembedBackfillPayload {
    /// Stable dedupe key — one in-flight backfill chain per signature.
    pub fn dedupe_key(&self) -> String {
        format!("reembed_backfill:{}", self.signature)
    }
}

/// One row in `mem_tree_jobs`. `payload_json` is left as a raw string so
/// callers parse it lazily based on `kind`.
#[derive(Clone, Debug)]
pub struct Job {
    pub id: String,
    pub kind: JobKind,
    pub payload_json: String,
    pub dedupe_key: Option<String>,
    pub status: JobStatus,
    pub attempts: u32,
    pub max_attempts: u32,
    pub available_at_ms: i64,
    pub locked_until_ms: Option<i64>,
    pub last_error: Option<String>,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
}

/// Caller-side bundle for `enqueue` — `Job` minus the persistence-only
/// columns. Keeps producers from having to mint timestamps and ids by hand.
#[derive(Clone, Debug)]
pub struct NewJob {
    pub kind: JobKind,
    pub payload_json: String,
    pub dedupe_key: Option<String>,
    /// `None` means "available immediately." Set this for delayed jobs
    /// (retries, scheduled work).
    pub available_at_ms: Option<i64>,
    pub max_attempts: Option<u32>,
}

impl NewJob {
    /// Build an [`JobKind::ExtractChunk`] enqueue request.
    pub fn extract_chunk(p: &ExtractChunkPayload) -> Result<Self> {
        Ok(Self {
            kind: JobKind::ExtractChunk,
            payload_json: serde_json::to_string(p)?,
            dedupe_key: Some(p.dedupe_key()),
            available_at_ms: None,
            max_attempts: None,
        })
    }

    /// Build an [`JobKind::AppendBuffer`] enqueue request.
    pub fn append_buffer(p: &AppendBufferPayload) -> Result<Self> {
        Ok(Self {
            kind: JobKind::AppendBuffer,
            payload_json: serde_json::to_string(p)?,
            dedupe_key: Some(p.dedupe_key()),
            available_at_ms: None,
            max_attempts: None,
        })
    }

    /// Build an [`JobKind::Seal`] enqueue request.
    pub fn seal(p: &SealPayload) -> Result<Self> {
        Ok(Self {
            kind: JobKind::Seal,
            payload_json: serde_json::to_string(p)?,
            dedupe_key: Some(p.dedupe_key()),
            available_at_ms: None,
            max_attempts: None,
        })
    }

    /// Build an [`JobKind::TopicRoute`] enqueue request.
    pub fn topic_route(p: &TopicRoutePayload) -> Result<Self> {
        Ok(Self {
            kind: JobKind::TopicRoute,
            payload_json: serde_json::to_string(p)?,
            dedupe_key: Some(p.dedupe_key()),
            available_at_ms: None,
            max_attempts: None,
        })
    }

    /// Build an [`JobKind::DigestDaily`] enqueue request.
    pub fn digest_daily(p: &DigestDailyPayload) -> Result<Self> {
        Ok(Self {
            kind: JobKind::DigestDaily,
            payload_json: serde_json::to_string(p)?,
            dedupe_key: Some(p.dedupe_key()),
            available_at_ms: None,
            max_attempts: None,
        })
    }

    /// Build a [`JobKind::FlushStale`] enqueue request scoped to a
    /// 3-hour UTC block. Callers compute `date_iso` and `hour_block`
    /// from a single `Utc::now()` reading so the dedupe key is
    /// boundary-safe; see [`FlushStalePayload::dedupe_key`].
    pub fn flush_stale(p: &FlushStalePayload, date_iso: &str, hour_block: u32) -> Result<Self> {
        Ok(Self {
            kind: JobKind::FlushStale,
            payload_json: serde_json::to_string(p)?,
            dedupe_key: Some(p.dedupe_key(date_iso, hour_block)),
            available_at_ms: None,
            max_attempts: None,
        })
    }

    /// Build a [`JobKind::ReembedBackfill`] enqueue request (#1574 §6).
    pub fn reembed_backfill(p: &ReembedBackfillPayload) -> Result<Self> {
        Ok(Self {
            kind: JobKind::ReembedBackfill,
            payload_json: serde_json::to_string(p)?,
            dedupe_key: Some(p.dedupe_key()),
            available_at_ms: None,
            max_attempts: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_kind_roundtrip() {
        for k in [
            JobKind::ExtractChunk,
            JobKind::AppendBuffer,
            JobKind::Seal,
            JobKind::TopicRoute,
            JobKind::DigestDaily,
            JobKind::FlushStale,
            JobKind::ReembedBackfill,
        ] {
            assert_eq!(JobKind::parse(k.as_str()).unwrap(), k);
        }
    }

    #[test]
    fn job_status_terminality() {
        assert!(!JobStatus::Ready.is_terminal());
        assert!(!JobStatus::Running.is_terminal());
        assert!(JobStatus::Done.is_terminal());
        assert!(JobStatus::Failed.is_terminal());
        assert!(JobStatus::Cancelled.is_terminal());
    }

    #[test]
    fn dedupe_keys_distinguish_targets() {
        let p_src = AppendBufferPayload {
            node: NodeRef::Leaf {
                chunk_id: "c1".into(),
            },
            target: AppendTarget::Source {
                source_id: "slack:#eng".into(),
            },
        };
        let p_topic = AppendBufferPayload {
            node: NodeRef::Leaf {
                chunk_id: "c1".into(),
            },
            target: AppendTarget::Topic {
                tree_id: "topic:abc".into(),
            },
        };
        assert_ne!(p_src.dedupe_key(), p_topic.dedupe_key());
    }

    #[test]
    fn dedupe_keys_distinguish_node_kinds() {
        let p_leaf = AppendBufferPayload {
            node: NodeRef::Leaf {
                chunk_id: "x".into(),
            },
            target: AppendTarget::Topic {
                tree_id: "t".into(),
            },
        };
        let p_summary = AppendBufferPayload {
            node: NodeRef::Summary {
                summary_id: "x".into(),
            },
            target: AppendTarget::Topic {
                tree_id: "t".into(),
            },
        };
        assert_ne!(p_leaf.dedupe_key(), p_summary.dedupe_key());

        let r_leaf = TopicRoutePayload {
            node: NodeRef::Leaf {
                chunk_id: "x".into(),
            },
        };
        let r_summary = TopicRoutePayload {
            node: NodeRef::Summary {
                summary_id: "x".into(),
            },
        };
        assert_ne!(r_leaf.dedupe_key(), r_summary.dedupe_key());
    }

    #[test]
    fn flush_stale_dedupe_key_is_pure_and_per_3h_block() {
        let p = FlushStalePayload::default();
        // Same (date, block) → same key.
        assert_eq!(p.dedupe_key("2026-05-19", 2), p.dedupe_key("2026-05-19", 2));
        // Different block within same day → distinct keys (8 buckets/day).
        assert_ne!(p.dedupe_key("2026-05-19", 2), p.dedupe_key("2026-05-19", 3));
        // Different day, same block → distinct keys.
        assert_ne!(p.dedupe_key("2026-05-19", 2), p.dedupe_key("2026-05-20", 2));
        // Shape sanity.
        assert_eq!(p.dedupe_key("2026-05-19", 0), "flush_stale:2026-05-19-h0");
        assert_eq!(p.dedupe_key("2026-05-19", 7), "flush_stale:2026-05-19-h7");
    }

    #[test]
    fn llm_bound_kinds() {
        assert!(JobKind::ExtractChunk.is_llm_bound());
        assert!(JobKind::Seal.is_llm_bound());
        assert!(JobKind::DigestDaily.is_llm_bound());
        assert!(JobKind::TopicRoute.is_llm_bound());
        assert!(JobKind::ReembedBackfill.is_llm_bound());
        assert!(!JobKind::AppendBuffer.is_llm_bound());
        assert!(!JobKind::FlushStale.is_llm_bound());
    }

    #[test]
    fn node_ref_serializes_with_kind_tag() {
        let leaf = NodeRef::Leaf {
            chunk_id: "x".into(),
        };
        let s = serde_json::to_string(&leaf).unwrap();
        assert!(s.contains("\"kind\":\"leaf\""));
        let back: NodeRef = serde_json::from_str(&s).unwrap();
        assert_eq!(back, leaf);
    }

    #[test]
    fn append_target_serializes_with_kind_tag() {
        let p = AppendTarget::Source {
            source_id: "x".into(),
        };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"kind\":\"source\""));
        assert!(s.contains("\"source_id\":\"x\""));
        let back: AppendTarget = serde_json::from_str(&s).unwrap();
        match back {
            AppendTarget::Source { source_id } => assert_eq!(source_id, "x"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn new_job_extract_chunk_builder_sets_kind_payload_and_dedupe_key() {
        let payload = ExtractChunkPayload {
            chunk_id: "chunk-123".into(),
        };
        let job = NewJob::extract_chunk(&payload).unwrap();
        assert_eq!(job.kind, JobKind::ExtractChunk);
        assert_eq!(job.dedupe_key.as_deref(), Some("extract:chunk-123"));
        assert_eq!(job.available_at_ms, None);
        assert_eq!(job.max_attempts, None);
        let roundtrip: ExtractChunkPayload = serde_json::from_str(&job.payload_json).unwrap();
        assert_eq!(roundtrip.chunk_id, "chunk-123");
    }

    #[test]
    fn new_job_append_buffer_builder_uses_payload_dedupe_key() {
        let payload = AppendBufferPayload {
            node: NodeRef::Summary {
                summary_id: "summary-9".into(),
            },
            target: AppendTarget::Topic {
                tree_id: "topic:ops".into(),
            },
        };
        let job = NewJob::append_buffer(&payload).unwrap();
        assert_eq!(job.kind, JobKind::AppendBuffer);
        assert_eq!(
            job.dedupe_key.as_deref(),
            Some("append:topic:topic:ops:summary:summary-9")
        );
        let roundtrip: AppendBufferPayload = serde_json::from_str(&job.payload_json).unwrap();
        assert_eq!(roundtrip.dedupe_key(), payload.dedupe_key());
    }

    #[test]
    fn new_job_flush_stale_builder_uses_supplied_time_bucket() {
        let payload = FlushStalePayload {
            max_age_secs: Some(600),
        };
        let job = NewJob::flush_stale(&payload, "2026-05-24", 4).unwrap();
        assert_eq!(job.kind, JobKind::FlushStale);
        assert_eq!(job.dedupe_key.as_deref(), Some("flush_stale:2026-05-24-h4"));
        let roundtrip: FlushStalePayload = serde_json::from_str(&job.payload_json).unwrap();
        assert_eq!(roundtrip.max_age_secs, Some(600));
    }

    #[test]
    fn new_job_reembed_backfill_builder_is_one_chain_per_signature() {
        let payload = ReembedBackfillPayload {
            signature: "embed-v2".into(),
        };
        let job = NewJob::reembed_backfill(&payload).unwrap();
        assert_eq!(job.kind, JobKind::ReembedBackfill);
        assert_eq!(job.dedupe_key.as_deref(), Some("reembed_backfill:embed-v2"));
        let roundtrip: ReembedBackfillPayload = serde_json::from_str(&job.payload_json).unwrap();
        assert_eq!(roundtrip.signature, "embed-v2");
    }
}
