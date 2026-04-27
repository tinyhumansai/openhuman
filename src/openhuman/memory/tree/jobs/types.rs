//! Job types for the async memory-tree pipeline (#TBD).
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
}

impl JobKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobKind::ExtractChunk => "extract_chunk",
            JobKind::AppendBuffer => "append_buffer",
            JobKind::Seal => "seal",
            JobKind::TopicRoute => "topic_route",
            JobKind::DigestDaily => "digest_daily",
            JobKind::FlushStale => "flush_stale",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "extract_chunk" => JobKind::ExtractChunk,
            "append_buffer" => JobKind::AppendBuffer,
            "seal" => JobKind::Seal,
            "topic_route" => JobKind::TopicRoute,
            "digest_daily" => JobKind::DigestDaily,
            "flush_stale" => JobKind::FlushStale,
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
            JobKind::ExtractChunk | JobKind::Seal | JobKind::DigestDaily | JobKind::TopicRoute
        )
    }
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
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Ready => "ready",
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }

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
    /// Stringified id with kind prefix, suitable for dedupe-key composition.
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
    pub fn dedupe_key(&self, date_iso: &str) -> String {
        format!("flush_stale:{date_iso}")
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
    pub fn extract_chunk(p: &ExtractChunkPayload) -> Result<Self> {
        Ok(Self {
            kind: JobKind::ExtractChunk,
            payload_json: serde_json::to_string(p)?,
            dedupe_key: Some(p.dedupe_key()),
            available_at_ms: None,
            max_attempts: None,
        })
    }

    pub fn append_buffer(p: &AppendBufferPayload) -> Result<Self> {
        Ok(Self {
            kind: JobKind::AppendBuffer,
            payload_json: serde_json::to_string(p)?,
            dedupe_key: Some(p.dedupe_key()),
            available_at_ms: None,
            max_attempts: None,
        })
    }

    pub fn seal(p: &SealPayload) -> Result<Self> {
        Ok(Self {
            kind: JobKind::Seal,
            payload_json: serde_json::to_string(p)?,
            dedupe_key: Some(p.dedupe_key()),
            available_at_ms: None,
            max_attempts: None,
        })
    }

    pub fn topic_route(p: &TopicRoutePayload) -> Result<Self> {
        Ok(Self {
            kind: JobKind::TopicRoute,
            payload_json: serde_json::to_string(p)?,
            dedupe_key: Some(p.dedupe_key()),
            available_at_ms: None,
            max_attempts: None,
        })
    }

    pub fn digest_daily(p: &DigestDailyPayload) -> Result<Self> {
        Ok(Self {
            kind: JobKind::DigestDaily,
            payload_json: serde_json::to_string(p)?,
            dedupe_key: Some(p.dedupe_key()),
            available_at_ms: None,
            max_attempts: None,
        })
    }

    pub fn flush_stale(p: &FlushStalePayload, date_iso: &str) -> Result<Self> {
        Ok(Self {
            kind: JobKind::FlushStale,
            payload_json: serde_json::to_string(p)?,
            dedupe_key: Some(p.dedupe_key(date_iso)),
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
    fn llm_bound_kinds() {
        assert!(JobKind::ExtractChunk.is_llm_bound());
        assert!(JobKind::Seal.is_llm_bound());
        assert!(JobKind::DigestDaily.is_llm_bound());
        assert!(JobKind::TopicRoute.is_llm_bound());
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
}
