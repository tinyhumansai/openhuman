//! Core types for Phase 3a — summary trees, per-source bucket-seal (#709).
//!
//! These types sit on top of Phase 1's chunk leaves. A [`Tree`] groups leaves
//! under one scope (e.g. one chat channel, one email account). When a
//! [`Buffer`] at some level accumulates enough tokens, its contents seal
//! into a [`SummaryNode`] at level+1 and the buffer clears. Summary nodes
//! are immutable once emitted — updates to children use the Phase 1/2
//! tombstone pattern, never rewrite parents.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What kind of tree this is. Source trees live per ingest source; topic
/// and global trees are introduced in Phase 3b/3c and share the same
/// schema.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TreeKind {
    /// One tree per ingest source (e.g. `chat:slack:#eng`, `email:gmail:user`).
    Source,
    /// Reserved for Phase 3c — per-entity/topic tree.
    Topic,
    /// Reserved for Phase 3b — cross-source daily digest tree.
    Global,
}

impl TreeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Topic => "topic",
            Self::Global => "global",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "source" => Ok(Self::Source),
            "topic" => Ok(Self::Topic),
            "global" => Ok(Self::Global),
            other => Err(format!("unknown tree kind: {other}")),
        }
    }
}

/// Activity state of a tree. Archived trees stay queryable but don't accept
/// new leaves — used by Phase 3c when a topic tree's entity goes cold.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeStatus {
    Active,
    Archived,
}

impl TreeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "active" => Ok(Self::Active),
            "archived" => Ok(Self::Archived),
            other => Err(format!("unknown tree status: {other}")),
        }
    }
}

/// One summary-tree instance.
///
/// `root_id` is `None` until the first seal emits an L1 node. `max_level`
/// tracks the highest level that has ever sealed; `root_id` points at the
/// current top node at that level (changes on root-split).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Tree {
    pub id: String,
    pub kind: TreeKind,
    /// Logical identifier for what the tree covers. Format conventions:
    /// - Source: `<source_kind>:<provider>:<source_id>` or the chunk's
    ///   `source_id` directly (Phase 3a uses the chunk source_id verbatim)
    /// - Topic: canonical entity id
    /// - Global: the literal string `"global"`
    pub scope: String,
    pub root_id: Option<String>,
    pub max_level: u32,
    pub status: TreeStatus,
    pub created_at: DateTime<Utc>,
    pub last_sealed_at: Option<DateTime<Utc>>,
}

/// A sealed summary node — one level above raw leaves.
///
/// `child_ids` points at the concrete children that were in the buffer when
/// this node sealed. For L1 nodes those are leaf `chunk.id`s; for L2+ they
/// are lower-level summary ids. Relation is fixed at seal time — never
/// modified afterwards.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SummaryNode {
    pub id: String,
    pub tree_id: String,
    pub tree_kind: TreeKind,
    /// 1 for summaries over raw leaves, 2 over L1 summaries, and so on.
    pub level: u32,
    pub parent_id: Option<String>,
    pub child_ids: Vec<String>,
    /// Summariser output. Typical target: 800–1500 tokens.
    pub content: String,
    pub token_count: u32,
    /// Curated subset of children's entity canonical-ids.
    pub entities: Vec<String>,
    /// Curated topic labels (hashtag-like short phrases).
    pub topics: Vec<String>,
    pub time_range_start: DateTime<Utc>,
    pub time_range_end: DateTime<Utc>,
    /// Max of children's scores at seal time — cheap heuristic, preserved
    /// for reranking in Phase 4.
    pub score: f32,
    pub sealed_at: DateTime<Utc>,
    /// Tombstone flag — stays `false` in Phase 3a since summaries are
    /// immutable. Reserved for future cleanup passes (e.g. archive cascade).
    pub deleted: bool,
    /// Phase 4 (#710): summary content embedding for semantic rerank.
    ///
    /// `Some` on new seals — populated before the write tx opens so a
    /// failed embed aborts the seal (see `bucket_seal::seal_one_level`).
    /// `None` on legacy summaries sealed before Phase 4, or on reads
    /// where the blob column is NULL. Retrieval tolerates `None` by
    /// dropping those rows to the bottom of semantic rerank results.
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,
}

/// Unsealed frontier at a given `(tree_id, level)`. One row per level per
/// tree. `oldest_at` is `None` when the buffer is empty; used by the
/// time-based flush trigger.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Buffer {
    pub tree_id: String,
    pub level: u32,
    pub item_ids: Vec<String>,
    pub token_sum: i64,
    pub oldest_at: Option<DateTime<Utc>>,
}

impl Buffer {
    /// Empty buffer at the given key.
    pub fn empty(tree_id: &str, level: u32) -> Self {
        Self {
            tree_id: tree_id.to_string(),
            level,
            item_ids: Vec::new(),
            token_sum: 0,
            oldest_at: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.item_ids.is_empty()
    }

    /// Whether the buffer's oldest item is older than `max_age`. Returns
    /// `false` for an empty buffer.
    pub fn is_stale(&self, now: DateTime<Utc>, max_age: chrono::Duration) -> bool {
        match self.oldest_at {
            Some(ts) => now.signed_duration_since(ts) > max_age,
            None => false,
        }
    }
}

/// Token ceiling for one summariser invocation — aligned with the Phase 1
/// chunker ceiling so a single leaf never busts a seal on its own.
pub const TOKEN_BUDGET: u32 = 10_000;

/// Default age at which a non-empty buffer is force-sealed even under the
/// token budget. Keeps recent activity from stalling waiting for more
/// leaves that may never arrive.
pub const DEFAULT_FLUSH_AGE_SECS: i64 = 7 * 24 * 60 * 60;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_kind_round_trip() {
        for k in [TreeKind::Source, TreeKind::Topic, TreeKind::Global] {
            assert_eq!(TreeKind::parse(k.as_str()).unwrap(), k);
        }
        assert!(TreeKind::parse("bogus").is_err());
    }

    #[test]
    fn tree_status_round_trip() {
        for s in [TreeStatus::Active, TreeStatus::Archived] {
            assert_eq!(TreeStatus::parse(s.as_str()).unwrap(), s);
        }
        assert!(TreeStatus::parse("live").is_err());
    }

    #[test]
    fn empty_buffer_is_not_stale() {
        let b = Buffer::empty("t1", 0);
        assert!(b.is_empty());
        assert!(!b.is_stale(Utc::now(), chrono::Duration::zero()));
    }

    #[test]
    fn stale_buffer_detected() {
        let past = Utc::now() - chrono::Duration::hours(10);
        let b = Buffer {
            tree_id: "t1".into(),
            level: 0,
            item_ids: vec!["leaf-1".into()],
            token_sum: 100,
            oldest_at: Some(past),
        };
        assert!(b.is_stale(Utc::now(), chrono::Duration::hours(1)));
        assert!(!b.is_stale(Utc::now(), chrono::Duration::hours(20)));
    }
}
