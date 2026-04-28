//! Window-scoped recap retrieval for the global activity tree (#709 Phase 3b).
//!
//! Given a duration (e.g. `Duration::days(7)`), pick the tree level that
//! naturally matches the time axis and return the latest summary at that
//! level. This is the read half of the global digest: the digest builder
//! plants daily/weekly/monthly/yearly nodes, and `recap` retrieves the one
//! best suited for the caller's question.
//!
//! Level selection (width thresholds chosen to cover expected call sites):
//!   - `< 2 days`  → latest L0 (today's digest)
//!   - `< 14 days` → latest L1 (weekly)
//!   - `< 60 days` → latest L2 (monthly)
//!   - `≥ 60 days` → latest L3 (yearly), padded with the covering L2s when no L3 has sealed yet.
//!
//! When no summary exists at the chosen level, the function falls back
//! downward (to the latest lower-level node) and reports the actual level
//! used in the `level_used` field of the result so callers can surface
//! "best available" to users. Returns `None` only when the global tree has
//! no sealed summaries at all.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::global_tree::registry::get_or_create_global_tree;
use crate::openhuman::memory::tree::source_tree::store;
use crate::openhuman::memory::tree::source_tree::types::SummaryNode;

/// Aggregated recap returned to the caller.
#[derive(Debug, Clone)]
pub struct RecapOutput {
    /// The rolled-up content for the chosen window.
    pub content: String,
    /// The time span actually covered by the returned content. Start is the
    /// earliest `time_range_start` across included summaries, end is the
    /// latest `time_range_end`.
    pub time_range: (DateTime<Utc>, DateTime<Utc>),
    /// The level actually used to build the recap. May be lower than the
    /// requested level when the higher level has no sealed nodes yet.
    pub level_used: u32,
    /// One entry per summary folded into the content, in the order they
    /// were concatenated. Lets callers surface provenance ("this recap
    /// covers weekly summaries W, W-1, W-2").
    pub summary_ids: Vec<String>,
}

/// Return a recap for the given window, or `None` if no global summaries
/// have sealed yet.
pub async fn recap(config: &Config, window: Duration) -> Result<Option<RecapOutput>> {
    let target_level = pick_level(window);
    log::info!(
        "[global_tree::recap] recap window={:?} target_level={}",
        window,
        target_level
    );

    let global = get_or_create_global_tree(config)?;
    let now = Utc::now();
    let window_start = now - window;

    // Walk down from `target_level` to 0 looking for material.
    for level in (0..=target_level).rev() {
        let all_at_level = store::list_summaries_at_level(config, &global.id, level)?;
        if all_at_level.is_empty() {
            continue;
        }
        let covering = pick_covering(&all_at_level, window_start, now);
        if covering.is_empty() {
            continue;
        }
        log::debug!(
            "[global_tree::recap] using level={} summaries={}",
            level,
            covering.len()
        );
        return Ok(Some(assemble_recap(&covering, level)));
    }

    log::info!("[global_tree::recap] no global summaries yet — nothing to recap");
    Ok(None)
}

/// Map a window duration to the level whose node-granularity best matches
/// the window. See module-level doc for the thresholds.
pub fn pick_level(window: Duration) -> u32 {
    // Direct comparisons keep the selection readable versus a table walk
    // since there are only four bands. See module-level doc for the exact
    // ceilings.
    if window < Duration::days(2) {
        0
    } else if window < Duration::days(14) {
        1
    } else if window < Duration::days(60) {
        2
    } else {
        3
    }
}

/// Select every summary at the given level whose time range overlaps the
/// [window_start, now] window, ordered oldest → newest. When none overlap
/// (a long quiet stretch ending before the window) we fall back to the
/// single latest summary so callers still get *something* useful.
fn pick_covering(
    summaries: &[SummaryNode],
    window_start: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Vec<&SummaryNode> {
    let mut overlapping: Vec<&SummaryNode> = summaries
        .iter()
        .filter(|s| s.time_range_end >= window_start && s.time_range_start <= now)
        .collect();
    overlapping.sort_by_key(|s| s.time_range_start);

    if overlapping.is_empty() {
        if let Some(latest) = summaries.iter().max_by_key(|s| s.sealed_at) {
            return vec![latest];
        }
    }
    overlapping
}

/// Concatenate the selected summaries with provenance markers and compute
/// the time envelope.
fn assemble_recap(covering: &[&SummaryNode], level: u32) -> RecapOutput {
    let mut parts: Vec<String> = Vec::with_capacity(covering.len());
    let mut summary_ids: Vec<String> = Vec::with_capacity(covering.len());
    for s in covering {
        parts.push(format!(
            "[{} → {}]\n{}",
            s.time_range_start.to_rfc3339(),
            s.time_range_end.to_rfc3339(),
            s.content
        ));
        summary_ids.push(s.id.clone());
    }
    let content = parts.join("\n\n");

    let time_start = covering
        .iter()
        .map(|s| s.time_range_start)
        .min()
        .unwrap_or_else(Utc::now);
    let time_end = covering
        .iter()
        .map(|s| s.time_range_end)
        .max()
        .unwrap_or_else(Utc::now);

    RecapOutput {
        content,
        time_range: (time_start, time_end),
        level_used: level,
        summary_ids,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::global_tree::digest::{end_of_day_digest, DigestOutcome};
    use crate::openhuman::memory::tree::source_tree::bucket_seal::{
        append_leaf, LabelStrategy, LeafRef,
    };
    use crate::openhuman::memory::tree::source_tree::registry::get_or_create_source_tree;
    use crate::openhuman::memory::tree::source_tree::summariser::inert::InertSummariser;
    use crate::openhuman::memory::tree::store::upsert_chunks;
    use crate::openhuman::memory::tree::types::{chunk_id, Chunk, Metadata, SourceKind, SourceRef};
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        // Phase 4 (#710): recap exercises the digest path which embeds.
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    #[test]
    fn pick_level_matches_window_thresholds() {
        assert_eq!(pick_level(Duration::hours(1)), 0);
        assert_eq!(pick_level(Duration::days(1)), 0);
        assert_eq!(pick_level(Duration::days(2)), 1);
        assert_eq!(pick_level(Duration::days(7)), 1);
        assert_eq!(pick_level(Duration::days(13)), 1);
        assert_eq!(pick_level(Duration::days(14)), 2);
        assert_eq!(pick_level(Duration::days(30)), 2);
        assert_eq!(pick_level(Duration::days(59)), 2);
        assert_eq!(pick_level(Duration::days(60)), 3);
        assert_eq!(pick_level(Duration::days(365)), 3);
    }

    #[tokio::test]
    async fn recap_on_empty_tree_returns_none() {
        let (_tmp, cfg) = test_config();
        let out = recap(&cfg, Duration::days(7)).await.unwrap();
        assert!(out.is_none());
    }

    async fn seed_source_l1(cfg: &Config, scope: &str, ts: DateTime<Utc>) {
        let tree = get_or_create_source_tree(cfg, scope).unwrap();
        let summariser = InertSummariser::new();
        let c1 = Chunk {
            id: chunk_id(SourceKind::Chat, scope, 0, "test-content"),
            content: format!("c1-{scope}"),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: scope.into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: Some(SourceRef::new("slack://x")),
            },
            token_count: 6_000,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        };
        let c2 = Chunk {
            id: chunk_id(SourceKind::Chat, scope, 1, "test-content"),
            content: format!("c2-{scope}"),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: scope.into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: Some(SourceRef::new("slack://y")),
            },
            token_count: 6_000,
            seq_in_source: 1,
            created_at: ts,
            partial_message: false,
        };
        upsert_chunks(cfg, &[c1.clone(), c2.clone()]).unwrap();
        append_leaf(
            cfg,
            &tree,
            &LeafRef {
                chunk_id: c1.id.clone(),
                token_count: 6_000,
                timestamp: ts,
                content: c1.content.clone(),
                entities: vec![],
                topics: vec![],
                score: 0.5,
            },
            &summariser,
            &LabelStrategy::Empty,
        )
        .await
        .unwrap();
        append_leaf(
            cfg,
            &tree,
            &LeafRef {
                chunk_id: c2.id.clone(),
                token_count: 6_000,
                timestamp: ts,
                content: c2.content.clone(),
                entities: vec![],
                topics: vec![],
                score: 0.5,
            },
            &summariser,
            &LabelStrategy::Empty,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn recap_one_day_window_returns_latest_l0() {
        // One daily digest → recap(1 day) should return the L0 at the
        // correct level.
        let (_tmp, cfg) = test_config();
        let summariser = InertSummariser::new();
        // Use "today" so the digest's time range covers now.
        let day = Utc::now().date_naive();
        let ts = day.and_hms_opt(12, 0, 0).unwrap().and_utc();
        seed_source_l1(&cfg, "slack:#eng", ts).await;
        let outcome = end_of_day_digest(&cfg, day, &summariser).await.unwrap();
        assert!(matches!(outcome, DigestOutcome::Emitted { .. }));

        let r = recap(&cfg, Duration::hours(24))
            .await
            .unwrap()
            .expect("expected a recap with one daily node emitted");
        assert_eq!(r.level_used, 0);
        assert_eq!(r.summary_ids.len(), 1);
        assert!(!r.content.is_empty());
    }

    #[tokio::test]
    async fn recap_weekly_window_falls_back_to_l0_when_no_l1() {
        // With only 3 daily nodes (< 7) no L1 has sealed. A 7-day recap
        // should fall back from level 1 to level 0 and return whatever
        // daily nodes exist.
        let (_tmp, cfg) = test_config();
        let summariser = InertSummariser::new();
        let today = Utc::now().date_naive();
        let base = today - Duration::days(2);
        for i in 0..3 {
            let day = base + Duration::days(i);
            let ts = day.and_hms_opt(10, 0, 0).unwrap().and_utc();
            seed_source_l1(&cfg, &format!("slack:#d{i}"), ts).await;
            end_of_day_digest(&cfg, day, &summariser).await.unwrap();
        }
        let r = recap(&cfg, Duration::days(7))
            .await
            .unwrap()
            .expect("expected fallback recap");
        assert_eq!(
            r.level_used, 0,
            "no L1 available yet → recap falls back to L0"
        );
        assert_eq!(r.summary_ids.len(), 3, "all three daily nodes folded in");
    }

    #[tokio::test]
    async fn recap_weekly_window_uses_l1_when_sealed() {
        // After 7 daily digests a weekly L1 exists. A 7-day recap should
        // return that L1 at level 1.
        let (_tmp, cfg) = test_config();
        let summariser = InertSummariser::new();
        let today = Utc::now().date_naive();
        let base = today - Duration::days(6);
        for i in 0..7 {
            let day = base + Duration::days(i);
            let ts = day.and_hms_opt(10, 0, 0).unwrap().and_utc();
            seed_source_l1(&cfg, &format!("slack:#w{i}"), ts).await;
            end_of_day_digest(&cfg, day, &summariser).await.unwrap();
        }
        let r = recap(&cfg, Duration::days(7))
            .await
            .unwrap()
            .expect("expected recap with weekly seal");
        assert_eq!(r.level_used, 1);
        assert_eq!(
            r.summary_ids.len(),
            1,
            "one weekly L1 node covers the window"
        );
    }
}
