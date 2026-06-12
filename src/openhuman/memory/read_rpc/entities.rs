use anyhow::{Context, Result};
use rusqlite::params;

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::chunks::store::with_connection;
use crate::openhuman::memory_tree::score::store as score_store;
use crate::rpc::RpcOutcome;

use super::types::{DeleteChunkResponse, EntityRef, ScoreBreakdown, ScoreSignal, MAX_LIST_LIMIT};

// ── entity index lookups ────────────────────────────────────────────────

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

pub async fn chunks_for_entity_rpc(
    config: &Config,
    entity_id: String,
) -> Result<RpcOutcome<Vec<String>>, String> {
    let cfg = config.clone();
    let eid = entity_id.clone();
    let chunk_ids = tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
        with_connection(&cfg, |conn| {
            let mut stmt = conn.prepare(
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

pub async fn chunk_score_rpc(
    config: &Config,
    chunk_id: String,
) -> Result<RpcOutcome<Option<ScoreBreakdown>>, String> {
    let cfg = config.clone();
    let id = chunk_id.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Option<ScoreBreakdown>> {
        let row = score_store::get_score(&cfg, &id)?;
        Ok(row.map(|r| {
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
                threshold: crate::openhuman::memory_tree::score::DEFAULT_DROP_THRESHOLD,
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

pub async fn delete_chunk_rpc(
    config: &Config,
    chunk_id: String,
) -> Result<RpcOutcome<DeleteChunkResponse>, String> {
    let cfg = config.clone();
    let id = chunk_id.clone();
    let resp = tokio::task::spawn_blocking(move || -> Result<DeleteChunkResponse> {
        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
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
            if let Some(rel) = content_path {
                let mut path = cfg.memory_tree_content_root();
                for component in rel.split('/') {
                    path.push(component);
                }
                if let Err(e) = std::fs::remove_file(&path) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        log::warn!(
                            "[memory_tree::read::delete] failed to remove chunk file path_hash={}: {e}",
                            crate::openhuman::memory::util::redact::redact(&rel),
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
