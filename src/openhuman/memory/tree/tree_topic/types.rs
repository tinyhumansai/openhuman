//! Core types for Phase 3c — lazy topic-tree materialisation (#709).
//!
//! A *topic tree* is a per-entity summary tree whose leaves are all chunks
//! mentioning that entity, regardless of the source they came from. They
//! are materialised lazily, driven by a cheap arithmetic *hotness* score
//! over pre-existing signals. Tree mechanics (buffer / seal / cascade)
//! reuse [`source_tree`] end-to-end — topic trees only differ by the
//! `TreeKind::Topic` discriminator and the per-entity `scope`.
//!
//! This file defines:
//! - [`EntityIndexStats`] — input record for the hotness calculation
//! - [`HotnessCounters`] — the persisted row in `mem_tree_entity_hotness`
//! - threshold / cadence constants ([`TOPIC_CREATION_THRESHOLD`],
//!   [`TOPIC_ARCHIVE_THRESHOLD`], [`TOPIC_RECHECK_EVERY`])
//!
//! Persistence helpers for these types live in [`super::store`].

use serde::{Deserialize, Serialize};

/// Hotness threshold above which a topic tree is materialised for an
/// entity. Tuned (per design) to roughly "several mentions across a few
/// sources" — see [`super::hotness::hotness`] for the formula.
pub const TOPIC_CREATION_THRESHOLD: f32 = 10.0;

/// Hotness threshold below which a topic tree becomes an archive candidate.
/// Archiving is a primitive in Phase 3c — the scheduled sweep is deferred.
pub const TOPIC_ARCHIVE_THRESHOLD: f32 = 2.0;

/// How often (in ingests touching the entity) to recompute hotness from
/// the full [`EntityIndexStats`]. Between recomputes we only bump
/// `mention_count_30d` and `last_seen_ms` — cheap integer arithmetic.
pub const TOPIC_RECHECK_EVERY: u32 = 100;

/// Input record fed to [`super::hotness::hotness`].
///
/// Every field is a signal that already exists somewhere in the memory
/// tree (scoring rows, entity index, potential future graph metrics); the
/// struct is an explicit contract so the hotness math can be unit-tested
/// in isolation without touching SQLite.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EntityIndexStats {
    /// Total mentions in the last 30 days. Phase 3c currently bumps this
    /// forever — the 30d window is a TODO once we have a billable clock.
    pub mention_count_30d: u32,
    /// Number of distinct source trees this entity has appeared in. A
    /// cross-source signal — an entity spoken about in one chat channel
    /// but nowhere else is less interesting than one that appears in
    /// Slack + email + docs.
    pub distinct_sources: u32,
    /// Epoch-millis of the last ingest that referenced this entity.
    pub last_seen_ms: Option<i64>,
    /// Reserved for Phase 4 retrieval: bump whenever a user query returns
    /// this entity. Phase 3c stores the column but never increments it.
    pub query_hits_30d: u32,
    /// Reserved for a later phase: graph centrality from the entity graph.
    /// `None` means "unknown" — not "zero". See [`super::hotness::hotness`].
    pub graph_centrality: Option<f32>,
}

/// Row persisted in `mem_tree_entity_hotness`. Callers interact with this
/// through [`super::store`]; [`EntityIndexStats`] is the hotness-compute
/// view derived from it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HotnessCounters {
    pub entity_id: String,
    pub mention_count_30d: u32,
    pub distinct_sources: u32,
    pub last_seen_ms: Option<i64>,
    pub query_hits_30d: u32,
    pub graph_centrality: Option<f32>,
    /// Counts ingests **touching this entity** since the last full hotness
    /// recompute. When `>= TOPIC_RECHECK_EVERY` the curator refreshes
    /// `distinct_sources` / `last_hotness` and resets this to 0.
    pub ingests_since_check: u32,
    pub last_hotness: Option<f32>,
    pub last_updated_ms: i64,
}

impl HotnessCounters {
    /// Fresh row for an entity seen for the first time.
    pub fn fresh(entity_id: &str, now_ms: i64) -> Self {
        Self {
            entity_id: entity_id.to_string(),
            mention_count_30d: 0,
            distinct_sources: 0,
            last_seen_ms: None,
            query_hits_30d: 0,
            graph_centrality: None,
            ingests_since_check: 0,
            last_hotness: None,
            last_updated_ms: now_ms,
        }
    }

    /// Project the persisted row into an [`EntityIndexStats`] ready for
    /// [`super::hotness::hotness`].
    pub fn stats(&self) -> EntityIndexStats {
        EntityIndexStats {
            mention_count_30d: self.mention_count_30d,
            distinct_sources: self.distinct_sources,
            last_seen_ms: self.last_seen_ms,
            query_hits_30d: self.query_hits_30d,
            graph_centrality: self.graph_centrality,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_counters_are_zero() {
        let c = HotnessCounters::fresh("email:alice@example.com", 1_700_000_000_000);
        assert_eq!(c.entity_id, "email:alice@example.com");
        assert_eq!(c.mention_count_30d, 0);
        assert_eq!(c.distinct_sources, 0);
        assert_eq!(c.ingests_since_check, 0);
        assert!(c.last_hotness.is_none());
        assert!(c.last_seen_ms.is_none());
        assert_eq!(c.last_updated_ms, 1_700_000_000_000);
    }

    #[test]
    fn stats_projection_mirrors_row() {
        let c = HotnessCounters {
            entity_id: "e".into(),
            mention_count_30d: 5,
            distinct_sources: 2,
            last_seen_ms: Some(42),
            query_hits_30d: 1,
            graph_centrality: Some(0.3),
            ingests_since_check: 4,
            last_hotness: Some(9.9),
            last_updated_ms: 100,
        };
        let s = c.stats();
        assert_eq!(s.mention_count_30d, 5);
        assert_eq!(s.distinct_sources, 2);
        assert_eq!(s.last_seen_ms, Some(42));
        assert_eq!(s.query_hits_30d, 1);
        assert_eq!(s.graph_centrality, Some(0.3));
    }

    #[test]
    fn thresholds_make_creation_strictly_above_archive() {
        assert!(TOPIC_CREATION_THRESHOLD > TOPIC_ARCHIVE_THRESHOLD);
        assert!(TOPIC_RECHECK_EVERY > 0);
    }
}
