//! Append + cascade-seal for summary trees (#709).
//!
//! `append_leaf` pushes a persisted chunk into the L0 buffer of a tree.
//! Seal gates differ by level:
//!
//! - **L0 (leaves → L1)**: seal when `token_sum >= TOKEN_BUDGET`. Bounds
//!   the summariser's raw input.
//! - **L≥1 (summaries → next level)**: seal when `item_ids.len() >=
//!   SUMMARY_FANOUT`. Per-summary token size depends on summariser
//!   quality, so a token-based gate collapses to a 1:1:1 chain when the
//!   summariser is weak. Counting siblings keeps the tree's fan-in
//!   stable regardless.
//!
//! When a buffer seals, its items move into the new summary's
//! `child_ids`, the buffer clears, and the new summary id is queued at
//! the next level. The cascade continues upward until a buffer fails its
//! gate.
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

use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::Transaction;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::content_store::{
    atomic::stage_summary, paths::slugify_source_id, SummaryComposeInput, SummaryTreeKind,
};
use crate::openhuman::memory::tree::score::embed::build_embedder_from_config;
use crate::openhuman::memory::tree::score::extract::EntityExtractor;
use crate::openhuman::memory::tree::score::resolver::canonicalise;
use crate::openhuman::memory::tree::store::with_connection;
use crate::openhuman::memory::tree::tree_source::registry::new_summary_id;
use crate::openhuman::memory::tree::tree_source::store;
use crate::openhuman::memory::tree::tree_source::summariser::{
    Summariser, SummaryContext, SummaryInput,
};
use crate::openhuman::memory::tree::tree_source::types::{
    Buffer, SummaryNode, Tree, TreeKind, SUMMARY_FANOUT, TOKEN_BUDGET,
};

/// Hard cap on cascade depth — prevents runaway loops if token accounting
/// ever slips. 32 levels at even a 2x fan-in is more than enough for any
/// realistic source.
const MAX_CASCADE_DEPTH: u32 = 32;

/// How a sealed summary node's `entities` and `topics` fields get populated.
///
/// Each tree kind has different correct semantics:
/// - **Source** trees use [`LabelStrategy::ExtractFromContent`] so the
///   summariser's freshly-synthesised text gets its own pass through an
///   extractor. Captures emergent themes that no individual leaf expressed.
/// - **Global** trees use [`LabelStrategy::UnionFromChildren`] — their
///   inputs are already-labeled source-tree summaries; union preserves
///   labels for time-based retrieval ("days that mentioned Alice")
///   without an LLM call.
/// - **Topic** trees use [`LabelStrategy::Empty`] — their scope already
///   pins the dominant theme; inheriting auxiliary entities would
///   cross-pollinate unrelated topic trees and noise the entity index.
#[derive(Clone)]
pub enum LabelStrategy {
    /// Run the extractor on the new summary's content; canonicalise the
    /// result into `entities` (canonical_ids) and `topics` (labels).
    ExtractFromContent(Arc<dyn EntityExtractor>),
    /// Dedup-merge each input's `entities` and `topics` into the parent.
    UnionFromChildren,
    /// Leave both fields empty regardless of inputs.
    Empty,
}

impl std::fmt::Debug for LabelStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExtractFromContent(ex) => write!(f, "ExtractFromContent({})", ex.name()),
            Self::UnionFromChildren => f.write_str("UnionFromChildren"),
            Self::Empty => f.write_str("Empty"),
        }
    }
}

/// Resolve `entities` and `topics` for a freshly-summarised node according
/// to the chosen strategy. Errors propagate from the extractor (when used).
async fn resolve_labels(
    strategy: &LabelStrategy,
    inputs: &[SummaryInput],
    summary_content: &str,
) -> Result<(Vec<String>, Vec<String>)> {
    match strategy {
        LabelStrategy::ExtractFromContent(extractor) => {
            let extracted = extractor
                .extract(summary_content)
                .await
                .context("seal-time extractor failed")?;
            let canonical = canonicalise(&extracted);
            let mut entities: Vec<String> = canonical
                .into_iter()
                .map(|c| c.canonical_id)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            entities.sort();
            let mut topics: Vec<String> = extracted
                .topics
                .into_iter()
                .map(|t| t.label)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            topics.sort();
            Ok((entities, topics))
        }
        LabelStrategy::UnionFromChildren => {
            let mut entities: BTreeSet<String> = BTreeSet::new();
            let mut topics: BTreeSet<String> = BTreeSet::new();
            for inp in inputs {
                for e in &inp.entities {
                    entities.insert(e.clone());
                }
                for t in &inp.topics {
                    topics.insert(t.clone());
                }
            }
            Ok((entities.into_iter().collect(), topics.into_iter().collect()))
        }
        LabelStrategy::Empty => Ok((Vec::new(), Vec::new())),
    }
}

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
///
/// `strategy` controls how each sealed summary's `entities` and `topics`
/// are populated — see [`LabelStrategy`].
pub async fn append_leaf(
    config: &Config,
    tree: &Tree,
    leaf: &LeafRef,
    summariser: &dyn Summariser,
    strategy: &LabelStrategy,
) -> Result<Vec<String>> {
    log::debug!(
        "[tree_source::bucket_seal] append_leaf tree_id={} leaf_id={} tokens={} strategy={:?}",
        tree.id,
        leaf.chunk_id,
        leaf.token_count,
        strategy
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
    cascade_seals(config, tree, summariser, strategy).await
}

/// Queue-oriented variant of [`append_leaf`].
///
/// This only appends the leaf to the L0 buffer and returns whether the
/// caller should enqueue a follow-up seal job for level 0.
pub fn append_leaf_deferred(config: &Config, tree: &Tree, leaf: &LeafRef) -> Result<bool> {
    append_to_buffer(
        config,
        &tree.id,
        0,
        &leaf.chunk_id,
        leaf.token_count as i64,
        leaf.timestamp,
    )?;
    let buf = store::get_buffer(config, &tree.id, 0)?;
    Ok(should_seal(&buf))
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
                "[tree_source::bucket_seal] append_to_buffer: {item_id} already in buffer \
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
    strategy: &LabelStrategy,
) -> Result<Vec<String>> {
    cascade_all_from(config, tree, 0, summariser, None, strategy).await
}

/// Seal buffers starting at `start_level` and cascade upward. When
/// `force_now` is `Some`, the buffer at `start_level` is sealed regardless
/// of token budget (used by time-based flush). Upper levels are sealed
/// only when they cross the budget.
///
/// `strategy` is forwarded to every sealed level — same semantics as
/// [`append_leaf`].
pub async fn cascade_all_from(
    config: &Config,
    tree: &Tree,
    start_level: u32,
    summariser: &dyn Summariser,
    force_now: Option<DateTime<Utc>>,
    strategy: &LabelStrategy,
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
                "[tree_source::bucket_seal] cascade done tree_id={} stop_level={} token_sum={}",
                tree.id,
                level,
                buf.token_sum
            );
            break;
        }
        if buf.is_empty() {
            log::debug!(
                "[tree_source::bucket_seal] cascade hit empty buffer tree_id={} level={} — stopping",
                tree.id,
                level
            );
            break;
        }

        // Sync cascade — drives the level walk itself; doesn't need the
        // queue follow-ups (we'll hit `seal_one_level` again next iter).
        let summary_id = seal_one_level(config, tree, &buf, summariser, strategy, false).await?;
        sealed_ids.push(summary_id);
        level += 1;
    }

    Ok(sealed_ids)
}

/// Level-aware seal gate.
///
/// L0 buffers (raw leaves) gate on `token_sum` so the summariser's input
/// stays bounded. L≥1 buffers gate on sibling count so the tree's
/// fan-in is independent of per-summary token size — without this,
/// summarisers that emit at the full token budget (e.g. the inert
/// fallback) collapse the cascade into a 1:1:1 chain instead of a real
/// tree.
pub(crate) fn should_seal(buf: &Buffer) -> bool {
    if buf.is_empty() {
        return false;
    }
    if buf.level == 0 {
        buf.token_sum >= TOKEN_BUDGET as i64
    } else {
        (buf.item_ids.len() as u32) >= SUMMARY_FANOUT
    }
}

/// Seal `buf` at `level` into one summary at `level + 1`. Returns the new
/// summary id.
///
/// `strategy` decides how `entities` and `topics` get populated on the new
/// summary node — see [`LabelStrategy`].
///
/// When `enqueue_follow_ups` is `true`, the function additionally inserts
/// follow-up job rows **inside the same transaction** that commits the
/// seal:
/// - `seal { tree_id, level: parent_level }` if the parent buffer's gate
///   is now met (parent-cascade enqueue)
/// - `topic_route { NodeRef::Summary { summary_id } }` for source trees
///   (so summary-level entities feed the topic-tree spawn pipeline)
///
/// Atomic enqueue eliminates the crash window where a seal commits but
/// the post-commit follow-up enqueues are silently lost on a worker
/// crash. The async-pipeline handler (`handle_seal`) passes `true`. The
/// synchronous in-process cascade caller ([`cascade_all_from`]) passes
/// `false` because it drives the cascade itself and topic_route isn't
/// part of the test/flush sync path.
pub(crate) async fn seal_one_level(
    config: &Config,
    tree: &Tree,
    buf: &Buffer,
    summariser: &dyn Summariser,
    strategy: &LabelStrategy,
    enqueue_follow_ups: bool,
) -> Result<String> {
    let level = buf.level;
    let target_level = level + 1;

    // Hydrate inputs (synchronous DB reads).
    let inputs = hydrate_inputs(config, level, &buf.item_ids)?;
    if inputs.is_empty() {
        anyhow::bail!(
            "[tree_source::bucket_seal] refused to seal empty buffer tree_id={} level={}",
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
        tree_kind: tree.kind,
        target_level,
        token_budget: TOKEN_BUDGET,
    };
    let output = summariser
        .summarise(&inputs, &ctx)
        .await
        .context("summariser failed during seal")?;

    // Resolve labels (entities/topics) for the new summary node according
    // to the chosen strategy. Done before the write tx so an extractor
    // failure aborts the seal cleanly — same shape as the embedder guard
    // below.
    let (node_entities, node_topics) = resolve_labels(strategy, &inputs, &output.content).await?;

    // Phase 4 (#710): embed the summary BEFORE opening the write tx so an
    // embedder failure aborts the seal cleanly — nothing is persisted,
    // the buffer stays intact, and a retry re-embeds from scratch. The
    // tx below would otherwise commit a summary with no embedding,
    // polluting retrieval's semantic rerank.
    //
    // Embedder context-window guard: `nomic-embed-text-v1.5` accepts
    // up to 8192 tokens of input. Summary content is bounded by
    // `ctx.token_budget = TOKEN_BUDGET = 10_000` so a worst-case
    // summary overshoots the embedder. We truncate the input passed
    // to `embed()` to fit (the persisted summary content stays full;
    // only the embedding's "view" of it is clamped). 6000 tokens
    // leaves headroom for tokenizer differences across embedders.
    let embedder = build_embedder_from_config(config).context("build embedder during seal")?;
    // Conservative cap. Slack-style chat content (URLs, mentions,
    // emoji) tokenizes 2-4× higher than the 4-chars/token heuristic.
    // 1000 approx-tokens (~4000 chars) is comfortably under 8192
    // even at 4× tokenizer ratio.
    let embed_input = truncate_for_embed(&output.content, 1_000);
    log::info!(
        "[tree_source::bucket_seal] embed input: original_chars={} truncated_chars={}",
        output.content.len(),
        embed_input.len()
    );
    let embedding = embedder.embed(&embed_input).await.with_context(|| {
        format!(
            "embed summary during seal tree_id={} level={}",
            tree.id, level
        )
    })?;
    log::debug!(
        "[tree_source::bucket_seal] embedded summary tree_id={} level={}→{} bytes={} provider={}",
        tree.id,
        level,
        target_level,
        output.content.len(),
        embedder.name()
    );

    // Build the new summary node.
    let now = Utc::now();
    let summary_id = new_summary_id(target_level);
    let node = SummaryNode {
        id: summary_id.clone(),
        tree_id: tree.id.clone(),
        // `seal_one_level` runs for source AND topic trees (handle_seal,
        // cascade_all_from, flush). Hardcoding Source here would write
        // topic-tree summaries with tree_kind='source' in
        // mem_tree_summaries, breaking any query filtering on tree_kind.
        tree_kind: tree.kind,
        level: target_level,
        parent_id: None,
        child_ids: buf.item_ids.clone(),
        content: output.content,
        token_count: output.token_count,
        entities: node_entities,
        topics: node_topics,
        time_range_start,
        time_range_end,
        score,
        sealed_at: now,
        deleted: false,
        embedding: Some(embedding),
    };

    // Phase MD-content: stage the summary .md file BEFORE opening the write
    // tx. A staging failure aborts the seal cleanly — nothing is persisted
    // and the buffer stays intact for retry.
    //
    // `bucket_seal.rs` handles both Source and Topic tree seals (Topic trees
    // use the same cascade machinery via `handle_seal` in the job handler).
    // Map TreeKind to SummaryTreeKind accordingly.
    let summary_tree_kind = match tree.kind {
        TreeKind::Topic => SummaryTreeKind::Topic,
        _ => SummaryTreeKind::Source,
    };
    let scope_slug = {
        // Path slug semantics per source kind:
        //
        // - Gmail source trees: scope is `"gmail:<participants>"` where
        //   participants is `addr1|addr2|...`. Strip the `gmail:` prefix so the
        //   path is `summaries/source/<participants_slug>/...` and mirrors the
        //   chunk layout under `email/<participants_slug>/`.
        //
        // - Topic trees: scope is the canonical entity_id (e.g.
        //   `"email:alice@example.com"`). Slugify the FULL string so topic-tree
        //   summaries and source-tree summaries don't share a path prefix.
        //
        // - All other source kinds (slack:, discord:, document:, …): slugify the
        //   FULL scope string. Stripping the prefix for non-Gmail sources was a
        //   bug — `"slack:#eng"` and `"discord:#eng"` would both produce slug
        //   `"eng"` and collide in `summaries/source/eng/`.
        let s = &tree.scope;
        match tree.kind {
            TreeKind::Topic => slugify_source_id(s),
            _ => {
                if s.starts_with("gmail:") {
                    // Strip "gmail:" prefix; slugify the participants portion.
                    slugify_source_id(&s["gmail:".len()..])
                } else {
                    // All other source kinds: slugify the full scope string.
                    slugify_source_id(s)
                }
            }
        }
    };
    // For L1 seals (children are chunks), point each child wikilink at
    // the raw archive file the chunk's body lives in — the email
    // chunk-store path `email/<scope>/<chunk_id>.md` no longer
    // exists, so `[[<chunk_id>]]` would be an unresolved Obsidian
    // link. The raw file basename `<ts_ms>_<msg_id>` matches what
    // `raw_store::write_raw_items` writes. L≥2 children are
    // summary ids whose default `sanitize_filename` resolves to
    // existing `wiki/summaries/...md` files — leave overrides
    // unset there.
    let child_basename_overrides: Option<Vec<Option<String>>> = if node.level == 1 {
        let overrides: Vec<Option<String>> = node
            .child_ids
            .iter()
            .map(|chunk_id| {
                // Surface lookup failures explicitly — silently
                // falling back to `[[<chunk_hash>]]` would commit an
                // unresolved Obsidian wikilink without any signal.
                // We still yield `None` (so `compose_summary_md`
                // takes the sanitised-id fallback) but a warn log
                // makes the SQL error visible for diagnosis.
                match crate::openhuman::memory::tree::store::get_chunk_raw_refs(config, chunk_id) {
                    Ok(refs_opt) => {
                        refs_opt
                            .and_then(|refs| refs.into_iter().next())
                            .and_then(|r| {
                                std::path::Path::new(&r.path)
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .map(|s| s.to_string())
                            })
                    }
                    Err(e) => {
                        log::warn!(
                            "[tree_source::bucket_seal] get_chunk_raw_refs failed \
                             chunk_id={chunk_id} err={e:#} — falling back to \
                             chunk_id-based wikilink"
                        );
                        None
                    }
                }
            })
            .collect();
        Some(overrides)
    } else {
        None
    };
    let compose_input = SummaryComposeInput {
        summary_id: &node.id,
        tree_kind: summary_tree_kind,
        tree_id: &node.tree_id,
        tree_scope: &tree.scope,
        level: node.level,
        child_ids: &node.child_ids,
        child_basenames: child_basename_overrides.as_deref(),
        child_count: node.child_ids.len(),
        time_range_start: node.time_range_start,
        time_range_end: node.time_range_end,
        sealed_at: node.sealed_at,
        body: &node.content,
    };
    // Stage the summary .md file and propagate any error — a staging failure
    // aborts the seal entirely so the database never commits a row with
    // content_path = NULL. The buffer stays unsealed and the job-retry path
    // will re-attempt the file write on next execution.
    let content_root = config.memory_tree_content_root();
    let staged =
        stage_summary(&content_root, &compose_input, &scope_slug, None).with_context(|| {
            format!(
                "stage_summary failed for {}; seal aborted, buffer stays unsealed for retry",
                node.id
            )
        })?;
    log::debug!(
        "[tree_source::bucket_seal] staged summary {} → {}",
        node.id,
        staged.content_path
    );

    // Single write transaction: insert summary, clear this buffer, append
    // summary id to parent buffer, bump tree max_level/root if needed,
    // and (when `enqueue_follow_ups`) atomically enqueue parent-seal +
    // topic_route follow-ups so they can never desync from the commit.
    // Re-read `max_level` from inside the tx so cascading seals within
    // one call see the updated value from earlier levels.
    let summary_id_for_closure = summary_id.clone();
    let target_level_for_closure = target_level;
    let tree_id = tree.id.clone();
    let tree_kind = tree.kind;
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

        store::insert_summary_tx(&tx, &node, Some(&staged))?;
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

        // Atomic follow-up enqueues. Done INSIDE this tx — if the commit
        // rolls back, the queue rows go with it; if it succeeds, the
        // rows are durably visible to the worker pool. Eliminates the
        // crash window where the seal commits but post-commit enqueues
        // are lost.
        if enqueue_follow_ups {
            // Parent-cascade: if the new summary made the parent buffer
            // cross its gate, enqueue the next level's seal. Dedupe key
            // `seal:{tree_id}:{parent_level}` prevents duplicates if a
            // parallel path already queued it.
            if should_seal(&parent) {
                use crate::openhuman::memory::tree::jobs::store::enqueue_tx as enqueue_job_tx;
                use crate::openhuman::memory::tree::jobs::types::{NewJob, SealPayload};
                let parent_seal = SealPayload {
                    tree_id: tree_id.clone(),
                    level: target_level_for_closure,
                    force_now_ms: None,
                };
                enqueue_job_tx(&tx, &NewJob::seal(&parent_seal)?)?;
            }
            // Source-tree summary routing: feed the new summary's
            // entities back into the topic-tree spawn pipeline. Topic
            // and global trees are sinks — no fan-out from their seals.
            if matches!(tree_kind, TreeKind::Source) {
                use crate::openhuman::memory::tree::jobs::store::enqueue_tx as enqueue_job_tx;
                use crate::openhuman::memory::tree::jobs::types::{
                    NewJob, NodeRef, TopicRoutePayload,
                };
                let route = TopicRoutePayload {
                    node: NodeRef::Summary {
                        summary_id: summary_id_for_closure.clone(),
                    },
                };
                enqueue_job_tx(&tx, &NewJob::topic_route(&route)?)?;
            }
        }

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
        "[tree_source::bucket_seal] sealed tree_id={} level={}→{} summary_id={} children={}",
        tree.id,
        level,
        target_level,
        summary_id,
        buf.item_ids.len()
    );

    Ok(summary_id)
}

/// Clamp `text` to roughly `max_tokens` tokens before passing to the
/// embedder. Uses the same ~4 chars/token heuristic as
/// `approx_token_count`. Embedders have hard input-size limits (e.g.
/// `nomic-embed-text-v1.5` = 8192 tokens) and an overshoot returns
/// HTTP 500 from Ollama rather than auto-truncating, which would
/// abort the seal transaction.
fn truncate_for_embed(text: &str, max_tokens: u32) -> String {
    let approx = crate::openhuman::memory::tree::types::approx_token_count(text);
    if approx <= max_tokens {
        return text.to_string();
    }
    let char_ceiling = (max_tokens as usize).saturating_mul(4);
    text.chars().take(char_ceiling).collect()
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
    use crate::openhuman::memory::tree::content_store::read as content_read;
    use crate::openhuman::memory::tree::score::store::{get_score, list_entity_ids_for_node};
    use crate::openhuman::memory::tree::store::get_chunk;

    let mut out: Vec<SummaryInput> = Vec::with_capacity(chunk_ids.len());
    for id in chunk_ids {
        let chunk = match get_chunk(config, id)? {
            Some(c) => c,
            None => {
                log::warn!(
                    "[tree_source::bucket_seal] hydrate_leaf_inputs: missing chunk {id} — skipping"
                );
                continue;
            }
        };
        let score_value = get_score(config, id)?.map(|row| row.total).unwrap_or(0.0);
        // Pull canonical entity ids from the inverted index — that's the
        // authoritative source for "what entities are attached to this
        // chunk." Topics live on the chunk's metadata tags.
        // [`LabelStrategy::UnionFromChildren`] reads these fields off
        // each `SummaryInput` to roll labels up the tree.
        let entities = list_entity_ids_for_node(config, id).unwrap_or_default();
        // Read the full body from disk — the `content` column in SQLite holds
        // a ≤500-char preview after the MD-on-disk migration. The summariser
        // must receive the complete chunk text so the seal output is not a
        // summary of previews.
        //
        // For pre-MD-migration chunks (no content_path recorded) this call
        // returns Err; callers that want to handle legacy rows should check
        // content_path presence before calling hydrate_inputs.
        let body = content_read::read_chunk_body(config, id).with_context(|| {
            format!("[tree_source::bucket_seal] hydrate_leaf_inputs: read body for chunk {id}")
        })?;
        out.push(SummaryInput {
            id: chunk.id.clone(),
            content: body,
            token_count: chunk.token_count,
            entities,
            topics: chunk.metadata.tags.clone(),
            time_range_start: chunk.metadata.time_range.0,
            time_range_end: chunk.metadata.time_range.1,
            score: score_value,
        });
    }
    Ok(out)
}

fn hydrate_summary_inputs(config: &Config, summary_ids: &[String]) -> Result<Vec<SummaryInput>> {
    use crate::openhuman::memory::tree::content_store::read as content_read;

    let mut out: Vec<SummaryInput> = Vec::with_capacity(summary_ids.len());
    for id in summary_ids {
        let node = match store::get_summary(config, id)? {
            Some(n) => n,
            None => {
                log::warn!(
                    "[tree_source::bucket_seal] hydrate_summary_inputs: missing summary {id} — skipping"
                );
                continue;
            }
        };
        // Read the full body from disk — `node.content` is a ≤500-char preview
        // after the MD-on-disk migration. Higher-level seals (L2+) summarise
        // over L1 summary content and need the full text, not a preview.
        let body = content_read::read_summary_body(config, id).with_context(|| {
            format!("[tree_source::bucket_seal] hydrate_summary_inputs: read body for summary {id}")
        })?;
        out.push(SummaryInput {
            id: node.id.clone(),
            content: body,
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
