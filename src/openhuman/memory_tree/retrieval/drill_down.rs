//! `memory_tree_drill_down` — walk `child_ids` from a summary node (Phase 4
//! / #710).
//!
//! Primary use case: the LLM gets a summary hit back from `query_source` or
//! `query_topic` and wants to look at the next level down — either more
//! summaries (for L2+ nodes) or the raw chunks (for L1 nodes). This is
//! deliberately a one-step expansion; for multi-step walks the caller
//! passes `max_depth > 1`.
//!
//! When `query` is `Some`, visited children are reranked by cosine similarity
//! against the query embedding so a deep summary with many children can surface
//! the relevant ones to the top. When `query` is `None`, children are returned
//! in BFS order (same as before).
//!
//! Behaviour:
//! - Unknown `node_id` → empty vec (not an error — the LLM can recover).
//! - `max_depth == 0` → empty vec (documented as "no-op").
//! - Leaves have no children; drilling into a leaf id returns empty.
//! - `limit` is optional; when set, it truncates the final (reranked) output.

use std::collections::{HashMap, HashSet};

use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::chunks::store::{
    get_chunk, get_chunk_embeddings_batch, get_chunks_batch,
};
use crate::openhuman::memory_store::content::read as content_read;
use crate::openhuman::memory_store::trees::store::{get_summaries_batch, get_trees_batch};
use crate::openhuman::memory_tree::retrieval::types::{
    hit_from_chunk, hit_from_summary, RetrievalHit,
};
use crate::openhuman::memory_tree::score::embed::{build_embedder_from_config, cosine_similarity};
use crate::openhuman::memory_tree::tree::store;

/// Upper-bound estimate of how many children a summary node fans out to,
/// used only to pre-size the next-level BFS frontier. Over-estimating wastes
/// a little transient capacity; under-estimating costs a realloc — neither is
/// load-bearing for correctness.
const EXPECTED_CHILD_FANOUT: usize = 10;

/// Walk the summary hierarchy down one step (or more if `max_depth > 1`)
/// and return the hydrated child hits. Children at level 1 are raw chunks;
/// deeper children are summaries.
///
/// When `query` is `Some`, the returned hits are reranked by cosine similarity
/// to the query embedding; hits without a stored embedding (legacy rows) sort
/// to the bottom. When `None`, BFS order is preserved.
pub async fn drill_down(
    config: &Config,
    node_id: &str,
    max_depth: u32,
    query: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<RetrievalHit>> {
    // Redact `node_id` — embeds tree scope (e.g. `summary:L1:<uuid>` or
    // `chat:slack:#<channel>:<seq>`) which can carry workspace hints. Log
    // the id's structural prefix only.
    let node_kind_prefix = node_id.split_once(':').map(|(k, _)| k).unwrap_or("unknown");
    log::debug!(
        "[retrieval::drill_down] drill_down node_kind={} max_depth={} has_query={} limit={:?}",
        node_kind_prefix,
        max_depth,
        query.is_some(),
        limit
    );
    if max_depth == 0 {
        log::debug!("[retrieval::drill_down] max_depth=0 — returning empty vec");
        return Ok(Vec::new());
    }

    // Phase 1 — blocking walk produces hits + the per-hit embedding so the
    // async rerank pass can avoid a second trip through the DB.
    let node_id_owned = node_id.to_string();
    let config_owned = config.clone();
    let (hits, embeddings) = tokio::task::spawn_blocking(
        move || -> Result<(Vec<RetrievalHit>, Vec<Option<Vec<f32>>>)> {
            walk_with_embeddings(&config_owned, &node_id_owned, max_depth)
        },
    )
    .await
    .map_err(|e| anyhow::anyhow!("drill_down join error: {e}"))??;

    // Phase 2 — optional query rerank.
    let hits = if let Some(q) = query {
        rerank_by_semantic_similarity(config, q, hits, embeddings).await?
    } else {
        hits
    };

    // Phase 3 — apply optional limit AFTER rerank so the top-K is relevance-
    // based when `query` is Some, BFS-based otherwise.
    let hits = match limit {
        Some(n) if hits.len() > n => hits.into_iter().take(n).collect(),
        _ => hits,
    };

    log::debug!("[retrieval::drill_down] returning hits={}", hits.len());
    Ok(hits)
}

/// Rerank hits by cosine similarity to the query embedding. Mirrors the
/// pattern used by `query_source` / `query_topic`. Legacy rows without
/// embeddings land at the end in BFS order.
///
/// On any error (embedder build failure or embedding inference failure) we log
/// a warning and return hits in BFS order rather than bubbling the error up
/// through the chat turn. This ensures local AI unavailability never surfaces
/// as a visible error to the user.
async fn rerank_by_semantic_similarity(
    config: &Config,
    query: &str,
    hits: Vec<RetrievalHit>,
    embeddings: Vec<Option<Vec<f32>>>,
) -> Result<Vec<RetrievalHit>> {
    debug_assert_eq!(hits.len(), embeddings.len());
    let embedder = match build_embedder_from_config(config) {
        Ok(e) => e,
        Err(err) => {
            log::warn!(
                "[retrieval::drill_down] embedder build failed — returning BFS order: {err}"
            );
            return Ok(hits);
        }
    };
    let query_vec = match embedder.embed(query).await {
        Ok(v) => v,
        Err(err) => {
            log::warn!("[retrieval::drill_down] embed query failed — returning BFS order: {err}");
            return Ok(hits);
        }
    };
    log::debug!(
        "[retrieval::drill_down] query embedded provider={} hits_to_rerank={}",
        embedder.name(),
        hits.len()
    );

    let mut decorated: Vec<(f32, bool, RetrievalHit)> = hits
        .into_iter()
        .zip(embeddings.into_iter())
        .map(|(h, emb)| match emb {
            Some(v) if v.len() == query_vec.len() => {
                let sim = cosine_similarity(&query_vec, &v);
                (sim, true, h)
            }
            _ => (f32::NEG_INFINITY, false, h),
        })
        .collect();

    decorated.sort_by(|a, b| match (a.1, b.1) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        // Both ranked (or both unranked): similarity DESC, then by time.
        _ => {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.2.time_range_end.cmp(&a.2.time_range_end))
        }
    });

    Ok(decorated.into_iter().map(|(_, _, h)| h).collect())
}

/// Blocking walker. BFS-style expansion up to `max_depth` levels. Returns
/// each hit paired with its stored embedding (if any), so the async rerank
/// pass doesn't have to round-trip through the DB again.
///
/// **Batched per BFS depth.** For each level we issue at most four SQLite
/// round-trips (one each for summaries / trees / chunks / chunk
/// embeddings) covering every node at that depth, then walk the level's
/// id slice in BFS order to populate `out` + collect the next-depth
/// frontier. The per-node `get_summary` / `get_tree` / `get_chunk` /
/// `get_chunk_embedding` loop (one round-trip per node × 4 tables) is
/// replaced by `O(depth)` round-trips instead of `O(nodes × 4)`. File
/// I/O via `read_summary_body` / `read_chunk_body` stays per-id — each
/// body lives in its own on-disk file, so batching would mean concurrent
/// opens, not a single round-trip; left untouched.
fn walk_with_embeddings(
    config: &Config,
    start_id: &str,
    max_depth: u32,
) -> Result<(Vec<RetrievalHit>, Vec<Option<Vec<f32>>>)> {
    // Fetch the root. If it's a summary we expand its child_ids; if it's a
    // chunk it has no children. If it's neither we return empty.
    let root_summary = store::get_summary(config, start_id)?;
    let root_tree_scope = match root_summary.as_ref().map(|s| s.tree_id.clone()) {
        Some(tid) => store::get_tree(config, &tid)?
            .map(|t| t.scope)
            .unwrap_or_default(),
        None => String::new(),
    };

    let mut out: Vec<RetrievalHit> = Vec::new();
    let mut embeddings: Vec<Option<Vec<f32>>> = Vec::new();

    let start_children: Vec<String> = match root_summary {
        Some(s) => s.child_ids.clone(),
        None => {
            if let Some(_c) = get_chunk(config, start_id)? {
                return Ok((out, embeddings));
            }
            log::debug!(
                "[retrieval::drill_down] node_id={start_id} not found in summaries or chunks"
            );
            return Ok((out, embeddings));
        }
    };

    // BFS-by-level. `current_level` holds every node id at the current
    // depth, walked in FIFO (BFS) order — siblings are always returned
    // before any descendant at a deeper depth (regression for PR #831's
    // CodeRabbit fix away from `Vec::pop` DFS). Batched fetches for the
    // whole level happen up-front so the per-id walk only does HashMap
    // lookups + the unavoidable per-file body read.
    let mut current_level: Vec<String> = start_children;
    let mut depth: u32 = 1;

    // Latest-version-per-document filter (document source trees, e.g. Notion).
    // A document's chunks roll up to a per-doc subtree whose root carries
    // `(doc_id, version_ms)`; editing a page seals a NEW doc-root (higher
    // `version_ms`) alongside the old one, so the merge tier reaches both. We
    // surface only the newest revision: as the walk encounters doc-roots we
    // track `max(version_ms)` per `doc_id` and skip any doc-root below that
    // max (and therefore its whole stale subtree). Nothing is mutated on disk
    // — superseded revisions simply never appear in results. Non-document
    // nodes (doc_id == None) are unaffected.
    let mut max_version_by_doc: HashMap<String, i64> = HashMap::new();
    // Doc-roots already surfaced, to dedup at the winning version: if a
    // `SealDocument` job partially committed then retried, it can mint a second
    // doc-root for the SAME `(doc_id, version_ms)`. Emit only the first one per
    // doc_id so a duplicate revision never double-surfaces.
    let mut emitted_docs: HashSet<String> = HashSet::new();

    while !current_level.is_empty() && depth <= max_depth {
        log::trace!(
            "[retrieval::drill_down] level depth={} ids={}",
            depth,
            current_level.len()
        );

        // 1) Batched summary fetch covers every id on this level. Missing
        //    ids stay silently absent from the map (same `Ok(None)`
        //    contract as the per-row `get_summary`); those ids are then
        //    tried as chunks below.
        let mut summary_by_id = get_summaries_batch(config, &current_level)?;

        // Update the per-document latest-version map with any doc-roots on
        // THIS level before walking it, so two revisions of the same document
        // sitting side-by-side (the common case — both are merge-tier leaves
        // at the same depth) resolve to the newer one regardless of walk
        // order. A doc-root is a summary with `doc_id` set; `version_ms`
        // defaults to i64::MIN so a (legacy) untagged doc-root never wins over
        // a tagged one.
        for id in &current_level {
            if let Some(s) = summary_by_id.get(id) {
                if let Some(doc_id) = s.doc_id.as_deref() {
                    let v = s.version_ms.unwrap_or(i64::MIN);
                    max_version_by_doc
                        .entry(doc_id.to_string())
                        .and_modify(|cur| {
                            if v > *cur {
                                *cur = v;
                            }
                        })
                        .or_insert(v);
                }
            }
        }

        // 2) Distinct tree_ids referenced by this level's summaries —
        //    dedup is purely to avoid redundant DB params (the per-id
        //    walk below routes each summary to its own scope via the
        //    map). Insertion-order preserving for deterministic logs.
        let distinct_tree_ids: Vec<String> = {
            // `seen` borrows the tree_id slices straight out of
            // `summary_by_id` (which outlives this block) — dedup costs no
            // allocation; only the surviving distinct ids are cloned into
            // `out` for `get_trees_batch`.
            let mut seen: HashSet<&str> = HashSet::new();
            let mut out: Vec<String> = Vec::new();
            for id in &current_level {
                if let Some(s) = summary_by_id.get(id) {
                    if seen.insert(s.tree_id.as_str()) {
                        out.push(s.tree_id.clone());
                    }
                }
            }
            out
        };
        let tree_by_id = get_trees_batch(config, &distinct_tree_ids)?;

        // 3) Ids on this level that AREN'T summaries are candidate
        //    chunk leaves; batch-fetch both the chunk rows and their
        //    embeddings. Missing ids are silently absent — the warn
        //    path at the end of the per-id walk catches "points at
        //    nothing" cases (preserving the existing contract).
        let chunk_ids: Vec<String> = current_level
            .iter()
            .filter(|id| !summary_by_id.contains_key(*id))
            .cloned()
            .collect();
        let mut chunk_by_id = get_chunks_batch(config, &chunk_ids)?;
        // `get_chunk_embeddings_batch` returns only present ids
        // (mirroring per-row `get_chunk_embedding` returning
        // `Ok(None)` for legacy rows without an embedding row);
        // `.get(id).cloned()` yields the equivalent `Option<Vec<f32>>`.
        let emb_by_id = get_chunk_embeddings_batch(config, &chunk_ids)?;

        // 4) Walk this level in BFS order, populate hits, collect next
        //    level. Per-id HashMap lookups (keyed by id, not by
        //    enumerate() position over the input slice — otherwise a
        //    sibling could shadow another's scope or chunk body).
        // Pre-size against expected fan-out so the per-level child
        // accumulation avoids repeated reallocs. Only the non-final depths
        // extend `next_level` (see the `depth < max_depth` guard below), so
        // skip the reservation at the last depth where it would stay empty.
        let mut next_level: Vec<String> = if depth < max_depth {
            Vec::with_capacity(current_level.len() * EXPECTED_CHILD_FANOUT)
        } else {
            Vec::new()
        };
        for id in &current_level {
            if let Some(mut summary) = summary_by_id.remove(id) {
                // Latest-wins: skip a doc-root that a newer revision of the
                // same document supersedes. Its subtree is not expanded, so
                // the stale revision's chunks never surface.
                if let Some(doc_id) = summary.doc_id.as_deref() {
                    let v = summary.version_ms.unwrap_or(i64::MIN);
                    if max_version_by_doc.get(doc_id).is_some_and(|&max| v < max) {
                        log::debug!(
                            "[retrieval::drill_down] skipping superseded doc-root \
                             doc_id={doc_id} version_ms={v} (latest is newer)"
                        );
                        continue;
                    }
                    // Dedup duplicates at the winning version (e.g. a retried
                    // SealDocument that minted a second doc-root for the same
                    // (doc_id, version_ms)) — surface only the first.
                    if !emitted_docs.insert(doc_id.to_string()) {
                        log::debug!(
                            "[retrieval::drill_down] skipping duplicate doc-root \
                             doc_id={doc_id} version_ms={v} (already surfaced)"
                        );
                        continue;
                    }
                }
                let scope = tree_by_id
                    .get(&summary.tree_id)
                    .map(|t| t.scope.clone())
                    .unwrap_or_else(|| root_tree_scope.clone());
                // Hydrate the full body from disk — `summary.content` is
                // a ≤500-char preview after the MD-on-disk migration.
                // Non-fatal fallback for pre-MD-migration rows.
                match content_read::read_summary_body(config, id) {
                    Ok(body) => summary.content = body,
                    Err(e) => {
                        log::warn!(
                            "[retrieval::drill_down] read_summary_body failed — serving preview: {e:#}"
                        );
                    }
                }
                // Summary embeddings live on the struct directly
                // (Phase 4 amend).
                embeddings.push(summary.embedding.clone());
                let child_ids = summary.child_ids.clone();
                out.push(hit_from_summary(&summary, &scope));
                if depth < max_depth {
                    next_level.extend(child_ids);
                }
                continue;
            }
            if let Some(mut chunk) = chunk_by_id.remove(id) {
                // Missing embedding → None (legacy row); identical to
                // the per-row `get_chunk_embedding(...) Ok(None)` arm.
                let emb = emb_by_id.get(id).cloned();
                embeddings.push(emb);
                // Hydrate the full body from disk — `chunk.content` is
                // a ≤500-char preview after the MD-on-disk migration.
                match content_read::read_chunk_body(config, id) {
                    Ok(body) => chunk.content = body,
                    Err(e) => {
                        log::warn!(
                            "[retrieval::drill_down] read_chunk_body failed — serving preview: {e:#}"
                        );
                    }
                }
                // Score unknown here; 0.0 neutral placeholder.
                out.push(hit_from_chunk(&chunk, "", &chunk.metadata.source_id, 0.0));
                continue;
            }
            // Redact the child id — may contain source scope (e.g.
            // `chat:slack:#<channel>:seq`). Log the kind prefix only.
            let kind_prefix = id.split_once(':').map(|(k, _)| k).unwrap_or("unknown");
            log::warn!(
                "[retrieval::drill_down] child kind={kind_prefix} points at nothing — skipping"
            );
        }

        current_level = next_level;
        depth += 1;
    }
    Ok((out, embeddings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::chat::{test_override, ChatProvider, StaticChatProvider};
    use crate::openhuman::memory::tree_source::registry::get_or_create_source_tree;
    use crate::openhuman::memory_store::chunks::store::upsert_chunks;
    use crate::openhuman::memory_store::chunks::types::{
        chunk_id, Chunk, Metadata, SourceKind, SourceRef,
    };
    use crate::openhuman::memory_store::content as content_store;
    use crate::openhuman::memory_store::trees::types::TreeKind;
    use crate::openhuman::memory_tree::tree::bucket_seal::{append_leaf, LabelStrategy, LeafRef};
    use chrono::Utc;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        // Phase 4 (#710): seeding requires seals which embed.
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    async fn seed_sealed_tree(cfg: &Config) -> (String, String) {
        // Seed two 6k-token leaves so the L0 buffer seals into an L1 node.
        let ts = Utc::now();
        let tree = get_or_create_source_tree(cfg, "slack:#eng").unwrap();
        let provider: Arc<dyn ChatProvider> =
            Arc::new(StaticChatProvider::new("test summary content"));
        let content_root = cfg.memory_tree_content_root();
        std::fs::create_dir_all(&content_root).unwrap();
        let mut leaf_ids: Vec<String> = Vec::new();
        for seq in 0..2u32 {
            let c = Chunk {
                id: chunk_id(SourceKind::Chat, "slack:#eng", seq, "test-content"),
                content: format!("content-{seq}"),
                metadata: Metadata {
                    source_kind: SourceKind::Chat,
                    source_id: "slack:#eng".into(),
                    owner: "alice".into(),
                    timestamp: ts,
                    time_range: (ts, ts),
                    tags: vec![],
                    source_ref: Some(SourceRef::new("slack://x")),
                    path_scope: None,
                },
                token_count: crate::openhuman::memory_store::trees::types::INPUT_TOKEN_BUDGET * 6
                    / 10,
                seq_in_source: seq,
                created_at: ts,
                partial_message: false,
            };
            upsert_chunks(cfg, &[c.clone()]).unwrap();
            // Stage to disk so `hydrate_leaf_inputs` can read the full body
            // via `read_chunk_body` during the seal triggered by `append_leaf`.
            let staged = content_store::stage_chunks(&content_root, &[c.clone()]).unwrap();
            crate::openhuman::memory_store::chunks::store::with_connection(cfg, |conn| {
                let tx = conn.unchecked_transaction()?;
                crate::openhuman::memory_store::chunks::store::upsert_staged_chunks_tx(
                    &tx, &staged,
                )?;
                tx.commit()?;
                Ok(())
            })
            .unwrap();
            leaf_ids.push(c.id.clone());
            let leaf = LeafRef {
                chunk_id: c.id.clone(),
                token_count: crate::openhuman::memory_store::trees::types::INPUT_TOKEN_BUDGET * 6
                    / 10,
                timestamp: ts,
                content: c.content.clone(),
                entities: vec![],
                topics: vec![],
                score: 0.5,
            };
            test_override::with_provider(Arc::clone(&provider), async {
                append_leaf(cfg, &tree, &leaf, &LabelStrategy::Empty)
                    .await
                    .unwrap()
            })
            .await;
        }
        // Fetch the sealed L1 summary id from the tree row.
        let refreshed = store::get_tree(cfg, &tree.id).unwrap().unwrap();
        assert_eq!(refreshed.kind, TreeKind::Source);
        let root_id = refreshed.root_id.unwrap();
        (root_id, leaf_ids.remove(0))
    }

    #[tokio::test]
    async fn depth_zero_returns_empty() {
        let (_tmp, cfg) = test_config();
        let (root_id, _) = seed_sealed_tree(&cfg).await;
        let out = drill_down(&cfg, &root_id, 0, None, None).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn invalid_id_returns_empty() {
        let (_tmp, cfg) = test_config();
        let out = drill_down(&cfg, "nonexistent:id", 1, None, None)
            .await
            .unwrap();
        assert!(out.is_empty());
    }

    /// Read-time latest-wins: a merge root referencing two per-doc roots of
    /// the SAME document (v1 < v2) must surface only the newer revision's
    /// subtree; the superseded doc-root and its chunk are filtered out and
    /// never traversed — without anything being deleted on disk.
    #[tokio::test]
    async fn drill_down_surfaces_only_latest_doc_version() {
        use crate::openhuman::memory_store::chunks::store::{upsert_chunks, with_connection};
        use crate::openhuman::memory_store::trees::types::{SummaryNode, Tree, TreeStatus};
        use crate::openhuman::memory_tree::tree::store as tree_store;

        let (_tmp, cfg) = test_config();
        let ts = Utc::now();

        let tree = Tree {
            id: "test:notion-tree".into(),
            kind: TreeKind::Source,
            scope: "notion:conn1".into(),
            root_id: Some("s:merge:root".into()),
            max_level: 1000,
            status: TreeStatus::Active,
            created_at: ts,
            last_sealed_at: Some(ts),
        };

        let mk_chunk = |content: &str| Chunk {
            id: chunk_id(SourceKind::Document, "notion:conn1:pageA", 0, content),
            content: content.to_string(),
            metadata: Metadata {
                source_kind: SourceKind::Document,
                source_id: "notion:conn1:pageA".into(),
                owner: "notion:conn1".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec!["notion".into()],
                source_ref: Some(SourceRef::new("notion://page/pageA")),
                path_scope: Some("notion:conn1".into()),
            },
            token_count: 10,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        };
        // Distinct content → distinct chunk ids (content is hashed in).
        let chunk_v1 = mk_chunk("old version body");
        let chunk_v2 = mk_chunk("new version body");
        upsert_chunks(&cfg, &[chunk_v1.clone(), chunk_v2.clone()]).unwrap();

        let mk_root = |id: &str, version: i64, child: &str| SummaryNode {
            id: id.into(),
            tree_id: tree.id.clone(),
            tree_kind: TreeKind::Source,
            level: 1,
            parent_id: Some("s:merge:root".into()),
            child_ids: vec![child.to_string()],
            content: format!("doc-root v{version}"),
            token_count: 5,
            entities: vec![],
            topics: vec![],
            time_range_start: ts,
            time_range_end: ts,
            score: 0.5,
            sealed_at: ts,
            deleted: false,
            embedding: None,
            doc_id: Some("notion:conn1:pageA".into()),
            version_ms: Some(version),
        };
        let v1_root = mk_root("s:docA:v1", 100, &chunk_v1.id);
        let v2_root = mk_root("s:docA:v2", 200, &chunk_v2.id);

        let merge_root = SummaryNode {
            id: "s:merge:root".into(),
            tree_id: tree.id.clone(),
            tree_kind: TreeKind::Source,
            level: 1000,
            parent_id: None,
            child_ids: vec![v1_root.id.clone(), v2_root.id.clone()],
            content: "merge root".into(),
            token_count: 5,
            entities: vec![],
            topics: vec![],
            time_range_start: ts,
            time_range_end: ts,
            score: 0.5,
            sealed_at: ts,
            deleted: false,
            embedding: None,
            doc_id: None,
            version_ms: None,
        };

        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            tree_store::insert_tree_conn(&tx, &tree)?;
            tree_store::insert_summary_tx(&tx, &v1_root, None, "test")?;
            tree_store::insert_summary_tx(&tx, &v2_root, None, "test")?;
            tree_store::insert_summary_tx(&tx, &merge_root, None, "test")?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();

        let out = drill_down(&cfg, "s:merge:root", 3, None, None)
            .await
            .unwrap();
        let ids: Vec<&str> = out.iter().map(|h| h.node_id.as_str()).collect();

        assert!(
            ids.contains(&"s:docA:v2"),
            "latest doc-root must surface; got {ids:?}"
        );
        assert!(
            ids.contains(&chunk_v2.id.as_str()),
            "latest version's chunk must surface; got {ids:?}"
        );
        assert!(
            !ids.contains(&"s:docA:v1"),
            "superseded doc-root must be filtered; got {ids:?}"
        );
        assert!(
            !ids.contains(&chunk_v1.id.as_str()),
            "superseded version's chunk must not surface; got {ids:?}"
        );
    }

    #[tokio::test]
    async fn summary_drills_to_leaves_at_depth_one() {
        let (_tmp, cfg) = test_config();
        let (root_id, _) = seed_sealed_tree(&cfg).await;
        let out = drill_down(&cfg, &root_id, 1, None, None).await.unwrap();
        assert_eq!(out.len(), 2, "L1 has 2 leaf children");
        for hit in &out {
            assert_eq!(hit.level, 0, "direct children of L1 are leaves");
        }
    }

    #[tokio::test]
    async fn leaf_drill_down_returns_empty() {
        let (_tmp, cfg) = test_config();
        let (_root_id, leaf_id) = seed_sealed_tree(&cfg).await;
        let out = drill_down(&cfg, &leaf_id, 3, None, None).await.unwrap();
        assert!(out.is_empty(), "leaves have no children");
    }

    #[tokio::test]
    async fn deeper_max_depth_does_not_break_on_shallow_tree() {
        // Only one summary level exists; asking for max_depth=5 is fine.
        let (_tmp, cfg) = test_config();
        let (root_id, _) = seed_sealed_tree(&cfg).await;
        let out = drill_down(&cfg, &root_id, 5, None, None).await.unwrap();
        assert_eq!(out.len(), 2);
    }

    #[tokio::test]
    async fn query_with_limit_truncates_after_rerank() {
        // Verifies the plumbing for the query param: embedder is invoked
        // (InertEmbedder under this test config — all-zero vectors so
        // cosine is 0 for every candidate), limit truncates the output,
        // and the function completes without error.
        let (_tmp, cfg) = test_config();
        let (root_id, _) = seed_sealed_tree(&cfg).await;
        let out = drill_down(&cfg, &root_id, 1, Some("phoenix migration timing"), Some(1))
            .await
            .unwrap();
        assert_eq!(out.len(), 1, "limit=1 truncates 2 children to 1");
    }

    #[tokio::test]
    async fn query_without_limit_returns_all_children() {
        let (_tmp, cfg) = test_config();
        let (root_id, _) = seed_sealed_tree(&cfg).await;
        let out = drill_down(&cfg, &root_id, 1, Some("phoenix"), None)
            .await
            .unwrap();
        assert_eq!(out.len(), 2, "no limit — both children returned");
    }

    // ── Regression: BFS (not DFS) traversal ──────────────────────────
    //
    // `walk_with_embeddings` walks level-by-level (all nodes at depth N
    // before any at depth N+1) — originally flagged on PR #831 CodeRabbit
    // review after the initial `Vec::pop()` implementation was DFS.
    //
    // A single-level tree can't distinguish the two (both produce the same
    // output). We need a 2-level tree where BFS yields
    //   [L1_A, L1_B, c_A_1, c_A_2, c_B_1, c_B_2]
    // and DFS would yield
    //   [L1_B, c_B_2, c_B_1, L1_A, c_A_2, c_A_1]
    // (or similar — the key invariant is that BFS returns all siblings at
    // one depth before any descendant at a deeper depth).

    use crate::openhuman::memory_store::chunks::store::with_connection;
    use crate::openhuman::memory_store::trees::types::{SummaryNode, Tree, TreeStatus};
    use crate::openhuman::memory_tree::tree::store as tree_store;

    /// Build a tiny 2-level tree directly via store inserts so we can
    /// assert BFS ordering without needing ~100 leaves to cascade L1→L2
    /// through the token-budget seal path.
    async fn seed_two_level_tree(cfg: &Config) -> (String, Vec<String>, Vec<String>) {
        let ts = Utc::now();
        let tree = Tree {
            id: "test:two-level".into(),
            kind: TreeKind::Source,
            scope: "slack:#eng".into(),
            root_id: Some("s:L2:root".into()),
            max_level: 2,
            status: TreeStatus::Active,
            created_at: ts,
            last_sealed_at: Some(ts),
        };
        let leaf_a_1 = Chunk {
            id: "chat:slack:#eng:0".into(),
            content: "leaf-a-1".into(),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: "slack:#eng".into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: Some(SourceRef::new("slack://x")),
                path_scope: None,
            },
            token_count: 10,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        };
        let leaf_a_2 = Chunk {
            id: "chat:slack:#eng:1".into(),
            content: "leaf-a-2".into(),
            metadata: leaf_a_1.metadata.clone(),
            seq_in_source: 1,
            ..leaf_a_1.clone()
        };
        let leaf_b_1 = Chunk {
            id: "chat:slack:#eng:2".into(),
            content: "leaf-b-1".into(),
            metadata: leaf_a_1.metadata.clone(),
            seq_in_source: 2,
            ..leaf_a_1.clone()
        };
        let leaf_b_2 = Chunk {
            id: "chat:slack:#eng:3".into(),
            content: "leaf-b-2".into(),
            metadata: leaf_a_1.metadata.clone(),
            seq_in_source: 3,
            ..leaf_a_1.clone()
        };
        let all_leaves = [
            leaf_a_1.clone(),
            leaf_a_2.clone(),
            leaf_b_1.clone(),
            leaf_b_2.clone(),
        ];
        upsert_chunks(cfg, &all_leaves).unwrap();
        // Stage to disk so `walk_with_embeddings` can read full bodies via
        // `read_chunk_body` for leaf hits returned by the drill-down.
        let content_root = cfg.memory_tree_content_root();
        std::fs::create_dir_all(&content_root).unwrap();
        let staged = content_store::stage_chunks(&content_root, &all_leaves).unwrap();
        crate::openhuman::memory_store::chunks::store::with_connection(cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            crate::openhuman::memory_store::chunks::store::upsert_staged_chunks_tx(&tx, &staged)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();

        let l1_a = SummaryNode {
            id: "s:L1:a".into(),
            tree_id: tree.id.clone(),
            tree_kind: TreeKind::Source,
            level: 1,
            parent_id: Some("s:L2:root".into()),
            child_ids: vec![leaf_a_1.id.clone(), leaf_a_2.id.clone()],
            content: "L1 summary A".into(),
            token_count: 50,
            entities: vec![],
            topics: vec![],
            time_range_start: ts,
            time_range_end: ts,
            score: 0.5,
            sealed_at: ts,
            deleted: false,
            embedding: None,
            doc_id: None,
            version_ms: None,
        };
        let l1_b = SummaryNode {
            id: "s:L1:b".into(),
            child_ids: vec![leaf_b_1.id.clone(), leaf_b_2.id.clone()],
            ..l1_a.clone()
        };
        let root = SummaryNode {
            id: "s:L2:root".into(),
            level: 2,
            parent_id: None,
            child_ids: vec![l1_a.id.clone(), l1_b.id.clone()],
            content: "L2 root".into(),
            ..l1_a.clone()
        };

        // Open the shared connection to the memory_tree DB and write the
        // tree + three summaries in one transaction.
        with_connection(cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            tree_store::insert_tree_conn(&tx, &tree)?;
            tree_store::insert_summary_tx(&tx, &l1_a, None, "test")?;
            tree_store::insert_summary_tx(&tx, &l1_b, None, "test")?;
            tree_store::insert_summary_tx(&tx, &root, None, "test")?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();

        (
            root.id,
            vec![l1_a.id, l1_b.id],
            vec![leaf_a_1.id, leaf_a_2.id, leaf_b_1.id, leaf_b_2.id],
        )
    }

    #[tokio::test]
    async fn walk_visits_siblings_before_descendants_bfs_order() {
        let (_tmp, cfg) = test_config();
        let (root_id, l1_ids, leaf_ids) = seed_two_level_tree(&cfg).await;

        let out = drill_down(&cfg, &root_id, 2, None, None).await.unwrap();
        // Both L1s + all 4 leaves = 6 hits.
        assert_eq!(out.len(), 6, "L2 with 2×L1 × 2 leaves each = 6 hits");

        // Collect ids in returned order.
        let ordered: Vec<&str> = out.iter().map(|h| h.node_id.as_str()).collect();

        // BFS invariant: every L1 index must come BEFORE every leaf index.
        // (DFS would interleave a whole L1 subtree before the other L1.)
        let last_l1 = l1_ids
            .iter()
            .map(|id| ordered.iter().position(|&n| n == id).unwrap())
            .max()
            .unwrap();
        let first_leaf = leaf_ids
            .iter()
            .map(|id| ordered.iter().position(|&n| n == id).unwrap())
            .min()
            .unwrap();
        assert!(
            last_l1 < first_leaf,
            "BFS must return both L1 summaries before any leaf; got ordered={ordered:?}"
        );
    }

    // ── Regression: per-depth batching keys HashMap by id, not position ──
    //
    // The per-depth batch refactor replaces the `for id in frontier` loop
    // (one `get_summary` + one `get_tree` + (chunk: + one `get_chunk` +
    // one `get_chunk_embedding`) per node) with one batched fetch per
    // table, per level. The per-id walk that follows MUST look up by id
    // — a refactor that mistakenly switches to `enumerate()` position
    // over the input slice would silently shadow sibling A's `Tree`
    // (and therefore `scope`) with sibling B's. This test seeds two L1
    // summaries belonging to **distinct trees** with **distinct scopes**
    // and asserts each summary's emitted hit carries its own tree scope
    // — proving the HashMap-keyed-by-id contract.
    async fn seed_two_l1s_in_distinct_trees(cfg: &Config) -> (String, String, String) {
        let ts = Utc::now();
        // Root sits in tree_a; its child L1s live in DIFFERENT trees so
        // the per-depth `get_trees_batch` actually has 2 distinct ids to
        // resolve and the HashMap lookup is non-trivial.
        let tree_a = Tree {
            id: "test:tree-a".into(),
            kind: TreeKind::Source,
            scope: "slack:#eng".into(),
            root_id: Some("s:L2:root-a".into()),
            max_level: 2,
            status: TreeStatus::Active,
            created_at: ts,
            last_sealed_at: Some(ts),
        };
        let tree_b = Tree {
            id: "test:tree-b".into(),
            scope: "slack:#design".into(),
            root_id: None,
            ..tree_a.clone()
        };

        let l1_a = SummaryNode {
            id: "s:L1:a".into(),
            tree_id: tree_a.id.clone(),
            tree_kind: TreeKind::Source,
            level: 1,
            parent_id: Some("s:L2:root-a".into()),
            child_ids: vec![],
            content: "L1 from tree-a".into(),
            token_count: 50,
            entities: vec![],
            topics: vec![],
            time_range_start: ts,
            time_range_end: ts,
            score: 0.5,
            sealed_at: ts,
            deleted: false,
            embedding: None,
            doc_id: None,
            version_ms: None,
        };
        let l1_b = SummaryNode {
            id: "s:L1:b".into(),
            tree_id: tree_b.id.clone(),
            content: "L1 from tree-b".into(),
            ..l1_a.clone()
        };
        let root_a = SummaryNode {
            id: "s:L2:root-a".into(),
            level: 2,
            parent_id: None,
            child_ids: vec![l1_a.id.clone(), l1_b.id.clone()],
            content: "L2 root in tree-a referencing L1s from two trees".into(),
            ..l1_a.clone()
        };

        with_connection(cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            tree_store::insert_tree_conn(&tx, &tree_a)?;
            tree_store::insert_tree_conn(&tx, &tree_b)?;
            tree_store::insert_summary_tx(&tx, &l1_a, None, "test")?;
            tree_store::insert_summary_tx(&tx, &l1_b, None, "test")?;
            tree_store::insert_summary_tx(&tx, &root_a, None, "test")?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();

        (root_a.id, l1_a.id, l1_b.id)
    }

    #[tokio::test]
    async fn per_depth_batch_keys_hit_scope_by_tree_id_not_position() {
        let (_tmp, cfg) = test_config();
        let (root_id, l1_a_id, l1_b_id) = seed_two_l1s_in_distinct_trees(&cfg).await;

        let out = drill_down(&cfg, &root_id, 1, None, None).await.unwrap();
        assert_eq!(out.len(), 2, "two L1 children expected");

        // Find each hit by id and assert its scope came from its own
        // tree row — not the other sibling's tree (which would happen
        // if the per-id lookup used `enumerate()` position).
        let hit_a = out
            .iter()
            .find(|h| h.node_id == l1_a_id)
            .expect("hit for L1 in tree-a missing");
        let hit_b = out
            .iter()
            .find(|h| h.node_id == l1_b_id)
            .expect("hit for L1 in tree-b missing");
        assert_eq!(
            hit_a.tree_scope, "slack:#eng",
            "L1 from tree-a must carry tree-a's scope"
        );
        assert_eq!(
            hit_b.tree_scope, "slack:#design",
            "L1 from tree-b must carry tree-b's scope (NOT tree-a's)"
        );
        assert_eq!(hit_a.tree_id, "test:tree-a");
        assert_eq!(hit_b.tree_id, "test:tree-b");
    }
}
