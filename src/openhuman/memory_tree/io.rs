//! Canonical input/output types for the memory_tree module.
//!
//! memory_tree exposes two fundamental operations against any tree:
//!
//! - **Write** — append a chunk (leaf) into a tree; cascading bucket-seals
//!   may produce new summary nodes at higher levels.
//! - **Read**  — navigate from a node into its descendants, optionally
//!   reranked by a query embedding.
//!
//! Internal mechanics (bucket_seal, flush, walk, summarise) take a mix of
//! `&Tree`, `&LeafRef`, `WalkOptions`, etc. This module defines a single
//! pair of contract types per direction so callers above memory_tree
//! (orchestrator, jobs, RPC) can talk to the module in one consistent
//! shape regardless of which tree kind they're targeting.
//!
//! These are pure contract types — no logic, no IO, no storage. They
//! compose the existing primitives from [`crate::openhuman::memory_store::trees`]
//! and the mechanics submodules; convert at the call boundary.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::openhuman::memory_store::trees::{Tree, TreeKind};
use crate::openhuman::memory_tree::tree::bucket_seal::LeafRef;

// ───────────────────────── Write ─────────────────────────

/// A leaf payload ready to be appended to a tree. Mirror of [`LeafRef`]
/// with serde derives so RPC callers and job payloads can share one shape.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeLeafPayload {
    pub chunk_id: String,
    pub token_count: u32,
    pub timestamp: DateTime<Utc>,
    pub content: String,
    #[serde(default)]
    pub entities: Vec<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub score: f32,
}

impl From<&TreeLeafPayload> for LeafRef {
    fn from(p: &TreeLeafPayload) -> Self {
        LeafRef {
            chunk_id: p.chunk_id.clone(),
            token_count: p.token_count,
            timestamp: p.timestamp,
            content: p.content.clone(),
            entities: p.entities.clone(),
            topics: p.topics.clone(),
            score: p.score,
        }
    }
}

impl From<LeafRef> for TreeLeafPayload {
    fn from(l: LeafRef) -> Self {
        Self {
            chunk_id: l.chunk_id,
            token_count: l.token_count,
            timestamp: l.timestamp,
            content: l.content,
            entities: l.entities,
            topics: l.topics,
            score: l.score,
        }
    }
}

/// How sealed summaries should be labelled with entities/topics. Mirrors
/// [`crate::openhuman::memory_tree::tree::bucket_seal::LabelStrategy`] in a
/// serde-friendly shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeLabelStrategy {
    /// Inherit entities/topics from child leaves (default).
    #[default]
    Inherit,
    /// Re-extract from the summary text via the resolver.
    Extract,
    /// Leave entities/topics empty.
    Empty,
}

/// Canonical write request: "append this leaf to this tree".
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeWriteRequest {
    /// Target tree id. Must already exist (callers go through the
    /// kind-specific registry to get one).
    pub tree_id: String,
    /// Tree kind — informational, carried through to log lines and
    /// downstream policy. Storage doesn't branch on it.
    pub tree_kind: TreeKind,
    /// The leaf to append.
    pub leaf: TreeLeafPayload,
    /// Labelling strategy applied to any summaries that seal during this
    /// call. Defaults to [`TreeLabelStrategy::Inherit`].
    #[serde(default)]
    pub label_strategy: TreeLabelStrategy,
    /// When `true`, only stage the leaf in the L0 buffer; do NOT cascade
    /// seals synchronously. Use this from job-driven pipelines where seal
    /// work is enqueued separately. Default `false`.
    #[serde(default)]
    pub deferred: bool,
}

/// Canonical write outcome: which buffers sealed (if any) and the summary
/// ids produced.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TreeWriteOutcome {
    /// Ids of summary nodes that sealed during this append. Empty when the
    /// L0 buffer was below budget or the call was deferred.
    pub new_summary_ids: Vec<String>,
    /// Set to `true` when the caller used `deferred = true` and should
    /// enqueue a follow-up seal job for level 0.
    pub seal_pending: bool,
}

// ───────────────────────── Read ─────────────────────────

/// What the caller wants out of the read. Bounds the BFS and controls
/// query-driven reranking.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeReadRequest {
    /// Tree id. Required so the read scope is explicit even when starting
    /// from a known node.
    pub tree_id: String,
    /// Starting node. `None` → start from the tree root.
    #[serde(default)]
    pub start_node_id: Option<String>,
    /// Maximum levels to descend from `start_node_id`. `0` returns an
    /// empty result.
    pub max_depth: u32,
    /// Optional natural-language query. When `Some`, hits are reranked by
    /// cosine similarity to the query embedding; hits with no stored
    /// embedding sort to the bottom. When `None`, BFS order is preserved.
    #[serde(default)]
    pub query: Option<String>,
    /// Max hits to return. `None` → backend default.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// One hit returned by a tree read. Compact projection — for the full
/// SummaryNode/Chunk row use the memory_store retrieval surface directly.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeReadHit {
    pub node_id: String,
    /// `"summary"` for sealed nodes, `"chunk"` for leaves.
    pub node_kind: String,
    /// Level in the tree (0 = leaf, 1+ = summary).
    pub level: u32,
    /// Summary text or chunk content, truncated by the backend if oversize.
    pub content: String,
    /// Cosine similarity score when `query` was set; `0.0` otherwise.
    #[serde(default)]
    pub score: f32,
}

/// Result of a tree read.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TreeReadResult {
    pub hits: Vec<TreeReadHit>,
    /// Total matches BEFORE `limit` truncation.
    pub total: usize,
    /// Echoes back the tree id the read targeted — useful for callers that
    /// fan out and need to attribute hits.
    pub tree_id: String,
}

impl TreeReadResult {
    pub fn empty(tree: &Tree) -> Self {
        Self {
            hits: Vec::new(),
            total: 0,
            tree_id: tree.id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory_store::trees::TreeStatus;

    fn sample_tree() -> Tree {
        Tree {
            id: "tree-1".into(),
            kind: TreeKind::Source,
            scope: "chat:slack:#eng".into(),
            root_id: Some("root-1".into()),
            max_level: 2,
            status: TreeStatus::Active,
            created_at: Utc::now(),
            last_sealed_at: None,
        }
    }

    #[test]
    fn tree_leaf_payload_converts_to_and_from_leaf_ref() {
        let payload = TreeLeafPayload {
            chunk_id: "chunk-1".into(),
            token_count: 12,
            timestamp: Utc::now(),
            content: "hello".into(),
            entities: vec!["person:alice".into()],
            topics: vec!["deploy".into()],
            score: 0.75,
        };
        let leaf: LeafRef = (&payload).into();
        let roundtrip = TreeLeafPayload::from(leaf);

        assert_eq!(roundtrip.chunk_id, payload.chunk_id);
        assert_eq!(roundtrip.token_count, payload.token_count);
        assert_eq!(roundtrip.content, payload.content);
        assert_eq!(roundtrip.entities, payload.entities);
        assert_eq!(roundtrip.topics, payload.topics);
        assert_eq!(roundtrip.score, payload.score);
    }

    #[test]
    fn tree_read_result_empty_copies_tree_id() {
        let tree = sample_tree();
        let result = TreeReadResult::empty(&tree);
        assert_eq!(result.tree_id, tree.id);
        assert_eq!(result.total, 0);
        assert!(result.hits.is_empty());
    }
}
