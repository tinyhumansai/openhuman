//! Shared types for Phase 4 retrieval tools (#710).
//!
//! These types are the wire / JSON-RPC shape returned by the six retrieval
//! primitives. They wrap the internal persistence structs (`SummaryNode`,
//! `Chunk`, `EntityHit`) into a single unified [`RetrievalHit`] shape so the
//! calling LLM sees the same schema regardless of which tool it invoked.
//!
//! Rules of the road:
//! - All types are [`serde::Serialize`] + [`serde::Deserialize`] so they
//!   round-trip through JSON-RPC without bespoke conversion.
//! - Time fields use `DateTime<Utc>` serialised as RFC3339 — matches the
//!   global recap convention so callers comparing hits across tools don't
//!   juggle epochs.
//! - `node_kind` discriminates leaf (raw chunk) vs. summary — retrieval
//!   consumers frequently branch on this (e.g. "drill down only on
//!   summaries").

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::openhuman::memory::tree::score::extract::EntityKind;
use crate::openhuman::memory::tree::source_tree::types::{SummaryNode, Tree, TreeKind};
use crate::openhuman::memory::tree::types::{Chunk, SourceKind};

/// Whether a hit represents a leaf (raw chunk) or a summary node.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// Leaf = one `mem_tree_chunks` row (level 0).
    Leaf,
    /// Summary = one `mem_tree_summaries` row (level ≥ 1 for source/topic,
    /// level ≥ 0 for global where L0 is a daily digest).
    Summary,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Leaf => "leaf",
            Self::Summary => "summary",
        }
    }
}

/// One unit of retrieval output. Shape is identical whether the hit was
/// sourced from a source tree summary, a topic tree summary, the global
/// digest, or a raw leaf chunk.
///
/// `tree_id` / `tree_kind` / `tree_scope` identify which tree the hit
/// belongs to so UIs can surface provenance ("from Slack #eng"). For bare
/// leaves not yet attached to any tree, `tree_id` is empty and `tree_kind`
/// is still meaningful (mirrors the leaf's source kind classification —
/// see [`leaf_tree_placeholder`] for how we synthesise it).
///
/// `child_ids` is empty on leaves; on summaries it points at the next level
/// down (chunks for L1, summaries for L2+).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetrievalHit {
    pub node_id: String,
    pub node_kind: NodeKind,
    pub tree_id: String,
    pub tree_kind: TreeKind,
    pub tree_scope: String,
    pub level: u32,
    pub content: String,
    pub entities: Vec<String>,
    pub topics: Vec<String>,
    pub time_range_start: DateTime<Utc>,
    pub time_range_end: DateTime<Utc>,
    pub score: f32,
    pub child_ids: Vec<String>,
    /// Populated for leaves (chunk back-pointer); `None` for summaries.
    pub source_ref: Option<String>,
}

/// Envelope for the four "query" tools (`query_source`, `query_global`,
/// `query_topic`, `drill_down` by wrapper).
///
/// `total` is the pre-truncation match count so callers can tell whether a
/// high-limit follow-up would return more. `truncated` is `total > hits.len()`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryResponse {
    pub hits: Vec<RetrievalHit>,
    pub total: usize,
    pub truncated: bool,
}

impl QueryResponse {
    /// Build a response from a post-filtered, post-sorted hit list. The
    /// `total_matches` is the count BEFORE applying `limit` so callers can
    /// see whether truncation happened.
    pub fn new(hits: Vec<RetrievalHit>, total_matches: usize) -> Self {
        let truncated = total_matches > hits.len();
        Self {
            hits,
            total: total_matches,
            truncated,
        }
    }

    /// Empty response (no matches). `total=0`, `truncated=false`.
    pub fn empty() -> Self {
        Self {
            hits: Vec::new(),
            total: 0,
            truncated: false,
        }
    }
}

/// Convert a sealed [`SummaryNode`] into a [`RetrievalHit`]. `tree_scope`
/// is threaded in from the caller so we don't force a tree lookup on every
/// conversion — the caller already has the parent [`Tree`] in hand.
pub fn hit_from_summary(node: &SummaryNode, tree_scope: &str) -> RetrievalHit {
    RetrievalHit {
        node_id: node.id.clone(),
        node_kind: NodeKind::Summary,
        tree_id: node.tree_id.clone(),
        tree_kind: node.tree_kind,
        tree_scope: tree_scope.to_string(),
        level: node.level,
        content: node.content.clone(),
        entities: node.entities.clone(),
        topics: node.topics.clone(),
        time_range_start: node.time_range_start,
        time_range_end: node.time_range_end,
        score: node.score,
        child_ids: node.child_ids.clone(),
        source_ref: None,
    }
}

/// Convert a sealed [`SummaryNode`] using a full [`Tree`] for the scope. A
/// thin convenience over [`hit_from_summary`].
pub fn hit_from_summary_with_tree(node: &SummaryNode, tree: &Tree) -> RetrievalHit {
    hit_from_summary(node, &tree.scope)
}

/// Convert a raw [`Chunk`] (leaf) into a [`RetrievalHit`]. Because a chunk
/// may not yet be attached to a summary tree, callers can pass `tree_id` /
/// `tree_scope` as empty strings. `tree_kind` is always [`TreeKind::Source`]
/// for leaves — raw chunks belong conceptually to their originating source
/// tree even when the tree hasn't materialised yet (no seals).
pub fn hit_from_chunk(chunk: &Chunk, tree_id: &str, tree_scope: &str, score: f32) -> RetrievalHit {
    let source_ref = chunk.metadata.source_ref.as_ref().map(|r| r.value.clone());
    RetrievalHit {
        node_id: chunk.id.clone(),
        node_kind: NodeKind::Leaf,
        tree_id: tree_id.to_string(),
        tree_kind: leaf_tree_placeholder(chunk.metadata.source_kind),
        tree_scope: tree_scope.to_string(),
        level: 0,
        content: chunk.content.clone(),
        entities: Vec::new(),
        topics: chunk.metadata.tags.clone(),
        time_range_start: chunk.metadata.time_range.0,
        time_range_end: chunk.metadata.time_range.1,
        score,
        child_ids: Vec::new(),
        source_ref,
    }
}

/// Decide the placeholder [`TreeKind`] to report on a leaf hit. Leaves live
/// under source trees regardless of the underlying `SourceKind`, so we
/// always return [`TreeKind::Source`]. Accepting the `SourceKind` argument
/// keeps the call site explicit about why the classification is stable.
pub fn leaf_tree_placeholder(_source_kind: SourceKind) -> TreeKind {
    TreeKind::Source
}

/// Output shape for `search_entities`. One row per canonical id with the
/// aggregate stats across the entity index.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntityMatch {
    /// Canonical id (e.g. `email:alice@example.com`, `topic:phoenix`).
    pub canonical_id: String,
    pub kind: EntityKind,
    /// Example surface form that matched — useful for UI display.
    pub surface: String,
    /// Total rows in `mem_tree_entity_index` grouped under this canonical id.
    pub mention_count: u64,
    /// Epoch-millis of the newest mention across all rows.
    pub last_seen_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_kind_as_str_round_trips() {
        assert_eq!(NodeKind::Leaf.as_str(), "leaf");
        assert_eq!(NodeKind::Summary.as_str(), "summary");
    }

    #[test]
    fn query_response_truncated_when_total_exceeds_hits() {
        let hit = sample_hit();
        let resp = QueryResponse::new(vec![hit.clone()], 5);
        assert_eq!(resp.hits.len(), 1);
        assert_eq!(resp.total, 5);
        assert!(resp.truncated);
    }

    #[test]
    fn query_response_not_truncated_when_all_returned() {
        let hit = sample_hit();
        let resp = QueryResponse::new(vec![hit.clone()], 1);
        assert!(!resp.truncated);
    }

    #[test]
    fn query_response_empty_is_inert() {
        let resp = QueryResponse::empty();
        assert!(resp.hits.is_empty());
        assert_eq!(resp.total, 0);
        assert!(!resp.truncated);
    }

    #[test]
    fn retrieval_hit_serde_round_trip() {
        let hit = sample_hit();
        let json = serde_json::to_string(&hit).unwrap();
        let back: RetrievalHit = serde_json::from_str(&json).unwrap();
        assert_eq!(back, hit);
    }

    #[test]
    fn entity_match_serde_round_trip() {
        let m = EntityMatch {
            canonical_id: "email:alice@example.com".into(),
            kind: EntityKind::Email,
            surface: "alice@example.com".into(),
            mention_count: 7,
            last_seen_ms: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: EntityMatch = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }

    fn sample_hit() -> RetrievalHit {
        RetrievalHit {
            node_id: "sum-1".into(),
            node_kind: NodeKind::Summary,
            tree_id: "tree-1".into(),
            tree_kind: TreeKind::Source,
            tree_scope: "slack:#eng".into(),
            level: 1,
            content: "the sealed summary content".into(),
            entities: vec!["email:alice@example.com".into()],
            topics: vec!["#launch".into()],
            time_range_start: Utc::now(),
            time_range_end: Utc::now(),
            score: 0.75,
            child_ids: vec!["leaf-a".into(), "leaf-b".into()],
            source_ref: None,
        }
    }
}
