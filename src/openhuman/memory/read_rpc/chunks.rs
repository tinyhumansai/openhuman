use anyhow::{Context, Result};

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::chunks::store::{self as chunk_store, with_connection};
use crate::openhuman::memory_store::content::read as content_read;
use crate::openhuman::memory_tree::retrieval::types::NodeKind;
use crate::rpc::RpcOutcome;

use super::types::{
    ChunkFilter, ChunkRow, ListChunksResponse, RecallResponse, Source, DEFAULT_LIST_LIMIT,
    MAX_LIST_LIMIT, PREVIEW_MAX_CHARS,
};

// ── list_chunks ──────────────────────────────────────────────────────────

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

pub(super) fn list_chunks_blocking(
    config: &Config,
    filter: &ChunkFilter,
) -> Result<ListChunksResponse> {
    let limit = filter
        .limit
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .clamp(1, MAX_LIST_LIMIT);
    let offset = filter.offset.unwrap_or(0);

    with_connection(config, |conn| {
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
                where_clauses.push("c.content LIKE ?".into());
                params_owned.push(Box::new(format!("%{}%", q)));
            }
        }

        if !where_clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_clauses.join(" AND "));
        }
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

        let count_params: Vec<&dyn rusqlite::ToSql> = params_owned
            .iter()
            .take(params_owned.len() - 2)
            .map(|b| b.as_ref() as &dyn rusqlite::ToSql)
            .collect();
        let total: i64 = conn
            .query_row(&count_sql, count_params.as_slice(), |r| r.get(0))
            .context("count chunks")?;

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

/// Compute the display name for a source.
///
/// Examples:
/// - `slack:#engineering` → `#engineering`
/// - `gmail:alice@example.com|bob@example.com` (user is alice) → `bob@example.com`
/// - `gmail:alice@example.com|bob@example.com` (user unknown) →
///   `alice@example.com ↔ bob@example.com`
pub fn display_name_for_source(source_id: &str, user_email_hint: Option<&str>) -> String {
    let body = match source_id.split_once(':') {
        Some((_platform, rest)) => rest,
        None => source_id,
    };
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
        return parts.join(" ↔ ");
    }
    body.to_string()
}

// ── search / recall ──────────────────────────────────────────────────────

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

pub async fn recall_rpc(
    config: &Config,
    query: String,
    k: u32,
) -> Result<RpcOutcome<RecallResponse>, String> {
    use rusqlite::params;

    let limit = k.clamp(1, MAX_LIST_LIMIT) as usize;
    log::debug!(
        "[memory_tree::read::recall] query_len={} k={}",
        query.len(),
        limit
    );

    let resp = crate::openhuman::memory_tree::retrieval::query_source(
        config,
        None,
        None,
        None,
        Some(query.as_str()),
        limit,
    )
    .await
    .map_err(|e| format!("recall query_source: {e:#}"))?;

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

// ── small helpers ───────────────────────────────────────────────────────

pub fn read_chunk_row(config: &Config, chunk_id: &str) -> Result<Option<ChunkRow>> {
    let chunk = match chunk_store::get_chunk(config, chunk_id)? {
        Some(c) => c,
        None => return Ok(None),
    };
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
