//! Pure hotness math for Phase 3c (#709).
//!
//! The formula intentionally folds a handful of pre-existing signals into
//! one arithmetic score. No LLM, no learned weights — the goal is
//! deterministic, greppable, testable behaviour:
//!
//! ```text
//! hotness = ln(mentions + 1)          // dampened high-volume bias
//!         + 0.5 * distinct_sources    // cross-source is valuable
//!         + recency_decay(last_seen)  // prefer active entities
//!         + graph_centrality          // Phase 4+ (None → 0.0)
//!         + 2.0 * query_hits          // retrieval feedback (Phase 4+)
//! ```
//!
//! Recency decay is a piecewise linear taper:
//! - age ≤ 1 day  → 1.0
//! - age 1…7 days → 1.0 → 0.5
//! - age 7…30 days → 0.5 → 0.0
//! - age > 30 days → 0.0
//!
//! The unit tests lock in the coarse behaviour (zero-mention, spike,
//! old-but-widely-cited) so tuning the constants later stays honest.

use chrono::Utc;

use crate::openhuman::memory::tree::topic_tree::types::EntityIndexStats;

/// Pure hotness function — no I/O, no clocks unless the caller passes one.
///
/// `entity_id` is taken for diagnostic logging only and has no effect on
/// the numeric result.
pub fn hotness(entity_id: &str, idx: &EntityIndexStats) -> f32 {
    let now_ms = Utc::now().timestamp_millis();
    hotness_at(entity_id, idx, now_ms)
}

/// Deterministic variant — computes hotness as if the current wall clock
/// were `now_ms`. Useful in tests so the recency term doesn't drift.
pub fn hotness_at(entity_id: &str, idx: &EntityIndexStats, now_ms: i64) -> f32 {
    let mention_weight = ((idx.mention_count_30d as f32) + 1.0).ln();
    let source_weight = (idx.distinct_sources as f32) * 0.5;
    let recency_weight = recency_decay(idx.last_seen_ms, now_ms);
    let centrality = idx.graph_centrality.unwrap_or(0.0);
    let query_weight = (idx.query_hits_30d as f32) * 2.0;

    let total = mention_weight + source_weight + recency_weight + centrality + query_weight;
    log::debug!(
        "[topic_tree::hotness] id={} mentions={} sources={} recency={:.3} centrality={:.3} \
         queries={} total={:.3}",
        entity_id,
        idx.mention_count_30d,
        idx.distinct_sources,
        recency_weight,
        centrality,
        idx.query_hits_30d,
        total
    );
    total
}

/// Recency decay helper. Operates on absolute epoch-millis so tests can
/// pin the clock. Returns 0.0 when `last_seen_ms` is `None`.
pub fn recency_decay(last_seen_ms: Option<i64>, now_ms: i64) -> f32 {
    let Some(last_seen) = last_seen_ms else {
        return 0.0;
    };
    let age_ms = (now_ms - last_seen).max(0);
    const DAY_MS: i64 = 24 * 60 * 60 * 1_000;
    let age_days = (age_ms as f32) / (DAY_MS as f32);

    if age_days <= 1.0 {
        1.0
    } else if age_days <= 7.0 {
        // 1.0 at day 1, 0.5 at day 7
        let frac = (age_days - 1.0) / 6.0;
        1.0 - 0.5 * frac
    } else if age_days <= 30.0 {
        // 0.5 at day 7, 0.0 at day 30
        let frac = (age_days - 7.0) / 23.0;
        0.5 - 0.5 * frac
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

    fn stats(mentions: u32, sources: u32, last_seen: Option<i64>) -> EntityIndexStats {
        EntityIndexStats {
            mention_count_30d: mentions,
            distinct_sources: sources,
            last_seen_ms: last_seen,
            query_hits_30d: 0,
            graph_centrality: None,
        }
    }

    #[test]
    fn zero_signal_entity_is_zero() {
        let now_ms = 1_700_000_000_000;
        let s = stats(0, 0, None);
        let h = hotness_at("e:none", &s, now_ms);
        // ln(0+1) + 0 + 0 + 0 + 0 = 0
        assert!(h.abs() < 1e-6);
    }

    #[test]
    fn spike_of_mentions_pushes_over_creation_threshold() {
        use crate::openhuman::memory::tree::topic_tree::types::TOPIC_CREATION_THRESHOLD;
        let now_ms = 1_700_000_000_000;
        // 100 mentions across 5 sources, 3 recent query hits, seen today.
        let s = EntityIndexStats {
            mention_count_30d: 100,
            distinct_sources: 5,
            last_seen_ms: Some(now_ms - DAY_MS / 2),
            query_hits_30d: 3,
            graph_centrality: None,
        };
        let h = hotness_at("e:hot", &s, now_ms);
        assert!(
            h > TOPIC_CREATION_THRESHOLD,
            "expected hot entity > {TOPIC_CREATION_THRESHOLD}, got {h}"
        );
    }

    #[test]
    fn old_but_widely_cited_still_has_some_heat() {
        // 50 mentions, 8 sources, last seen 20 days ago, no queries.
        let now_ms = 1_700_000_000_000;
        let s = EntityIndexStats {
            mention_count_30d: 50,
            distinct_sources: 8,
            last_seen_ms: Some(now_ms - 20 * DAY_MS),
            query_hits_30d: 0,
            graph_centrality: None,
        };
        let h = hotness_at("e:old-wide", &s, now_ms);
        // mention_weight = ln(51) ≈ 3.93, source_weight = 4.0,
        // recency at day 20 ≈ 0.5 * (30-20)/23 ≈ 0.217 → total ≈ 8.1
        assert!(h > 5.0, "widely-cited entity should retain signal: {h}");
    }

    #[test]
    fn ancient_single_mention_decays_toward_zero() {
        let now_ms = 1_700_000_000_000;
        let s = stats(1, 1, Some(now_ms - 60 * DAY_MS));
        let h = hotness_at("e:ancient", &s, now_ms);
        // ln(2) + 0.5 + 0 = ~1.19 — well below creation threshold
        assert!(h < 2.0, "ancient entity should decay: {h}");
    }

    #[test]
    fn recency_decay_today_is_one() {
        let now_ms = 1_700_000_000_000;
        let r = recency_decay(Some(now_ms), now_ms);
        assert!((r - 1.0).abs() < 1e-6);
    }

    #[test]
    fn recency_decay_week_old_is_half() {
        let now_ms = 1_700_000_000_000;
        let r = recency_decay(Some(now_ms - 7 * DAY_MS), now_ms);
        assert!((r - 0.5).abs() < 1e-3, "expected 0.5 at 7d, got {r}");
    }

    #[test]
    fn recency_decay_month_old_is_zero() {
        let now_ms = 1_700_000_000_000;
        let r = recency_decay(Some(now_ms - 30 * DAY_MS), now_ms);
        assert!(r.abs() < 1e-3, "expected ~0 at 30d, got {r}");
    }

    #[test]
    fn recency_decay_none_last_seen_is_zero() {
        assert_eq!(recency_decay(None, 1_700_000_000_000), 0.0);
    }

    #[test]
    fn query_hits_boost_hotness_aggressively() {
        let now_ms = 1_700_000_000_000;
        let base = stats(5, 1, Some(now_ms));
        let boosted = EntityIndexStats {
            query_hits_30d: 10,
            ..base.clone()
        };
        let h_base = hotness_at("e", &base, now_ms);
        let h_boosted = hotness_at("e", &boosted, now_ms);
        // 10 query hits * 2.0 = +20
        assert!(h_boosted - h_base > 19.0);
    }

    #[test]
    fn future_last_seen_is_treated_as_now() {
        // Clock drift could produce negative ages — we clamp at 0.
        let now_ms = 1_700_000_000_000;
        let r = recency_decay(Some(now_ms + DAY_MS), now_ms);
        assert!((r - 1.0).abs() < 1e-6);
    }
}
