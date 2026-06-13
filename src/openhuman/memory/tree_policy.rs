//! Tree policy layer.
//!
//! `tree` itself stays generic: summaries, buffers, sealing, and storage.
//! Flavor-specific tuning (global cadence, topic hotness thresholds, source
//! label policy) is centralized here so per-flavor modules don't each own
//! their own scattered constants and arithmetic.

use crate::openhuman::memory_store::trees::types::{
    EntityIndexStats, TOPIC_ARCHIVE_THRESHOLD, TOPIC_CREATION_THRESHOLD, TOPIC_RECHECK_EVERY,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreePolicy {
    Source,
    Topic,
    Global,
}

impl TreePolicy {
    pub fn global() -> Self {
        Self::Global
    }

    pub fn topic() -> Self {
        Self::Topic
    }

    pub fn source() -> Self {
        Self::Source
    }

    pub fn topic_creation_threshold(self) -> f32 {
        let _ = self;
        TOPIC_CREATION_THRESHOLD
    }

    pub fn topic_archive_threshold(self) -> f32 {
        let _ = self;
        TOPIC_ARCHIVE_THRESHOLD
    }

    pub fn topic_recheck_every(self) -> u32 {
        let _ = self;
        TOPIC_RECHECK_EVERY
    }

    pub fn topic_hotness(self, entity_id: &str, idx: &EntityIndexStats, now_ms: i64) -> f32 {
        let _ = self;
        let mention_weight = ((idx.mention_count_30d as f32) + 1.0).ln();
        let source_weight = (idx.distinct_sources as f32) * 0.5;
        let recency_weight = self.topic_recency_decay(idx.last_seen_ms, now_ms);
        let centrality = idx.graph_centrality.unwrap_or(0.0);
        let query_weight = (idx.query_hits_30d as f32) * 2.0;

        let total = mention_weight + source_weight + recency_weight + centrality + query_weight;
        log::debug!(
            "[tree_topic::hotness] id={} mentions={} sources={} recency={:.3} centrality={:.3} \
             queries={} total={:.3}",
            crate::openhuman::memory::util::redact::redact(entity_id),
            idx.mention_count_30d,
            idx.distinct_sources,
            recency_weight,
            centrality,
            idx.query_hits_30d,
            total
        );
        total
    }

    pub fn topic_recency_decay(self, last_seen_ms: Option<i64>, now_ms: i64) -> f32 {
        let _ = self;
        let Some(last_seen) = last_seen_ms else {
            return 0.0;
        };
        let age_ms = (now_ms - last_seen).max(0);
        const DAY_MS: i64 = 24 * 60 * 60 * 1_000;
        let age_days = (age_ms as f32) / (DAY_MS as f32);

        if age_days <= 1.0 {
            1.0
        } else if age_days <= 7.0 {
            let frac = (age_days - 1.0) / 6.0;
            1.0 - 0.5 * frac
        } else if age_days <= 30.0 {
            let frac = (age_days - 7.0) / 23.0;
            0.5 - 0.5 * frac
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory_store::trees::types::EntityIndexStats;

    const DAY_MS: i64 = 86_400_000;
    const NOW_MS: i64 = 1_700_000_000_000;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn zero_stats() -> EntityIndexStats {
        EntityIndexStats {
            mention_count_30d: 0,
            distinct_sources: 0,
            last_seen_ms: None,
            query_hits_30d: 0,
            graph_centrality: None,
        }
    }

    // ── 1. Constructors ───────────────────────────────────────────────────────

    #[test]
    fn constructors_return_expected_variants() {
        assert_eq!(TreePolicy::global(), TreePolicy::Global);
        assert_eq!(TreePolicy::topic(), TreePolicy::Topic);
        assert_eq!(TreePolicy::source(), TreePolicy::Source);
    }

    // ── 2. Threshold constants ────────────────────────────────────────────────

    #[test]
    fn threshold_constants_are_positive() {
        let p = TreePolicy::Topic;
        assert!(
            p.topic_creation_threshold() > 0.0,
            "creation threshold must be positive"
        );
        assert!(
            p.topic_archive_threshold() > 0.0,
            "archive threshold must be positive"
        );
        assert!(
            p.topic_recheck_every() > 0,
            "recheck cadence must be positive"
        );
    }

    #[test]
    fn creation_threshold_exceeds_archive_threshold() {
        let p = TreePolicy::Topic;
        assert!(
            p.topic_creation_threshold() > p.topic_archive_threshold(),
            "creation threshold ({}) must exceed archive threshold ({})",
            p.topic_creation_threshold(),
            p.topic_archive_threshold()
        );
    }

    // ── 3. Recency decay boundary values ──────────────────────────────────────

    #[test]
    fn recency_decay_none_last_seen_is_zero() {
        let decay = TreePolicy::Topic.topic_recency_decay(None, NOW_MS);
        assert_eq!(decay, 0.0);
    }

    #[test]
    fn recency_decay_age_zero_is_one() {
        // Seen exactly at now — age = 0.
        let decay = TreePolicy::Topic.topic_recency_decay(Some(NOW_MS), NOW_MS);
        assert_eq!(decay, 1.0);
    }

    #[test]
    fn recency_decay_age_one_day_is_one() {
        let last_seen = NOW_MS - DAY_MS;
        let decay = TreePolicy::Topic.topic_recency_decay(Some(last_seen), NOW_MS);
        assert_eq!(decay, 1.0);
    }

    #[test]
    fn recency_decay_age_seven_days_is_half() {
        let last_seen = NOW_MS - 7 * DAY_MS;
        let decay = TreePolicy::Topic.topic_recency_decay(Some(last_seen), NOW_MS);
        assert!(
            (decay - 0.5).abs() < 1e-4,
            "expected ~0.5 at 7 days, got {decay}"
        );
    }

    #[test]
    fn recency_decay_age_thirty_days_is_zero() {
        let last_seen = NOW_MS - 30 * DAY_MS;
        let decay = TreePolicy::Topic.topic_recency_decay(Some(last_seen), NOW_MS);
        assert!(decay.abs() < 1e-4, "expected ~0.0 at 30 days, got {decay}");
    }

    #[test]
    fn recency_decay_age_sixty_days_is_zero() {
        let last_seen = NOW_MS - 60 * DAY_MS;
        let decay = TreePolicy::Topic.topic_recency_decay(Some(last_seen), NOW_MS);
        assert_eq!(decay, 0.0, "expected exactly 0.0 beyond 30 days");
    }

    // ── 4. Recency decay mid-range interpolation ──────────────────────────────

    #[test]
    fn recency_decay_four_days_is_between_half_and_one() {
        // 4 days falls in the 1–7 day band (1.0 → 0.5).
        let last_seen = NOW_MS - 4 * DAY_MS;
        let decay = TreePolicy::Topic.topic_recency_decay(Some(last_seen), NOW_MS);
        assert!(
            decay > 0.5 && decay < 1.0,
            "expected decay in (0.5, 1.0) at 4 days, got {decay}"
        );
    }

    // ── 5. Hotness: zero-signal entity ────────────────────────────────────────

    #[test]
    fn hotness_zero_signal_entity_is_zero() {
        // mention_count=0 → ln(1)=0; sources=0; last_seen=None → recency=0;
        // centrality=None → 0; query_hits=0 → 0. Total must be 0.
        let stats = zero_stats();
        let h = TreePolicy::Topic.topic_hotness("entity:zero", &stats, NOW_MS);
        assert_eq!(h, 0.0, "zero-signal entity should have hotness 0.0");
    }

    // ── 6. Hotness: high-signal entity exceeds creation threshold ─────────────

    #[test]
    fn hotness_high_signal_exceeds_creation_threshold() {
        let stats = EntityIndexStats {
            mention_count_30d: 50,
            distinct_sources: 5,
            last_seen_ms: Some(NOW_MS - DAY_MS / 2), // half a day ago → recency = 1.0
            query_hits_30d: 10,
            graph_centrality: Some(1.0),
        };
        let h = TreePolicy::Topic.topic_hotness("entity:hot", &stats, NOW_MS);
        let threshold = TreePolicy::Topic.topic_creation_threshold();
        assert!(
            h > threshold,
            "high-signal hotness ({h:.3}) should exceed creation threshold ({threshold})"
        );
    }

    // ── 7. Query-hits boost is significant ────────────────────────────────────

    #[test]
    fn hotness_query_hits_boost_is_double() {
        // Two otherwise identical entities; one has query_hits=5, the other 0.
        // The difference must equal 2.0 * 5 = 10.0.
        let base = EntityIndexStats {
            mention_count_30d: 3,
            distinct_sources: 1,
            last_seen_ms: None,
            query_hits_30d: 0,
            graph_centrality: None,
        };
        let with_queries = EntityIndexStats {
            query_hits_30d: 5,
            ..base.clone()
        };

        let h_base = TreePolicy::Topic.topic_hotness("entity:base", &base, NOW_MS);
        let h_queries = TreePolicy::Topic.topic_hotness("entity:queries", &with_queries, NOW_MS);

        let expected_boost = 2.0 * 5.0_f32;
        assert!(
            (h_queries - h_base - expected_boost).abs() < 1e-4,
            "query boost should be {expected_boost}, got {:.3}",
            h_queries - h_base
        );
    }

    // ── 8. Graph centrality contributes ──────────────────────────────────────

    #[test]
    fn hotness_graph_centrality_contributes() {
        let base = EntityIndexStats {
            mention_count_30d: 2,
            distinct_sources: 1,
            last_seen_ms: None,
            query_hits_30d: 0,
            graph_centrality: None,
        };
        let with_centrality = EntityIndexStats {
            graph_centrality: Some(3.5),
            ..base.clone()
        };

        let h_base = TreePolicy::Topic.topic_hotness("entity:central_base", &base, NOW_MS);
        let h_central = TreePolicy::Topic.topic_hotness("entity:central", &with_centrality, NOW_MS);

        assert!(
            (h_central - h_base - 3.5).abs() < 1e-4,
            "centrality contribution should be 3.5, got {:.3}",
            h_central - h_base
        );
    }

    // ── 9. Ancient single mention decays toward zero ──────────────────────────

    #[test]
    fn hotness_ancient_single_mention_is_near_zero() {
        // 1 mention, 1 source, last seen 365 days ago → recency = 0.
        // hotness = ln(2) + 0.5 * 1 + 0 + 0 + 0 ≈ 0.693 + 0.5 = 1.193
        // That should be well below the creation threshold (10.0).
        let stats = EntityIndexStats {
            mention_count_30d: 1,
            distinct_sources: 1,
            last_seen_ms: Some(NOW_MS - 365 * DAY_MS),
            query_hits_30d: 0,
            graph_centrality: None,
        };
        let h = TreePolicy::Topic.topic_hotness("entity:ancient", &stats, NOW_MS);
        let threshold = TreePolicy::Topic.topic_creation_threshold();
        assert!(
            h < threshold,
            "ancient single-mention hotness ({h:.3}) should be below creation threshold ({threshold})"
        );
        // Recency component must be zero (age >> 30 days).
        let recency = TreePolicy::Topic.topic_recency_decay(Some(NOW_MS - 365 * DAY_MS), NOW_MS);
        assert_eq!(recency, 0.0, "recency for 365-day-old entity must be 0.0");
    }
}
