//! Read RPCs that back the new Memory tab UI.
//!
//! Distinct from [`super::rpc`] (write/ingest) and [`super::retrieval::rpc`]
//! (LLM-callable retrieval primitives), this module exposes a small set of
//! "list / inspect / search / recall / score-for / delete" methods designed
//! for a human-facing dashboard — not for an LLM tool loop.
//!
//! All methods are scoped under the existing `memory_tree` JSON-RPC
//! namespace so they share authentication, telemetry, and discovery with
//! the other memory-tree RPCs.
//!
//! Coverage:
//! - `memory_tree_list_chunks`         — paginated chunk listing with filters
//! - `memory_tree_list_sources`        — distinct sources + chunk counts
//! - `memory_tree_search`              — keyword search returning chunks
//! - `memory_tree_recall`              — semantic recall (via Phase 4 rerank)
//! - `memory_tree_entity_index_for`    — entities attached to one chunk
//! - `memory_tree_top_entities`        — most-frequent canonical entities
//! - `memory_tree_chunk_score`         — score breakdown for one chunk
//! - `memory_tree_delete_chunk`        — purge one chunk + dependent rows
//!
//! The `Source.display_name` un-slugs the SQL `source_id` so a UI can show
//! a human-friendly label (e.g. `gmail:enamakel@..|sanil@..` →
//! `Enamakel ↔ Sanil`). When the workspace has surfaced the user's primary
//! email via app_state, we also strip it from the display so the user sees
//! the *other* party.

use anyhow::{Context, Result};
use rusqlite::params;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::content_store::read as content_read;
use crate::openhuman::memory::tree::retrieval::types::NodeKind;
use crate::openhuman::memory::tree::score::store as score_store;
use crate::openhuman::memory::tree::store::{self as chunk_store, with_connection};
use crate::openhuman::memory::tree::types::SourceKind;
use crate::rpc::RpcOutcome;

const PREVIEW_MAX_CHARS: usize = 500;
const DEFAULT_LIST_LIMIT: u32 = 50;
const MAX_LIST_LIMIT: u32 = 1_000;

// ── Wire types ───────────────────────────────────────────────────────────

/// Wire-shape chunk returned by the read RPCs.
///
/// Distinct from [`crate::openhuman::memory::tree::types::Chunk`] in two
/// ways: serialised timestamps are ms-since-epoch (matches the rest of the
/// JSON-RPC surface) and the body is replaced with a `≤500-char preview`
/// + a flag indicating whether the row has an embedding. UIs needing the
/// full body call back via `memory_tree_get_chunk`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChunkRow {
    pub id: String,
    pub source_kind: String,
    pub source_id: String,
    #[serde(default)]
    pub source_ref: Option<String>,
    pub owner: String,
    pub timestamp_ms: i64,
    pub token_count: u32,
    pub lifecycle_status: String,
    #[serde(default)]
    pub content_path: Option<String>,
    #[serde(default)]
    pub content_preview: Option<String>,
    pub has_embedding: bool,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Filter shape for [`list_chunks`]. All fields are optional.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChunkFilter {
    #[serde(default)]
    pub source_kinds: Option<Vec<String>>,
    #[serde(default)]
    pub source_ids: Option<Vec<String>>,
    #[serde(default)]
    pub entity_ids: Option<Vec<String>>,
    #[serde(default)]
    pub since_ms: Option<i64>,
    #[serde(default)]
    pub until_ms: Option<i64>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
}

/// Response shape for [`list_chunks`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListChunksResponse {
    pub chunks: Vec<ChunkRow>,
    pub total: u64,
}

/// Distinct ingest source plus chunk counts. Returned by [`list_sources`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub source_id: String,
    /// Computed display name (un-slug + strip user email when known).
    pub display_name: String,
    pub source_kind: String,
    pub chunk_count: u32,
    pub most_recent_ms: i64,
}

/// Lightweight reference to a canonical entity.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntityRef {
    /// Canonical id (e.g. `email:alice@example.com`, `topic:phoenix`).
    pub entity_id: String,
    pub kind: String,
    pub surface: String,
    pub count: u32,
}

/// Per-signal weight + raw value pair.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScoreSignal {
    pub name: String,
    pub weight: f32,
    pub value: f32,
}

/// Score rationale returned by [`chunk_score`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    pub signals: Vec<ScoreSignal>,
    pub total: f32,
    pub threshold: f32,
    pub kept: bool,
    pub llm_consulted: bool,
}

// ── list_chunks ──────────────────────────────────────────────────────────

/// `memory_tree_list_chunks` — paginated chunk listing with filters.
pub async fn list_chunks_rpc(
    config: &Config,
    filter: ChunkFilter,
) -> Result<RpcOutcome<ListChunksResponse>, String> {
    let cfg = config.clone();
    let resp = tokio::task::spawn_blocking(move || -> Result<ListChunksResponse> {
        list_chunks_blocking(&cfg, &filter)
    })
    .await
    .map_err(|e| format!("list_chunks join error: {e}"))?
    .map_err(|e| format!("list_chunks: {e:#}"))?;

    let n = resp.chunks.len();
    let total = resp.total;
    Ok(RpcOutcome::single_log(
        resp,
        format!("memory_tree::read: list_chunks n={n} total={total}"),
    ))
}

fn list_chunks_blocking(config: &Config, filter: &ChunkFilter) -> Result<ListChunksResponse> {
    let limit = filter
        .limit
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .clamp(1, MAX_LIST_LIMIT);
    let offset = filter.offset.unwrap_or(0);

    with_connection(config, |conn| {
        // Build SQL with bound parameters. `entity_ids` requires an inner
        // join via `mem_tree_entity_index`; the rest stay on `mem_tree_chunks`.
        let mut sql = String::from(
            "SELECT DISTINCT
                c.id, c.source_kind, c.source_id, c.source_ref, c.owner,
                c.timestamp_ms, c.token_count, c.lifecycle_status,
                c.content_path, c.content, c.tags_json,
                CASE WHEN c.embedding IS NULL THEN 0 ELSE 1 END AS has_embedding
             FROM mem_tree_chunks c",
        );
        let mut where_clauses: Vec<String> = vec![];
        let mut params_owned: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(eids) = &filter.entity_ids {
            if !eids.is_empty() {
                sql.push_str(" INNER JOIN mem_tree_entity_index ei ON ei.node_id = c.id");
                let placeholders: Vec<String> = (0..eids.len()).map(|_| "?".to_string()).collect();
                where_clauses.push(format!("ei.entity_id IN ({})", placeholders.join(", ")));
                for eid in eids {
                    params_owned.push(Box::new(eid.clone()));
                }
            }
        }
        if let Some(kinds) = &filter.source_kinds {
            if !kinds.is_empty() {
                let placeholders: Vec<String> = (0..kinds.len()).map(|_| "?".to_string()).collect();
                where_clauses.push(format!("c.source_kind IN ({})", placeholders.join(", ")));
                for k in kinds {
                    params_owned.push(Box::new(k.clone()));
                }
            }
        }
        if let Some(sids) = &filter.source_ids {
            if !sids.is_empty() {
                let placeholders: Vec<String> = (0..sids.len()).map(|_| "?".to_string()).collect();
                where_clauses.push(format!("c.source_id IN ({})", placeholders.join(", ")));
                for s in sids {
                    params_owned.push(Box::new(s.clone()));
                }
            }
        }
        if let Some(since) = filter.since_ms {
            where_clauses.push("c.timestamp_ms >= ?".into());
            params_owned.push(Box::new(since));
        }
        if let Some(until) = filter.until_ms {
            where_clauses.push("c.timestamp_ms <= ?".into());
            params_owned.push(Box::new(until));
        }
        if let Some(query) = &filter.query {
            let q = query.trim();
            if !q.is_empty() {
                // NOTE: `c.content` is the ≤500-char preview kept in
                // SQLite, not the canonical body — that lives on disk
                // at `c.content_path`. This means search currently
                // misses any chunk whose match is past the first 500
                // chars. Acceptable for v1 (most matches land in the
                // first paragraph anyway); a follow-up should swap to
                // a full-text index over the on-disk body.
                where_clauses.push("c.content LIKE ?".into());
                params_owned.push(Box::new(format!("%{}%", q)));
            }
        }

        if !where_clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_clauses.join(" AND "));
        }
        // total count for pagination — do it before applying limit/offset.
        let count_sql = format!(
            "SELECT COUNT(*) FROM ({}) AS sub",
            sql.replacen(
                "SELECT DISTINCT\n                c.id, c.source_kind, c.source_id, c.source_ref, c.owner,\n                c.timestamp_ms, c.token_count, c.lifecycle_status,\n                c.content_path, c.content, c.tags_json,\n                CASE WHEN c.embedding IS NULL THEN 0 ELSE 1 END AS has_embedding",
                "SELECT DISTINCT c.id",
                1
            )
        );

        sql.push_str(" ORDER BY c.timestamp_ms DESC, c.seq_in_source ASC LIMIT ? OFFSET ?");
        params_owned.push(Box::new(limit as i64));
        params_owned.push(Box::new(offset as i64));

        // Execute count query — use the WHERE-bound params (without LIMIT/OFFSET).
        let count_params: Vec<&dyn rusqlite::ToSql> = params_owned
            .iter()
            .take(params_owned.len() - 2)
            .map(|b| b.as_ref() as &dyn rusqlite::ToSql)
            .collect();
        let total: i64 = conn
            .query_row(&count_sql, count_params.as_slice(), |r| r.get(0))
            .context("count chunks")?;

        // Execute list query.
        let mut stmt = conn.prepare(&sql).context("prepare list_chunks")?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params_owned
            .iter()
            .map(|b| b.as_ref() as &dyn rusqlite::ToSql)
            .collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                let id: String = row.get(0)?;
                let source_kind: String = row.get(1)?;
                let source_id: String = row.get(2)?;
                let source_ref: Option<String> = row.get(3)?;
                let owner: String = row.get(4)?;
                let timestamp_ms: i64 = row.get(5)?;
                let token_count: i64 = row.get(6)?;
                let lifecycle_status: String = row.get(7)?;
                let content_path: Option<String> = row.get(8)?;
                let content: String = row.get(9)?;
                let tags_json: String = row.get(10)?;
                let has_embedding: i64 = row.get(11)?;
                let preview: String = content.chars().take(PREVIEW_MAX_CHARS).collect();
                let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                Ok(ChunkRow {
                    id,
                    source_kind,
                    source_id,
                    source_ref,
                    owner,
                    timestamp_ms,
                    token_count: token_count.max(0) as u32,
                    lifecycle_status,
                    content_path,
                    content_preview: if preview.is_empty() {
                        None
                    } else {
                        Some(preview)
                    },
                    has_embedding: has_embedding != 0,
                    tags,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("collect list_chunks rows")?;

        Ok(ListChunksResponse {
            chunks: rows,
            total: total.max(0) as u64,
        })
    })
}

// ── list_sources ─────────────────────────────────────────────────────────

/// `memory_tree_list_sources` — distinct (source_kind, source_id) pairs
/// with aggregate chunk counts and most-recent timestamps. Display name is
/// computed from the `source_id` (un-slug; user email stripping where the
/// caller can supply the user's primary email via `user_email_hint`).
pub async fn list_sources_rpc(
    config: &Config,
    user_email_hint: Option<String>,
) -> Result<RpcOutcome<Vec<Source>>, String> {
    let cfg = config.clone();
    let sources = tokio::task::spawn_blocking(move || -> Result<Vec<Source>> {
        list_sources_blocking(&cfg, user_email_hint.as_deref())
    })
    .await
    .map_err(|e| format!("list_sources join error: {e}"))?
    .map_err(|e| format!("list_sources: {e:#}"))?;

    let n = sources.len();
    Ok(RpcOutcome::single_log(
        sources,
        format!("memory_tree::read: list_sources n={n}"),
    ))
}

fn list_sources_blocking(config: &Config, user_email_hint: Option<&str>) -> Result<Vec<Source>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT source_kind, source_id, COUNT(*) AS n, MAX(timestamp_ms) AS most_recent
               FROM mem_tree_chunks
              GROUP BY source_kind, source_id
              ORDER BY most_recent DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                let source_kind: String = row.get(0)?;
                let source_id: String = row.get(1)?;
                let n: i64 = row.get(2)?;
                let most_recent: i64 = row.get(3)?;
                let display_name = display_name_for_source(&source_id, user_email_hint);
                Ok(Source {
                    source_id,
                    display_name,
                    source_kind,
                    chunk_count: n.max(0) as u32,
                    most_recent_ms: most_recent,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("collect list_sources rows")?;
        Ok(rows)
    })
}

/// Compute the display name for a source. Pure / table-driven so the unit
/// tests can lock in the un-slug behaviour.
///
/// Examples:
/// - `slack:#engineering` → `#engineering` (slack channel)
/// - `gmail:alice@example.com|bob@example.com` (user is alice) → `bob@example.com`
/// - `gmail:alice@example.com|bob@example.com` (user unknown) →
///   `alice@example.com ↔ bob@example.com`
/// - `notion:page-id-1234` → `page-id-1234`
fn display_name_for_source(source_id: &str, user_email_hint: Option<&str>) -> String {
    // Drop the platform prefix if there is one.
    let body = match source_id.split_once(':') {
        Some((_platform, rest)) => rest,
        None => source_id,
    };
    // Email-thread ids often look like `a@x|b@y`. If the user's email is
    // surfaced and matches one side, return only the other side.
    if body.contains('|') {
        let parts: Vec<&str> = body.split('|').collect();
        if let Some(user) = user_email_hint {
            let user_lc = user.trim().to_ascii_lowercase();
            let others: Vec<&str> = parts
                .iter()
                .copied()
                .filter(|p| p.trim().to_ascii_lowercase() != user_lc)
                .collect();
            if !others.is_empty() && others.len() < parts.len() {
                return others.join(", ");
            }
        }
        // No user hint or no match — show all parties separated by an arrow.
        return parts.join(" ↔ ");
    }
    body.to_string()
}

// ── search / recall ──────────────────────────────────────────────────────

/// `memory_tree_search` — keyword `LIKE '%q%'` over chunk bodies. Cheap,
/// deterministic, and useful as a fast fallback when the embedder is
/// offline or the query is short. Returns hits ordered by recency.
pub async fn search_rpc(
    config: &Config,
    query: String,
    k: u32,
) -> Result<RpcOutcome<Vec<ChunkRow>>, String> {
    let limit = k.clamp(1, MAX_LIST_LIMIT);
    let filter = ChunkFilter {
        query: Some(query.clone()),
        limit: Some(limit),
        ..ChunkFilter::default()
    };
    let cfg = config.clone();
    let chunks = tokio::task::spawn_blocking(move || -> Result<Vec<ChunkRow>> {
        Ok(list_chunks_blocking(&cfg, &filter)?.chunks)
    })
    .await
    .map_err(|e| format!("search join error: {e}"))?
    .map_err(|e| format!("search: {e:#}"))?;

    let n = chunks.len();
    Ok(RpcOutcome::single_log(
        chunks,
        format!("memory_tree::read: search query_len={} n={n}", query.len()),
    ))
}

/// Response shape for [`recall_rpc`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecallResponse {
    pub chunks: Vec<ChunkRow>,
    pub scores: Vec<f32>,
}

/// `memory_tree_recall` — semantic recall via the existing Phase 4 rerank
/// path. Calls into `retrieval::query_source(query=Some(q))` and converts
/// the top-K summary hits into chunk rows by walking the summary
/// `child_ids`. UIs use this for "find me chunks like X".
///
/// Note: returns chunks (not summaries) because the Memory tab's design
/// is leaf-centric — users browse chunks, not summary nodes.
pub async fn recall_rpc(
    config: &Config,
    query: String,
    k: u32,
) -> Result<RpcOutcome<RecallResponse>, String> {
    let limit = k.clamp(1, MAX_LIST_LIMIT) as usize;
    log::debug!(
        "[memory_tree::read::recall] query_len={} k={}",
        query.len(),
        limit
    );

    // Reuse the source-tree retrieval path which already does cosine
    // rerank against query embeddings. We pull more summaries than `k`
    // because each summary expands into multiple leaves.
    let resp = crate::openhuman::memory::tree::retrieval::query_source(
        config,
        None,
        None,
        None,
        Some(query.as_str()),
        limit,
    )
    .await
    .map_err(|e| format!("recall query_source: {e:#}"))?;

    // Walk each hit's child_ids → leaves. Summary level=1 children are
    // chunks; for level>1 we'd need to recurse — keep it shallow for now
    // so a Memory tab call doesn't fan out unboundedly. Retrieval already
    // surfaces L1 first, so the shallow walk covers the common case.
    let mut chunk_rows: Vec<ChunkRow> = Vec::new();
    let mut scores: Vec<f32> = Vec::new();
    let cfg = config.clone();
    let leaves: Vec<(String, f32)> = resp
        .hits
        .into_iter()
        .filter(|h| matches!(h.node_kind, NodeKind::Summary) && h.level == 1)
        .flat_map(|h| {
            h.child_ids
                .into_iter()
                .map(move |id| (id, h.score))
                .collect::<Vec<_>>()
        })
        .collect();
    if !leaves.is_empty() {
        let collected = tokio::task::spawn_blocking(move || -> Result<Vec<(ChunkRow, f32)>> {
            with_connection(&cfg, |conn| {
                let mut out = Vec::with_capacity(leaves.len());
                for (chunk_id, score) in leaves {
                    let row = conn
                        .query_row(
                            "SELECT id, source_kind, source_id, source_ref, owner,
                                    timestamp_ms, token_count, lifecycle_status,
                                    content_path, content, tags_json,
                                    CASE WHEN embedding IS NULL THEN 0 ELSE 1 END
                               FROM mem_tree_chunks WHERE id = ?1",
                            params![chunk_id],
                            |r| {
                                let id: String = r.get(0)?;
                                let source_kind: String = r.get(1)?;
                                let source_id: String = r.get(2)?;
                                let source_ref: Option<String> = r.get(3)?;
                                let owner: String = r.get(4)?;
                                let timestamp_ms: i64 = r.get(5)?;
                                let token_count: i64 = r.get(6)?;
                                let lifecycle_status: String = r.get(7)?;
                                let content_path: Option<String> = r.get(8)?;
                                let content: String = r.get(9)?;
                                let tags_json: String = r.get(10)?;
                                let has_emb: i64 = r.get(11)?;
                                let preview: String =
                                    content.chars().take(PREVIEW_MAX_CHARS).collect();
                                let tags: Vec<String> =
                                    serde_json::from_str(&tags_json).unwrap_or_default();
                                Ok(ChunkRow {
                                    id,
                                    source_kind,
                                    source_id,
                                    source_ref,
                                    owner,
                                    timestamp_ms,
                                    token_count: token_count.max(0) as u32,
                                    lifecycle_status,
                                    content_path,
                                    content_preview: if preview.is_empty() {
                                        None
                                    } else {
                                        Some(preview)
                                    },
                                    has_embedding: has_emb != 0,
                                    tags,
                                })
                            },
                        )
                        .ok();
                    if let Some(r) = row {
                        out.push((r, score));
                    }
                }
                Ok(out)
            })
        })
        .await
        .map_err(|e| format!("recall join error: {e}"))?
        .map_err(|e| format!("recall hydrate: {e:#}"))?;
        for (row, sc) in collected {
            chunk_rows.push(row);
            scores.push(sc);
        }
    }
    chunk_rows.truncate(limit);
    scores.truncate(limit);

    let n = chunk_rows.len();
    Ok(RpcOutcome::single_log(
        RecallResponse {
            chunks: chunk_rows,
            scores,
        },
        format!("memory_tree::read: recall n={n}"),
    ))
}

// ── entity index lookups ────────────────────────────────────────────────

/// `memory_tree_entity_index_for` — return all canonical entities indexed
/// against a single chunk (or summary) node id.
pub async fn entity_index_for_rpc(
    config: &Config,
    chunk_id: String,
) -> Result<RpcOutcome<Vec<EntityRef>>, String> {
    let cfg = config.clone();
    let id = chunk_id.clone();
    let refs = tokio::task::spawn_blocking(move || -> Result<Vec<EntityRef>> {
        with_connection(&cfg, |conn| {
            let mut stmt = conn.prepare(
                "SELECT entity_id, entity_kind, surface, COUNT(*) AS n
                   FROM mem_tree_entity_index
                  WHERE node_id = ?1
                  GROUP BY entity_id, entity_kind, surface
                  ORDER BY n DESC, entity_id ASC",
            )?;
            let rows = stmt
                .query_map(params![id], |row| {
                    let entity_id: String = row.get(0)?;
                    let kind: String = row.get(1)?;
                    let surface: String = row.get(2)?;
                    let n: i64 = row.get(3)?;
                    Ok(EntityRef {
                        entity_id,
                        kind,
                        surface,
                        count: n.max(0) as u32,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("collect entity_index_for rows")?;
            Ok(rows)
        })
    })
    .await
    .map_err(|e| format!("entity_index_for join error: {e}"))?
    .map_err(|e| format!("entity_index_for: {e:#}"))?;

    let n = refs.len();
    Ok(RpcOutcome::single_log(
        refs,
        format!("memory_tree::read: entity_index_for chunk_id={chunk_id} n={n}"),
    ))
}

/// `memory_tree_chunks_for_entity` — return chunk IDs that reference an
/// entity_id. Inverse of `entity_index_for`. Used by the Memory tab's
/// People/Topics lenses to filter the chunk list to those mentioning a
/// selected entity.
pub async fn chunks_for_entity_rpc(
    config: &Config,
    entity_id: String,
) -> Result<RpcOutcome<Vec<String>>, String> {
    let cfg = config.clone();
    let eid = entity_id.clone();
    let chunk_ids = tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
        with_connection(&cfg, |conn| {
            let mut stmt = conn.prepare(
                // node_kind values are `leaf` (= chunk node, the actual
                // chunk_id) and `summary` (= sealed bucket summary).
                // Memory tab filtering wants the chunk-level rows only.
                "SELECT DISTINCT node_id
                   FROM mem_tree_entity_index
                  WHERE entity_id = ?1 AND node_kind = 'leaf'
                  ORDER BY timestamp_ms DESC",
            )?;
            let rows = stmt
                .query_map(params![eid], |row| {
                    let node_id: String = row.get(0)?;
                    Ok(node_id)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("collect chunks_for_entity rows")?;
            Ok(rows)
        })
    })
    .await
    .map_err(|e| format!("chunks_for_entity join error: {e}"))?
    .map_err(|e| format!("chunks_for_entity: {e:#}"))?;

    let n = chunk_ids.len();
    Ok(RpcOutcome::single_log(
        chunk_ids,
        format!("memory_tree::read: chunks_for_entity entity_id={entity_id} n={n}"),
    ))
}

/// `memory_tree_top_entities` — most-frequent canonical entities,
/// optionally narrowed to one [`EntityKind`].
pub async fn top_entities_rpc(
    config: &Config,
    kind: Option<String>,
    limit: u32,
) -> Result<RpcOutcome<Vec<EntityRef>>, String> {
    let limit = limit.clamp(1, MAX_LIST_LIMIT);
    let cfg = config.clone();
    let refs = tokio::task::spawn_blocking(move || -> Result<Vec<EntityRef>> {
        with_connection(&cfg, |conn| {
            let mut sql = String::from(
                "SELECT entity_id, entity_kind, MAX(surface) AS surface_sample, COUNT(*) AS n
                   FROM mem_tree_entity_index",
            );
            let mut params_owned: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            if let Some(k) = kind {
                sql.push_str(" WHERE entity_kind = ?");
                params_owned.push(Box::new(k));
            }
            sql.push_str(
                " GROUP BY entity_id, entity_kind
                  ORDER BY n DESC, MAX(timestamp_ms) DESC
                  LIMIT ?",
            );
            params_owned.push(Box::new(limit as i64));
            let mut stmt = conn.prepare(&sql)?;
            let param_refs: Vec<&dyn rusqlite::ToSql> = params_owned
                .iter()
                .map(|b| b.as_ref() as &dyn rusqlite::ToSql)
                .collect();
            let rows = stmt
                .query_map(param_refs.as_slice(), |row| {
                    let entity_id: String = row.get(0)?;
                    let kind: String = row.get(1)?;
                    let surface: String = row.get(2)?;
                    let n: i64 = row.get(3)?;
                    Ok(EntityRef {
                        entity_id,
                        kind,
                        surface,
                        count: n.max(0) as u32,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("collect top_entities rows")?;
            Ok(rows)
        })
    })
    .await
    .map_err(|e| format!("top_entities join error: {e}"))?
    .map_err(|e| format!("top_entities: {e:#}"))?;

    let n = refs.len();
    Ok(RpcOutcome::single_log(
        refs,
        format!("memory_tree::read: top_entities n={n}"),
    ))
}

// ── chunk_score ─────────────────────────────────────────────────────────

/// `memory_tree_chunk_score` — return the score breakdown stored in
/// `mem_tree_score` for one chunk. UI uses this to render the "why was
/// this kept / dropped" panel.
pub async fn chunk_score_rpc(
    config: &Config,
    chunk_id: String,
) -> Result<RpcOutcome<Option<ScoreBreakdown>>, String> {
    let cfg = config.clone();
    let id = chunk_id.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Option<ScoreBreakdown>> {
        let row = score_store::get_score(&cfg, &id)?;
        Ok(row.map(|r| {
            // Hard-code the cheap-signal weights from `SignalWeights::default()`
            // / `with_llm_enabled()`. The score row doesn't persist the weights
            // it was scored with, so we read them from the same defaults the
            // scoring path uses. This is acceptable because the weights are
            // derived constants — see `score::signals::types`.
            let llm_consulted = r.signals.llm_importance > 0.0;
            let signals = vec![
                ScoreSignal {
                    name: "token_count".into(),
                    weight: 1.0,
                    value: r.signals.token_count,
                },
                ScoreSignal {
                    name: "unique_words".into(),
                    weight: 1.0,
                    value: r.signals.unique_words,
                },
                ScoreSignal {
                    name: "metadata_weight".into(),
                    weight: 1.5,
                    value: r.signals.metadata_weight,
                },
                ScoreSignal {
                    name: "source_weight".into(),
                    weight: 1.5,
                    value: r.signals.source_weight,
                },
                ScoreSignal {
                    name: "interaction".into(),
                    weight: 3.0,
                    value: r.signals.interaction,
                },
                ScoreSignal {
                    name: "entity_density".into(),
                    weight: 1.0,
                    value: r.signals.entity_density,
                },
                ScoreSignal {
                    name: "llm_importance".into(),
                    weight: if llm_consulted { 2.0 } else { 0.0 },
                    value: r.signals.llm_importance,
                },
            ];
            ScoreBreakdown {
                signals,
                total: r.total,
                threshold: crate::openhuman::memory::tree::score::DEFAULT_DROP_THRESHOLD,
                kept: !r.dropped,
                llm_consulted,
            }
        }))
    })
    .await
    .map_err(|e| format!("chunk_score join error: {e}"))?
    .map_err(|e| format!("chunk_score: {e:#}"))?;
    Ok(RpcOutcome::single_log(
        result,
        format!("memory_tree::read: chunk_score id={chunk_id}"),
    ))
}

// ── delete_chunk ────────────────────────────────────────────────────────

/// `memory_tree_delete_chunk` — purge one chunk plus its score row and
/// entity-index rows. Idempotent — missing chunk returns success with
/// `deleted=false`.
///
/// Does NOT cascade through summary nodes — sealed summaries are
/// immutable; deletion of leaves attached to a sealed summary leaves the
/// summary referencing a now-missing child id. UIs warn the user and
/// callers wanting full cascade should rebuild the affected tree by
/// re-ingesting upstream.
pub async fn delete_chunk_rpc(
    config: &Config,
    chunk_id: String,
) -> Result<RpcOutcome<DeleteChunkResponse>, String> {
    let cfg = config.clone();
    let id = chunk_id.clone();
    let resp = tokio::task::spawn_blocking(move || -> Result<DeleteChunkResponse> {
        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            // Find the chunk's content_path so we can also remove the .md file.
            let content_path: Option<String> = tx
                .query_row(
                    "SELECT content_path FROM mem_tree_chunks WHERE id = ?1",
                    params![id],
                    |r| r.get::<_, Option<String>>(0),
                )
                .ok()
                .flatten();
            let removed_score =
                tx.execute("DELETE FROM mem_tree_score WHERE chunk_id = ?1", params![id])?;
            let removed_index = tx.execute(
                "DELETE FROM mem_tree_entity_index WHERE node_id = ?1",
                params![id],
            )?;
            let removed_chunk =
                tx.execute("DELETE FROM mem_tree_chunks WHERE id = ?1", params![id])?;
            tx.commit()?;
            // Best-effort filesystem cleanup outside the SQL tx.
            if let Some(rel) = content_path {
                let mut path = cfg.memory_tree_content_root();
                for component in rel.split('/') {
                    path.push(component);
                }
                if let Err(e) = std::fs::remove_file(&path) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        log::warn!(
                            "[memory_tree::read::delete] failed to remove chunk file path_hash={}: {e}",
                            crate::openhuman::memory::tree::util::redact::redact(&rel),
                        );
                    }
                }
            }
            Ok(DeleteChunkResponse {
                deleted: removed_chunk > 0,
                score_rows_removed: removed_score as u32,
                entity_index_rows_removed: removed_index as u32,
            })
        })
    })
    .await
    .map_err(|e| format!("delete_chunk join error: {e}"))?
    .map_err(|e| format!("delete_chunk: {e:#}"))?;
    Ok(RpcOutcome::single_log(
        resp.clone(),
        format!(
            "memory_tree::read: delete_chunk id={chunk_id} deleted={} score_rows={} entity_rows={}",
            resp.deleted, resp.score_rows_removed, resp.entity_index_rows_removed
        ),
    ))
}

/// Response shape for [`delete_chunk_rpc`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeleteChunkResponse {
    pub deleted: bool,
    pub score_rows_removed: u32,
    pub entity_index_rows_removed: u32,
}

// ── llm get/set ─────────────────────────────────────────────────────────

/// Response shape for [`get_llm_rpc`] / [`set_llm_rpc`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LlmResponse {
    /// `"cloud"` or `"local"`.
    pub current: String,
}

/// Request shape for [`set_llm_rpc`].
///
/// The handler always updates `memory_tree.llm_backend` from the required
/// `backend` field. The three model fields are optional and follow
/// "absent → unchanged, present → overwritten" semantics so the UI can
/// either flip the mode without touching models, or persist a per-role
/// model selection without forcing the caller to re-supply every other
/// model id. All updates land in a single atomic `Config::save` write.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct SetLlmRequest {
    /// Required: which backend to use for chat (extract + summariser).
    pub backend: String,

    /// Optional: when `backend = "cloud"`, the cloud model id to use. If
    /// `None`, the existing `config.memory_tree.cloud_llm_model` stays
    /// unchanged.
    #[serde(default)]
    pub cloud_model: Option<String>,

    /// Optional: when `backend = "local"`, the Ollama model id the
    /// `LlmEntityExtractor` should use. If `None`, the existing
    /// `config.memory_tree.llm_extractor_model` stays unchanged.
    #[serde(default)]
    pub extract_model: Option<String>,

    /// Optional: when `backend = "local"`, the Ollama model id the
    /// `LlmSummariser` should use. If `None`, the existing
    /// `config.memory_tree.llm_summariser_model` stays unchanged.
    #[serde(default)]
    pub summariser_model: Option<String>,
}

/// `memory_tree_get_llm` — read the currently configured LLM backend.
pub async fn get_llm_rpc(config: &Config) -> Result<RpcOutcome<LlmResponse>, String> {
    let current = config.memory_tree.llm_backend.as_str().to_string();
    Ok(RpcOutcome::single_log(
        LlmResponse {
            current: current.clone(),
        },
        format!("memory_tree::read: get_llm current={current}"),
    ))
}

/// `memory_tree_set_llm` — overwrite the LLM backend selector (and
/// optionally per-role model choices) and persist the result to
/// `config.toml`.
///
/// Mutates the in-memory [`Config`] passed in (so the caller's running
/// instance picks up the new value immediately) and writes it to disk via
/// [`Config::save`], which uses an atomic temp-file + rename so a crash
/// mid-write can't corrupt the config. The next sidecar restart reads the
/// persisted values rather than reverting to defaults.
///
/// The three optional model fields follow "absent → corresponding config
/// key untouched, present → overwritten" semantics, so the UI can call
/// `{ backend: "cloud" }` to flip the mode without touching the models or
/// `{ backend: "local", extract_model: Some(...), summariser_model: Some(...) }`
/// to flip mode + set both local models in one atomic write.
pub async fn set_llm_rpc(
    config: &mut Config,
    req: SetLlmRequest,
) -> Result<RpcOutcome<LlmResponse>, String> {
    let parsed = crate::openhuman::config::LlmBackend::parse(&req.backend)
        .map_err(|e| format!("set_llm: {e}"))?;

    // Stage all updates on a clone first, persist, and only commit to the
    // live `&mut Config` if save succeeds. Without this, a save() failure
    // (disk full, permissions, ENOSPC mid-write) leaves the in-memory
    // config divergent from disk: the worker pool would build a chat
    // provider against the new model id while config.toml still reflects
    // the old one, so the next sidecar restart would silently revert.
    let mut staged = config.clone();
    staged.memory_tree.llm_backend = parsed;

    let mut changed_models: Vec<&'static str> = Vec::new();
    if let Some(model) = req.cloud_model {
        log::debug!(
            "[memory_tree::read] staging memory_tree.cloud_llm_model={}",
            model
        );
        staged.memory_tree.cloud_llm_model = Some(model);
        changed_models.push("cloud_model");
    }
    if let Some(model) = req.extract_model {
        log::debug!(
            "[memory_tree::read] staging memory_tree.llm_extractor_model={}",
            model
        );
        staged.memory_tree.llm_extractor_model = Some(model);
        changed_models.push("extract_model");
    }
    if let Some(model) = req.summariser_model {
        log::debug!(
            "[memory_tree::read] staging memory_tree.llm_summariser_model={}",
            model
        );
        staged.memory_tree.llm_summariser_model = Some(model);
        changed_models.push("summariser_model");
    }

    // Persist the staged version to config.toml. Atomic write-temp +
    // rename per Config::save. Commit to the live config only after a
    // successful write.
    log::debug!(
        "[memory_tree::read] persisting memory_tree.llm_backend={} (changed_models={:?}) to {}",
        parsed.as_str(),
        changed_models,
        staged.config_path.display()
    );
    staged
        .save()
        .await
        .map_err(|e| format!("set_llm: persist to config.toml failed: {e}"))?;
    *config = staged;

    let effective = parsed.as_str().to_string();
    log::info!(
        "[memory_tree::read] llm_backend switched to {} (changed_models={:?}) and persisted to {}",
        effective,
        changed_models,
        config.config_path.display()
    );
    Ok(RpcOutcome::single_log(
        LlmResponse {
            current: effective.clone(),
        },
        format!(
            "memory_tree::read: set_llm current={effective} changed_models={:?}",
            changed_models
        ),
    ))
}

// ── small helpers ───────────────────────────────────────────────────────

/// Fetch the raw `mem_tree_chunks` row plus a content preview, suitable
/// for building a [`ChunkRow`]. Used by [`chunk_store::get_chunk`] callers
/// who don't want to walk all the way back through the existing read
/// path. Currently unused publicly — kept for the JSON-RPC layer to call
/// when wiring per-id reads.
#[allow(dead_code)]
pub(crate) fn read_chunk_row(config: &Config, chunk_id: &str) -> Result<Option<ChunkRow>> {
    let chunk = match chunk_store::get_chunk(config, chunk_id)? {
        Some(c) => c,
        None => return Ok(None),
    };
    // Try to load the full body for the preview, falling back to whatever
    // SQLite has if the on-disk file is missing.
    let body =
        content_read::read_chunk_body(config, chunk_id).unwrap_or_else(|_| chunk.content.clone());
    let preview: String = body.chars().take(PREVIEW_MAX_CHARS).collect();
    let has_embedding = chunk_store::get_chunk_embedding(config, chunk_id)?.is_some();
    Ok(Some(ChunkRow {
        id: chunk.id,
        source_kind: chunk.metadata.source_kind.as_str().to_string(),
        source_id: chunk.metadata.source_id,
        source_ref: chunk.metadata.source_ref.map(|r| r.value),
        owner: chunk.metadata.owner,
        timestamp_ms: chunk.metadata.timestamp.timestamp_millis(),
        token_count: chunk.token_count,
        lifecycle_status: chunk_store::get_chunk_lifecycle_status(config, chunk_id)?
            .unwrap_or_else(|| "unknown".to_string()),
        content_path: chunk_store::get_chunk_content_path(config, chunk_id)?,
        content_preview: if preview.is_empty() {
            None
        } else {
            Some(preview)
        },
        has_embedding,
        tags: chunk.metadata.tags,
    }))
}

#[allow(dead_code)]
fn parse_source_kind_str(s: &str) -> Option<SourceKind> {
    SourceKind::parse(s).ok()
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::canonicalize::chat::{ChatBatch, ChatMessage};
    use crate::openhuman::memory::tree::ingest::ingest_chat;
    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        // Point config_path inside the tempdir so set_llm_rpc's
        // Config::save call writes to a disposable location instead of
        // touching the real user config.
        cfg.config_path = tmp.path().join("config.toml");
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        // Default llm is Cloud — but the cloud provider needs a bearer
        // token to actually fire. Tests that exercise the LLM path
        // override either the backend or the extractor. The read RPCs
        // below don't touch the LLM, so this default is fine.
        (tmp, cfg)
    }

    async fn seed_chat_chunk(cfg: &Config, source: &str, body: &str) {
        let batch = ChatBatch {
            platform: "slack".into(),
            channel_label: source.into(),
            messages: vec![ChatMessage {
                author: "alice".into(),
                timestamp: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
                text: body.into(),
                source_ref: Some("slack://x".into()),
            }],
        };
        ingest_chat(cfg, source, "alice", vec![], batch)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn list_chunks_returns_seeded_chunk() {
        let (_tmp, cfg) = test_config();
        seed_chat_chunk(&cfg, "slack:#eng", "hello @alice phoenix migration").await;
        let resp = list_chunks_rpc(&cfg, ChunkFilter::default())
            .await
            .unwrap()
            .value;
        assert!(!resp.chunks.is_empty());
        assert_eq!(resp.total, resp.chunks.len() as u64);
    }

    #[tokio::test]
    async fn list_chunks_filters_by_source_id() {
        let (_tmp, cfg) = test_config();
        seed_chat_chunk(&cfg, "slack:#a", "alpha").await;
        seed_chat_chunk(&cfg, "slack:#b", "beta").await;
        let only_a = list_chunks_rpc(
            &cfg,
            ChunkFilter {
                source_ids: Some(vec!["slack:#a".into()]),
                ..ChunkFilter::default()
            },
        )
        .await
        .unwrap()
        .value;
        assert!(only_a.chunks.iter().all(|c| c.source_id == "slack:#a"));
        assert!(only_a.total >= 1);
    }

    #[tokio::test]
    async fn list_chunks_query_substring_works() {
        let (_tmp, cfg) = test_config();
        seed_chat_chunk(&cfg, "slack:#eng", "phoenix migration ships friday").await;
        seed_chat_chunk(&cfg, "slack:#eng", "different unrelated text").await;
        let resp = list_chunks_rpc(
            &cfg,
            ChunkFilter {
                query: Some("phoenix".into()),
                ..ChunkFilter::default()
            },
        )
        .await
        .unwrap()
        .value;
        assert!(resp.chunks.iter().any(|c| c
            .content_preview
            .as_deref()
            .unwrap_or("")
            .contains("phoenix")));
    }

    #[tokio::test]
    async fn list_sources_aggregates() {
        let (_tmp, cfg) = test_config();
        seed_chat_chunk(&cfg, "slack:#a", "x").await;
        seed_chat_chunk(&cfg, "slack:#a", "y").await;
        seed_chat_chunk(&cfg, "slack:#b", "z").await;
        let sources = list_sources_rpc(&cfg, None).await.unwrap().value;
        let a = sources
            .iter()
            .find(|s| s.source_id == "slack:#a")
            .expect("expected slack:#a");
        let b = sources
            .iter()
            .find(|s| s.source_id == "slack:#b")
            .expect("expected slack:#b");
        assert_eq!(a.chunk_count, 2);
        assert_eq!(b.chunk_count, 1);
    }

    #[tokio::test]
    async fn entity_index_for_returns_extracted_entities() {
        let (_tmp, cfg) = test_config();
        seed_chat_chunk(&cfg, "slack:#eng", "alice@example.com owns it").await;
        // Find the chunk we just seeded.
        let chunks = list_chunks_rpc(&cfg, ChunkFilter::default())
            .await
            .unwrap()
            .value
            .chunks;
        let id = &chunks[0].id;
        let refs = entity_index_for_rpc(&cfg, id.clone()).await.unwrap().value;
        assert!(
            refs.iter().any(|r| r.entity_id.contains("alice")),
            "expected alice entity in index, got: {refs:?}"
        );
    }

    #[tokio::test]
    async fn top_entities_returns_most_frequent() {
        let (_tmp, cfg) = test_config();
        seed_chat_chunk(&cfg, "slack:#a", "alice@example.com x").await;
        seed_chat_chunk(&cfg, "slack:#b", "alice@example.com y").await;
        seed_chat_chunk(&cfg, "slack:#c", "bob@example.com z").await;
        let top = top_entities_rpc(&cfg, Some("email".into()), 10)
            .await
            .unwrap()
            .value;
        assert!(top
            .iter()
            .any(|e| e.entity_id == "email:alice@example.com" && e.count >= 2));
    }

    #[tokio::test]
    async fn delete_chunk_removes_chunk_and_dependent_rows() {
        let (_tmp, cfg) = test_config();
        seed_chat_chunk(&cfg, "slack:#eng", "alice@example.com owns it").await;
        let chunks = list_chunks_rpc(&cfg, ChunkFilter::default())
            .await
            .unwrap()
            .value
            .chunks;
        let id = chunks[0].id.clone();
        let resp = delete_chunk_rpc(&cfg, id.clone()).await.unwrap().value;
        assert!(resp.deleted);
        // Re-list — the chunk should be gone.
        let after = list_chunks_rpc(&cfg, ChunkFilter::default())
            .await
            .unwrap()
            .value;
        assert!(after.chunks.iter().all(|c| c.id != id));
    }

    #[tokio::test]
    async fn delete_missing_chunk_is_idempotent() {
        let (_tmp, cfg) = test_config();
        let resp = delete_chunk_rpc(&cfg, "does-not-exist".into())
            .await
            .unwrap()
            .value;
        assert!(!resp.deleted);
        assert_eq!(resp.score_rows_removed, 0);
    }

    #[tokio::test]
    async fn chunk_score_returns_breakdown_after_ingest() {
        let (_tmp, cfg) = test_config();
        seed_chat_chunk(
            &cfg,
            "slack:#eng",
            "alice@example.com owns the phoenix migration",
        )
        .await;
        let chunks = list_chunks_rpc(&cfg, ChunkFilter::default())
            .await
            .unwrap()
            .value
            .chunks;
        let id = &chunks[0].id;
        let breakdown = chunk_score_rpc(&cfg, id.clone()).await.unwrap().value;
        assert!(breakdown.is_some(), "expected score row after ingest");
        let b = breakdown.unwrap();
        assert!(b.signals.iter().any(|s| s.name == "metadata_weight"));
        assert!(b.threshold > 0.0);
    }

    #[tokio::test]
    async fn search_returns_matching_chunks() {
        let (_tmp, cfg) = test_config();
        seed_chat_chunk(&cfg, "slack:#eng", "phoenix migration scheduled friday").await;
        seed_chat_chunk(&cfg, "slack:#eng", "different unrelated text").await;
        let hits = search_rpc(&cfg, "phoenix".into(), 10).await.unwrap().value;
        assert!(hits.iter().any(|c| c
            .content_preview
            .as_deref()
            .unwrap_or("")
            .contains("phoenix")));
    }

    #[tokio::test]
    async fn get_llm_returns_cloud_by_default() {
        let (_tmp, cfg) = test_config();
        let resp = get_llm_rpc(&cfg).await.unwrap().value;
        assert_eq!(resp.current, "cloud");
    }

    /// Test helper — build a backend-only `SetLlmRequest` with all model
    /// overrides set to `None`. Used by tests that want the legacy
    /// "flip the backend, leave models untouched" behaviour.
    fn req_backend_only(backend: &str) -> SetLlmRequest {
        SetLlmRequest {
            backend: backend.into(),
            cloud_model: None,
            extract_model: None,
            summariser_model: None,
        }
    }

    #[tokio::test]
    async fn set_llm_switches_in_memory_and_persists_to_config_toml() {
        let (_tmp, mut cfg) = test_config();
        let config_path = cfg.config_path.clone();

        let resp = set_llm_rpc(&mut cfg, req_backend_only("local"))
            .await
            .unwrap()
            .value;
        assert_eq!(resp.current, "local");
        // 1. In-memory state updated.
        assert_eq!(
            cfg.memory_tree.llm_backend,
            crate::openhuman::config::LlmBackend::Local
        );

        // 2. config.toml on disk updated. The file should exist (Config::save
        //    always writes — there is no "skip default" branch) and the
        //    [memory_tree] section should contain `llm_backend = "local"`.
        assert!(
            config_path.is_file(),
            "expected set_llm to create config.toml at {}",
            config_path.display()
        );
        let on_disk =
            std::fs::read_to_string(&config_path).expect("read config.toml after set_llm");
        let parsed: toml::Value =
            toml::from_str(&on_disk).expect("parse config.toml after set_llm");
        let llm_field = parsed
            .get("memory_tree")
            .and_then(|m| m.get("llm_backend"))
            .and_then(|v| v.as_str())
            .expect("memory_tree.llm_backend present in persisted config.toml");
        assert_eq!(llm_field, "local");

        // 3. get_llm_rpc on the same in-memory config reports the new value.
        let after = get_llm_rpc(&cfg).await.unwrap().value;
        assert_eq!(after.current, "local");
    }

    #[tokio::test]
    async fn set_llm_persists_when_section_does_not_yet_exist() {
        // First-call scenario: config.toml does not exist yet. set_llm_rpc
        // must create it (via Config::save) with a `[memory_tree]` section
        // containing the chosen value.
        let (_tmp, mut cfg) = test_config();
        let config_path = cfg.config_path.clone();
        assert!(
            !config_path.exists(),
            "precondition: config.toml should not exist yet"
        );

        let _ = set_llm_rpc(&mut cfg, req_backend_only("local"))
            .await
            .unwrap()
            .value;
        assert!(
            config_path.is_file(),
            "set_llm must create config.toml on first call"
        );
        let on_disk =
            std::fs::read_to_string(&config_path).expect("read config.toml after first set_llm");
        let parsed: toml::Value =
            toml::from_str(&on_disk).expect("parse config.toml after first set_llm");
        assert_eq!(
            parsed
                .get("memory_tree")
                .and_then(|m| m.get("llm_backend"))
                .and_then(|v| v.as_str()),
            Some("local"),
        );
    }

    #[tokio::test]
    async fn set_llm_rejects_unknown() {
        let (_tmp, mut cfg) = test_config();
        let err = set_llm_rpc(&mut cfg, req_backend_only("hybrid"))
            .await
            .unwrap_err();
        assert!(err.contains("unknown llm"));
    }

    #[tokio::test]
    async fn set_llm_with_cloud_model_persists_cloud_model() {
        // Backend=cloud + cloud_model=Some(...) → persisted config.toml has
        // both `llm_backend = "cloud"` AND `cloud_llm_model = "..."`.
        let (_tmp, mut cfg) = test_config();
        let config_path = cfg.config_path.clone();

        let resp = set_llm_rpc(
            &mut cfg,
            SetLlmRequest {
                backend: "cloud".into(),
                cloud_model: Some("summarizer-v2".into()),
                extract_model: None,
                summariser_model: None,
            },
        )
        .await
        .unwrap()
        .value;
        assert_eq!(resp.current, "cloud");

        // In-memory state updated.
        assert_eq!(
            cfg.memory_tree.cloud_llm_model.as_deref(),
            Some("summarizer-v2"),
        );

        // On-disk state updated — both fields land in [memory_tree].
        let on_disk = std::fs::read_to_string(&config_path).expect("read config.toml");
        let parsed: toml::Value = toml::from_str(&on_disk).expect("parse config.toml");
        let mt = parsed
            .get("memory_tree")
            .expect("expected [memory_tree] section");
        assert_eq!(
            mt.get("llm_backend").and_then(|v| v.as_str()),
            Some("cloud")
        );
        assert_eq!(
            mt.get("cloud_llm_model").and_then(|v| v.as_str()),
            Some("summarizer-v2"),
        );
    }

    #[tokio::test]
    async fn set_llm_with_local_models_persists_extract_and_summariser() {
        // Backend=local + both per-role model overrides → both fields land
        // in `[memory_tree]` in the same atomic write.
        let (_tmp, mut cfg) = test_config();
        let config_path = cfg.config_path.clone();

        let _ = set_llm_rpc(
            &mut cfg,
            SetLlmRequest {
                backend: "local".into(),
                cloud_model: None,
                extract_model: Some("qwen2.5:0.5b".into()),
                summariser_model: Some("gemma3:1b-it-qat".into()),
            },
        )
        .await
        .unwrap()
        .value;

        // In-memory state updated for both roles.
        assert_eq!(
            cfg.memory_tree.llm_extractor_model.as_deref(),
            Some("qwen2.5:0.5b"),
        );
        assert_eq!(
            cfg.memory_tree.llm_summariser_model.as_deref(),
            Some("gemma3:1b-it-qat"),
        );

        // Both fields persisted to disk under [memory_tree].
        let on_disk = std::fs::read_to_string(&config_path).expect("read config.toml");
        let parsed: toml::Value = toml::from_str(&on_disk).expect("parse config.toml");
        let mt = parsed
            .get("memory_tree")
            .expect("expected [memory_tree] section");
        assert_eq!(
            mt.get("llm_backend").and_then(|v| v.as_str()),
            Some("local")
        );
        assert_eq!(
            mt.get("llm_extractor_model").and_then(|v| v.as_str()),
            Some("qwen2.5:0.5b"),
        );
        assert_eq!(
            mt.get("llm_summariser_model").and_then(|v| v.as_str()),
            Some("gemma3:1b-it-qat"),
        );
    }

    #[tokio::test]
    async fn set_llm_without_models_leaves_existing_models_unchanged() {
        // Pre-seed config with an existing extractor model. Calling
        // set_llm_rpc with `{ backend: "local" }` (no model overrides)
        // must leave the existing `llm_extractor_model` intact on disk.
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.llm_extractor_model = Some("gemma3:1b".into());
        let config_path = cfg.config_path.clone();

        let _ = set_llm_rpc(&mut cfg, req_backend_only("local"))
            .await
            .unwrap()
            .value;

        // In-memory state still has the pre-seeded model.
        assert_eq!(
            cfg.memory_tree.llm_extractor_model.as_deref(),
            Some("gemma3:1b"),
        );

        // Disk also reflects the pre-seeded model — it was carried through
        // the Config::save round-trip even though set_llm didn't supply it.
        let on_disk = std::fs::read_to_string(&config_path).expect("read config.toml");
        let parsed: toml::Value = toml::from_str(&on_disk).expect("parse config.toml");
        assert_eq!(
            parsed
                .get("memory_tree")
                .and_then(|m| m.get("llm_extractor_model"))
                .and_then(|v| v.as_str()),
            Some("gemma3:1b"),
        );
    }

    #[tokio::test]
    async fn set_llm_with_partial_models_only_changes_provided() {
        // Pre-seed BOTH extract and summariser models. Call set_llm with
        // only `extract_model` set. The extractor must change; the
        // summariser must stay on the pre-seeded value.
        let (_tmp, mut cfg) = test_config();
        cfg.memory_tree.llm_extractor_model = Some("gemma3:1b".into());
        cfg.memory_tree.llm_summariser_model = Some("llama3.1:8b".into());
        let config_path = cfg.config_path.clone();

        let _ = set_llm_rpc(
            &mut cfg,
            SetLlmRequest {
                backend: "local".into(),
                cloud_model: None,
                extract_model: Some("qwen2.5:0.5b".into()),
                summariser_model: None,
            },
        )
        .await
        .unwrap()
        .value;

        // In-memory: extract changed, summariser unchanged.
        assert_eq!(
            cfg.memory_tree.llm_extractor_model.as_deref(),
            Some("qwen2.5:0.5b"),
        );
        assert_eq!(
            cfg.memory_tree.llm_summariser_model.as_deref(),
            Some("llama3.1:8b"),
        );

        // Disk reflects the same partial-update behaviour.
        let on_disk = std::fs::read_to_string(&config_path).expect("read config.toml");
        let parsed: toml::Value = toml::from_str(&on_disk).expect("parse config.toml");
        let mt = parsed
            .get("memory_tree")
            .expect("expected [memory_tree] section");
        assert_eq!(
            mt.get("llm_extractor_model").and_then(|v| v.as_str()),
            Some("qwen2.5:0.5b"),
        );
        assert_eq!(
            mt.get("llm_summariser_model").and_then(|v| v.as_str()),
            Some("llama3.1:8b"),
        );
    }

    #[test]
    fn display_name_unslugs_email_thread_with_user_hint() {
        let name = display_name_for_source(
            "gmail:alice@example.com|bob@example.com",
            Some("alice@example.com"),
        );
        assert_eq!(name, "bob@example.com");
    }

    #[test]
    fn display_name_falls_back_to_arrow_when_user_unknown() {
        let name = display_name_for_source("gmail:alice@example.com|bob@example.com", None);
        assert!(name.contains("alice@example.com"));
        assert!(name.contains("bob@example.com"));
        assert!(name.contains("↔"));
    }

    #[test]
    fn display_name_strips_platform_prefix() {
        assert_eq!(
            display_name_for_source("slack:#engineering", None),
            "#engineering"
        );
    }

    #[test]
    fn display_name_handles_no_prefix() {
        assert_eq!(display_name_for_source("loose-id", None), "loose-id");
    }
}
