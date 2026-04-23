//! Append + cascade-seal for summary trees (#709).
//!
//! `append_leaf` pushes a persisted chunk into the L0 buffer of a tree. If
//! the buffer's running `token_sum` crosses `TOKEN_BUDGET`, the buffer
//! seals into a level-1 summary, its items move into the summary's
//! `child_ids`, the buffer clears, and the new summary id is queued at
//! level 2. The cascade continues upward until a buffer stays under the
//! token budget.
//!
//! Concurrency: Phase 3a assumes a single-process SQLite workspace. All
//! writes in one seal step run in a single transaction; the async
//! summariser call happens outside any open transaction so a slow LLM
//! doesn't hold DB locks. Callers should serialise `append_leaf` per
//! tree — ingest achieves this by processing one batch's chunks
//! sequentially inside its `persist` task. Blocking SQLite calls inside
//! this async function are acceptable for Phase 3a because the Inert
//! summariser does no real I/O; when a networked summariser lands, wrap
//! DB calls in `tokio::task::spawn_blocking` to keep the runtime healthy.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::Transaction;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::source_tree::registry::new_summary_id;
use crate::openhuman::memory::tree::source_tree::store;
use crate::openhuman::memory::tree::source_tree::summariser::{
    Summariser, SummaryContext, SummaryInput,
};
use crate::openhuman::memory::tree::source_tree::types::{
    Buffer, SummaryNode, Tree, TreeKind, TOKEN_BUDGET,
};
use crate::openhuman::memory::tree::store::with_connection;

/// Hard cap on cascade depth — prevents runaway loops if token accounting
/// ever slips. 32 levels at even a 2x fan-in is more than enough for any
/// realistic source.
const MAX_CASCADE_DEPTH: u32 = 32;

/// A single leaf being appended to an L0 buffer.
#[derive(Clone, Debug)]
pub struct LeafRef {
    pub chunk_id: String,
    pub token_count: u32,
    pub timestamp: DateTime<Utc>,
    pub content: String,
    pub entities: Vec<String>,
    pub topics: Vec<String>,
    pub score: f32,
}

/// Append a leaf to the source tree for `tree`, sealing buffers as they
/// fill. Returns the ids of any summaries that sealed during this call.
pub async fn append_leaf(
    config: &Config,
    tree: &Tree,
    leaf: &LeafRef,
    summariser: &dyn Summariser,
) -> Result<Vec<String>> {
    log::debug!(
        "[source_tree::bucket_seal] append_leaf tree_id={} leaf_id={} tokens={}",
        tree.id,
        leaf.chunk_id,
        leaf.token_count
    );

    // 1. Push leaf into L0 buffer (transactional).
    append_to_buffer(
        config,
        &tree.id,
        0,
        &leaf.chunk_id,
        leaf.token_count as i64,
        leaf.timestamp,
    )?;

    // 2. Cascade seals upward until a level stays under budget.
    cascade_seals(config, tree, summariser).await
}

/// Transactionally append a single item to `(tree_id, level)`'s buffer.
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
        // Idempotent on (tree_id, level, item_id): a retry after a failed
        // cascade (step 2 of append_leaf) is a no-op, so duplicated children
        // and double-counted tokens can't slip into the buffer. oldest_at
        // stays on first-seen.
        if buf.item_ids.iter().any(|existing| existing == item_id) {
            log::debug!(
                "[source_tree::bucket_seal] append_to_buffer: {item_id} already in buffer \
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
    cascade_all_from(config, tree, 0, summariser, None).await
}

/// Seal buffers starting at `start_level` and cascade upward. When
/// `force_now` is `Some`, the buffer at `start_level` is sealed regardless
/// of token budget (used by time-based flush). Upper levels are sealed
/// only when they cross the budget.
pub async fn cascade_all_from(
    config: &Config,
    tree: &Tree,
    start_level: u32,
    summariser: &dyn Summariser,
    force_now: Option<DateTime<Utc>>,
) -> Result<Vec<String>> {
    let mut sealed_ids: Vec<String> = Vec::new();
    let mut level: u32 = start_level;
    let mut first_iteration = true;

    for _ in 0..MAX_CASCADE_DEPTH {
        let buf = store::get_buffer(config, &tree.id, level)?;
        let forced = first_iteration && force_now.is_some();
        first_iteration = false;

        if !forced && !should_seal(&buf) {
            log::debug!(
                "[source_tree::bucket_seal] cascade done tree_id={} stop_level={} token_sum={}",
                tree.id,
                level,
                buf.token_sum
            );
            break;
        }
        if buf.is_empty() {
            log::debug!(
                "[source_tree::bucket_seal] cascade hit empty buffer tree_id={} level={} — stopping",
                tree.id,
                level
            );
            break;
        }

        let summary_id = seal_one_level(config, tree, &buf, summariser).await?;
        sealed_ids.push(summary_id);
        level += 1;
    }

    Ok(sealed_ids)
}

fn should_seal(buf: &Buffer) -> bool {
    !buf.is_empty() && buf.token_sum >= TOKEN_BUDGET as i64
}

/// Seal `buf` at `level` into one summary at `level + 1`. Returns the new
/// summary id.
async fn seal_one_level(
    config: &Config,
    tree: &Tree,
    buf: &Buffer,
    summariser: &dyn Summariser,
) -> Result<String> {
    let level = buf.level;
    let target_level = level + 1;

    // Hydrate inputs (synchronous DB reads).
    let inputs = hydrate_inputs(config, level, &buf.item_ids)?;
    if inputs.is_empty() {
        anyhow::bail!(
            "[source_tree::bucket_seal] refused to seal empty buffer tree_id={} level={}",
            tree.id,
            level
        );
    }

    // Compute envelope across children (time range, max score).
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

    // Run summariser — async, OUTSIDE any DB transaction.
    let ctx = SummaryContext {
        tree_id: &tree.id,
        tree_kind: TreeKind::Source,
        target_level,
        token_budget: TOKEN_BUDGET,
    };
    let output = summariser
        .summarise(&inputs, &ctx)
        .await
        .context("summariser failed during seal")?;

    // Build the new summary node.
    let now = Utc::now();
    let summary_id = new_summary_id(target_level);
    let node = SummaryNode {
        id: summary_id.clone(),
        tree_id: tree.id.clone(),
        tree_kind: TreeKind::Source,
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

    // Single write transaction: insert summary, clear this buffer, append
    // summary id to parent buffer, bump tree max_level/root if needed.
    // Re-read `max_level` from inside the tx so cascading seals within
    // one call see the updated value from earlier levels.
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
            .context("Failed to read current max_level for tree")?;

        store::insert_summary_tx(&tx, &node)?;
        // Forward-compat: index any entities the summariser emitted into
        // `mem_tree_entity_index` so Phase 4 retrieval can resolve
        // "summaries mentioning Alice" via the same inverted index as
        // leaves. No-op under InertSummariser (entities is empty by
        // design — see summariser/inert.rs doc); becomes active once the
        // Ollama summariser lands and emits curated canonical ids.
        crate::openhuman::memory::tree::score::store::index_summary_entity_ids_tx(
            &tx,
            &node.entities,
            &node.id,
            node.score,
            now.timestamp_millis(),
            Some(&tree_id),
        )?;
        // Backlink children → new parent so leaf/parent traversal is a
        // single-row lookup in Phase 4. Skipped for levels already bound
        // to a parent (shouldn't happen — a child seals at most once).
        for child_id in &node.child_ids {
            if level == 0 {
                tx.execute(
                    "UPDATE mem_tree_chunks
                        SET parent_summary_id = ?1
                      WHERE id = ?2 AND parent_summary_id IS NULL",
                    rusqlite::params![&summary_id_for_closure, child_id],
                )
                .context("Failed to backlink chunk to parent summary")?;
            } else {
                tx.execute(
                    "UPDATE mem_tree_summaries
                        SET parent_id = ?1
                      WHERE id = ?2 AND parent_id IS NULL",
                    rusqlite::params![&summary_id_for_closure, child_id],
                )
                .context("Failed to backlink summary to parent summary")?;
            }
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
            // Same max level — still refresh last_sealed_at via same helper
            // but keep existing root intact. Root tracking at the same
            // level is resolved in Phase 4 retrieval.
            refresh_last_sealed_tx(&tx, &tree_id, now)?;
        }

        tx.commit()?;
        Ok(())
    })?;

    log::info!(
        "[source_tree::bucket_seal] sealed tree_id={} level={}→{} summary_id={} children={}",
        tree.id,
        level,
        target_level,
        summary_id,
        buf.item_ids.len()
    );

    Ok(summary_id)
}

fn refresh_last_sealed_tx(
    tx: &Transaction<'_>,
    tree_id: &str,
    sealed_at: DateTime<Utc>,
) -> Result<()> {
    tx.execute(
        "UPDATE mem_tree_trees SET last_sealed_at_ms = ?1 WHERE id = ?2",
        rusqlite::params![sealed_at.timestamp_millis(), tree_id],
    )
    .with_context(|| format!("Failed to refresh last_sealed_at for tree {tree_id}"))?;
    Ok(())
}

/// Fetch contributions for `item_ids`. At level 0 we pull from
/// `mem_tree_chunks` + `mem_tree_score`; at ≥1 we pull from
/// `mem_tree_summaries`.
fn hydrate_inputs(config: &Config, level: u32, item_ids: &[String]) -> Result<Vec<SummaryInput>> {
    if level == 0 {
        hydrate_leaf_inputs(config, item_ids)
    } else {
        hydrate_summary_inputs(config, item_ids)
    }
}

fn hydrate_leaf_inputs(config: &Config, chunk_ids: &[String]) -> Result<Vec<SummaryInput>> {
    use crate::openhuman::memory::tree::score::store::get_score;
    use crate::openhuman::memory::tree::store::get_chunk;

    let mut out: Vec<SummaryInput> = Vec::with_capacity(chunk_ids.len());
    for id in chunk_ids {
        let chunk = match get_chunk(config, id)? {
            Some(c) => c,
            None => {
                log::warn!(
                    "[source_tree::bucket_seal] hydrate_leaf_inputs: missing chunk {id} — skipping"
                );
                continue;
            }
        };
        let score = get_score(config, id)?;
        let (score_value, entities, topics) = match &score {
            Some(row) => (row.total, Vec::new(), chunk.metadata.tags.clone()),
            None => (0.0, Vec::new(), chunk.metadata.tags.clone()),
        };
        out.push(SummaryInput {
            id: chunk.id.clone(),
            content: chunk.content.clone(),
            token_count: chunk.token_count,
            entities,
            topics,
            time_range_start: chunk.metadata.time_range.0,
            time_range_end: chunk.metadata.time_range.1,
            score: score_value,
        });
    }
    Ok(out)
}

fn hydrate_summary_inputs(config: &Config, summary_ids: &[String]) -> Result<Vec<SummaryInput>> {
    let mut out: Vec<SummaryInput> = Vec::with_capacity(summary_ids.len());
    for id in summary_ids {
        let node = match store::get_summary(config, id)? {
            Some(n) => n,
            None => {
                log::warn!(
                    "[source_tree::bucket_seal] hydrate_summary_inputs: missing summary {id} — skipping"
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
#[path = "bucket_seal_tests.rs"]
mod tests;
