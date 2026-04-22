//! Count-based cascade-seal for the global activity digest tree (#709 Phase 3b).
//!
//! The global tree's trigger is **time/count-based**, not token-based: seal
//! L0 → L1 when 7 daily nodes accumulate, L1 → L2 when 4 weekly nodes
//! accumulate, L2 → L3 when 12 monthly nodes accumulate. This keeps the
//! tree aligned to the time axis (day / week / month / year) so
//! window-scoped recap queries can map a duration to a level deterministically.
//!
//! Reuses Phase 3a storage primitives from `source_tree::store` without
//! their token-budget cascade logic — all global seals route through
//! `mem_tree_summaries` on both sides (children and output), since even L0
//! is a sealed summary node rather than a raw chunk.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::global_tree::{
    GLOBAL_TOKEN_BUDGET, MONTHLY_SEAL_THRESHOLD, WEEKLY_SEAL_THRESHOLD, YEARLY_SEAL_THRESHOLD,
};
use crate::openhuman::memory::tree::source_tree::registry::new_summary_id;
use crate::openhuman::memory::tree::source_tree::store;
use crate::openhuman::memory::tree::source_tree::summariser::{
    Summariser, SummaryContext, SummaryInput,
};
use crate::openhuman::memory::tree::source_tree::types::{Buffer, SummaryNode, Tree, TreeKind};
use crate::openhuman::memory::tree::store::with_connection;

/// Hard cap on cascade depth — mirrors the source-tree constant. L0→L1→L2→L3
/// is only 3 hops so we have ample slack.
const MAX_CASCADE_DEPTH: u32 = 32;

/// Idempotently append one level-0 (daily) summary id to the global tree's
/// L0 buffer, then cascade-seal upward if count thresholds are crossed.
///
/// The caller (`digest::end_of_day_digest`) has already inserted the L0
/// node into `mem_tree_summaries`; this function only handles the buffer
/// accounting + cascade.
pub async fn append_daily_and_cascade(
    config: &Config,
    tree: &Tree,
    daily_summary: &SummaryNode,
    summariser: &dyn Summariser,
) -> Result<Vec<String>> {
    log::debug!(
        "[global_tree::seal] append_daily tree_id={} daily_id={} tokens={}",
        tree.id,
        daily_summary.id,
        daily_summary.token_count
    );

    append_to_buffer(
        config,
        &tree.id,
        0,
        &daily_summary.id,
        daily_summary.token_count as i64,
        daily_summary.time_range_start,
    )?;

    cascade_seals(config, tree, summariser).await
}

/// Transactionally append a single summary id to the buffer at
/// `(tree_id, level)`. Idempotent on the `(tree_id, level, item_id)` tuple
/// so retries of a partially-applied digest don't double-count.
fn append_to_buffer(
    config: &Config,
    tree_id: &str,
    level: u32,
    item_id: &str,
    token_delta: i64,
    item_ts: DateTime<Utc>,
) -> Result<()> {
    with_connection(config, |conn| {
        let tx = conn.unchecked_transaction()?;
        let mut buf = store::get_buffer_conn(&tx, tree_id, level)?;
        if buf.item_ids.iter().any(|existing| existing == item_id) {
            log::debug!(
                "[global_tree::seal] append_to_buffer: {item_id} already in buffer \
                 tree_id={tree_id} level={level} — no-op"
            );
            return Ok(());
        }
        buf.item_ids.push(item_id.to_string());
        buf.token_sum = buf.token_sum.saturating_add(token_delta);
        buf.oldest_at = match buf.oldest_at {
            Some(existing) => Some(existing.min(item_ts)),
            None => Some(item_ts),
        };
        store::upsert_buffer_tx(&tx, &buf)?;
        tx.commit()?;
        Ok(())
    })
}

async fn cascade_seals(
    config: &Config,
    tree: &Tree,
    summariser: &dyn Summariser,
) -> Result<Vec<String>> {
    let mut sealed_ids: Vec<String> = Vec::new();
    // `level` is independent of the iteration counter — it only bumps when a
    // seal fires, and the loop can break early if `should_seal` returns
    // false. Clippy's loop-counter suggestion would merge them incorrectly.
    #[allow(clippy::explicit_counter_loop)]
    {
        let mut level: u32 = 0;
        for _ in 0..MAX_CASCADE_DEPTH {
            let buf = store::get_buffer(config, &tree.id, level)?;
            if !should_seal(&buf, level) {
                log::debug!(
                    "[global_tree::seal] cascade done tree_id={} stop_level={} count={}",
                    tree.id,
                    level,
                    buf.item_ids.len()
                );
                break;
            }

            let summary_id = seal_one_level(config, tree, &buf, summariser).await?;
            sealed_ids.push(summary_id);
            level += 1;
        }
    }

    Ok(sealed_ids)
}

/// Count-based threshold per level. L0→L1 needs 7 daily nodes, L1→L2 needs
/// 4 weekly nodes, L2→L3 needs 12 monthly nodes. Levels ≥ 3 never seal in
/// this phase — a yearly node is the top of the global tree.
fn should_seal(buf: &Buffer, level: u32) -> bool {
    let threshold = match level {
        0 => WEEKLY_SEAL_THRESHOLD,
        1 => MONTHLY_SEAL_THRESHOLD,
        2 => YEARLY_SEAL_THRESHOLD,
        _ => return false,
    };
    !buf.is_empty() && buf.item_ids.len() >= threshold
}

async fn seal_one_level(
    config: &Config,
    tree: &Tree,
    buf: &Buffer,
    summariser: &dyn Summariser,
) -> Result<String> {
    let level = buf.level;
    let target_level = level + 1;

    let inputs = hydrate_summary_inputs(config, &buf.item_ids)?;
    if inputs.is_empty() {
        anyhow::bail!(
            "[global_tree::seal] refused to seal empty buffer tree_id={} level={}",
            tree.id,
            level
        );
    }

    let time_range_start = inputs
        .iter()
        .map(|i| i.time_range_start)
        .min()
        .unwrap_or_else(Utc::now);
    let time_range_end = inputs
        .iter()
        .map(|i| i.time_range_end)
        .max()
        .unwrap_or_else(Utc::now);
    let score = inputs
        .iter()
        .map(|i| i.score)
        .fold(f32::NEG_INFINITY, f32::max)
        .max(0.0);

    let ctx = SummaryContext {
        tree_id: &tree.id,
        tree_kind: TreeKind::Global,
        target_level,
        token_budget: GLOBAL_TOKEN_BUDGET,
    };
    let output = summariser
        .summarise(&inputs, &ctx)
        .await
        .context("summariser failed during global seal")?;

    let now = Utc::now();
    let summary_id = new_summary_id(target_level);
    let node = SummaryNode {
        id: summary_id.clone(),
        tree_id: tree.id.clone(),
        tree_kind: TreeKind::Global,
        level: target_level,
        parent_id: None,
        child_ids: buf.item_ids.clone(),
        content: output.content,
        token_count: output.token_count,
        entities: output.entities,
        topics: output.topics,
        time_range_start,
        time_range_end,
        score,
        sealed_at: now,
        deleted: false,
    };

    // Single write transaction: insert the new summary, clear this level's
    // buffer, append the new id to the parent buffer, and bump the tree's
    // max_level/root_id if we just climbed. Re-read `max_level` inside the
    // tx so cascading seals within one call see the bump from earlier
    // iterations.
    let summary_id_for_closure = summary_id.clone();
    let target_level_for_closure = target_level;
    let tree_id = tree.id.clone();
    with_connection(config, move |conn| {
        let tx = conn.unchecked_transaction()?;

        let current_max: u32 = tx
            .query_row(
                "SELECT max_level FROM mem_tree_trees WHERE id = ?1",
                rusqlite::params![&tree_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n.max(0) as u32)
            .context("Failed to read current max_level for global tree")?;

        store::insert_summary_tx(&tx, &node)?;
        // Index any entities the summariser emitted. No-op under
        // InertSummariser (entities stays empty by design — see
        // summariser/inert.rs). Becomes active when the Ollama summariser
        // lands and emits curated canonical ids.
        crate::openhuman::memory::tree::score::store::index_summary_entity_ids_tx(
            &tx,
            &node.entities,
            &node.id,
            node.score,
            now.timestamp_millis(),
            Some(&tree_id),
        )?;
        // Backlink children → new parent. In the global tree every level is
        // already a summary, so the backlink always targets
        // `mem_tree_summaries`.
        for child_id in &node.child_ids {
            tx.execute(
                "UPDATE mem_tree_summaries
                    SET parent_id = ?1
                  WHERE id = ?2 AND parent_id IS NULL",
                rusqlite::params![&summary_id_for_closure, child_id],
            )
            .context("Failed to backlink global summary to parent summary")?;
        }
        store::clear_buffer_tx(&tx, &tree_id, level)?;

        // Append to parent buffer.
        let mut parent = store::get_buffer_conn(&tx, &tree_id, target_level_for_closure)?;
        parent.item_ids.push(summary_id_for_closure.clone());
        parent.token_sum = parent.token_sum.saturating_add(node.token_count as i64);
        parent.oldest_at = match parent.oldest_at {
            Some(existing) => Some(existing.min(time_range_start)),
            None => Some(time_range_start),
        };
        store::upsert_buffer_tx(&tx, &parent)?;

        // Update tree root / max_level if we just climbed.
        if target_level_for_closure > current_max {
            store::update_tree_after_seal_tx(
                &tx,
                &tree_id,
                &summary_id_for_closure,
                target_level_for_closure,
                now,
            )?;
        } else {
            // Same max level — refresh last_sealed_at only.
            tx.execute(
                "UPDATE mem_tree_trees SET last_sealed_at_ms = ?1 WHERE id = ?2",
                rusqlite::params![now.timestamp_millis(), &tree_id],
            )
            .context("Failed to refresh last_sealed_at for global tree")?;
        }

        tx.commit()?;
        Ok(())
    })?;

    log::info!(
        "[global_tree::seal] sealed tree_id={} level={}→{} summary_id={} children={}",
        tree.id,
        level,
        target_level,
        summary_id,
        buf.item_ids.len()
    );

    Ok(summary_id)
}

/// Hydrate summary rows for the ids in a buffer. Global-tree buffers at
/// every level reference summary nodes (not chunks), so we always pull from
/// `mem_tree_summaries`.
fn hydrate_summary_inputs(config: &Config, summary_ids: &[String]) -> Result<Vec<SummaryInput>> {
    let mut out: Vec<SummaryInput> = Vec::with_capacity(summary_ids.len());
    for id in summary_ids {
        let node = match store::get_summary(config, id)? {
            Some(n) => n,
            None => {
                log::warn!(
                    "[global_tree::seal] hydrate_summary_inputs: missing summary {id} — skipping"
                );
                continue;
            }
        };
        out.push(SummaryInput {
            id: node.id.clone(),
            content: node.content.clone(),
            token_count: node.token_count,
            entities: node.entities.clone(),
            topics: node.topics.clone(),
            time_range_start: node.time_range_start,
            time_range_end: node.time_range_end,
            score: node.score,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::global_tree::registry::get_or_create_global_tree;
    use crate::openhuman::memory::tree::source_tree::summariser::inert::InertSummariser;
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    fn mk_daily(id: &str, tree_id: &str, day_ms: i64) -> SummaryNode {
        let ts = Utc.timestamp_millis_opt(day_ms).single().unwrap();
        SummaryNode {
            id: id.to_string(),
            tree_id: tree_id.to_string(),
            tree_kind: TreeKind::Global,
            level: 0,
            parent_id: None,
            child_ids: vec![], // not used by seal hydrator
            content: format!("daily digest {id}"),
            token_count: 200,
            entities: vec![],
            topics: vec![],
            time_range_start: ts,
            time_range_end: ts,
            score: 0.5,
            sealed_at: ts,
            deleted: false,
        }
    }

    fn insert_daily(cfg: &Config, node: &SummaryNode) {
        with_connection(cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            store::insert_summary_tx(&tx, node)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
    }

    #[tokio::test]
    async fn below_threshold_does_not_seal() {
        let (_tmp, cfg) = test_config();
        let tree = get_or_create_global_tree(&cfg).unwrap();
        let summariser = InertSummariser::new();

        // Append 3 daily nodes — well below the 7-day weekly threshold.
        for i in 0..3 {
            let node = mk_daily(
                &format!("summary:L0:day{i}"),
                &tree.id,
                1_700_000_000_000 + i,
            );
            insert_daily(&cfg, &node);
            let sealed = append_daily_and_cascade(&cfg, &tree, &node, &summariser)
                .await
                .unwrap();
            assert!(sealed.is_empty(), "no cascade expected below threshold");
        }

        let buf = store::get_buffer(&cfg, &tree.id, 0).unwrap();
        assert_eq!(buf.item_ids.len(), 3);
    }

    #[tokio::test]
    async fn crossing_weekly_threshold_seals_l1() {
        let (_tmp, cfg) = test_config();
        let tree = get_or_create_global_tree(&cfg).unwrap();
        let summariser = InertSummariser::new();

        // Append exactly 7 daily nodes — should trigger one L0→L1 seal.
        for i in 0..WEEKLY_SEAL_THRESHOLD {
            let node = mk_daily(
                &format!("summary:L0:day{i}"),
                &tree.id,
                1_700_000_000_000 + i as i64,
            );
            insert_daily(&cfg, &node);
            let sealed = append_daily_and_cascade(&cfg, &tree, &node, &summariser)
                .await
                .unwrap();
            if i + 1 < WEEKLY_SEAL_THRESHOLD {
                assert!(sealed.is_empty(), "no seal before threshold (i={i})");
            } else {
                assert_eq!(sealed.len(), 1, "expected one weekly seal on 7th append");
            }
        }

        // L0 buffer cleared; L1 buffer holds the new weekly summary.
        let l0 = store::get_buffer(&cfg, &tree.id, 0).unwrap();
        assert!(l0.is_empty());
        let l1 = store::get_buffer(&cfg, &tree.id, 1).unwrap();
        assert_eq!(l1.item_ids.len(), 1);

        // Tree metadata reflects the climb to level 1.
        let t = store::get_tree(&cfg, &tree.id).unwrap().unwrap();
        assert_eq!(t.max_level, 1);
        assert_eq!(t.root_id.as_deref(), Some(l1.item_ids[0].as_str()));
        assert!(t.last_sealed_at.is_some());

        // Weekly summary row carries children = the 7 daily ids.
        let weekly = store::get_summary(&cfg, &l1.item_ids[0]).unwrap().unwrap();
        assert_eq!(weekly.level, 1);
        assert_eq!(weekly.tree_kind, TreeKind::Global);
        assert_eq!(weekly.child_ids.len(), WEEKLY_SEAL_THRESHOLD);
    }

    #[tokio::test]
    async fn append_is_idempotent_on_retry() {
        let (_tmp, cfg) = test_config();
        let tree = get_or_create_global_tree(&cfg).unwrap();
        let summariser = InertSummariser::new();

        let node = mk_daily("summary:L0:dayA", &tree.id, 1_700_000_000_000);
        insert_daily(&cfg, &node);
        append_daily_and_cascade(&cfg, &tree, &node, &summariser)
            .await
            .unwrap();
        append_daily_and_cascade(&cfg, &tree, &node, &summariser)
            .await
            .unwrap();

        let buf = store::get_buffer(&cfg, &tree.id, 0).unwrap();
        assert_eq!(
            buf.item_ids.len(),
            1,
            "retry must not double-insert the same daily id"
        );
        assert_eq!(buf.token_sum, 200);
    }
}
