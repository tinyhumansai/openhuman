//! Domain RPC handlers for life-capture. Adapter handlers in `schemas.rs`
//! deserialise params and call into these functions; tests can call them
//! directly with a constructed `LifeCaptureRuntime`.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use crate::openhuman::life_capture::embedder::Embedder;
use crate::openhuman::life_capture::index::{IndexReader, IndexWriter, PersonalIndex};
use crate::openhuman::life_capture::types::{Item, Query, Source};
use crate::rpc::RpcOutcome;

/// Returns total item count, per-source counts, and the most recent item ts
/// (unix seconds, or null when the index is empty).
pub async fn handle_get_stats(idx: &PersonalIndex) -> Result<RpcOutcome<Value>, String> {
    // Stats is a read-only query but it runs through the writer connection
    // rather than the pool: the schema is tiny and we don't want to add a
    // pool-aware helper here just for three count()s. If this ever turns
    // into a hot path, switch it to `IndexReader::with_read_conn`.
    let conn = idx.writer.clone();
    let stats = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let guard = conn.blocking_lock();
        let total: i64 = guard
            .query_row("SELECT count(*) FROM items", [], |r| r.get(0))
            .map_err(|e| format!("count items: {e}"))?;

        let mut by_source: Vec<Value> = Vec::new();
        let mut stmt = guard
            .prepare("SELECT source, count(*) FROM items GROUP BY source ORDER BY source")
            .map_err(|e| format!("prepare by_source: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| format!("query by_source: {e}"))?;
        for r in rows {
            let (source, count) = r.map_err(|e| format!("row by_source: {e}"))?;
            by_source.push(json!({ "source": source, "count": count }));
        }

        let last_ts: Option<i64> = guard
            .query_row("SELECT max(ts) FROM items", [], |r| {
                r.get::<_, Option<i64>>(0)
            })
            .map_err(|e| format!("max ts: {e}"))?;

        Ok(json!({
            "total_items": total,
            "by_source": by_source,
            "last_ingest_ts": last_ts,
        }))
    })
    .await
    .map_err(|e| format!("get_stats task panicked: {e}"))??;

    Ok(RpcOutcome::new(stats, vec![]))
}

/// Embeds the query, runs hybrid search, and returns hits trimmed to a
/// flat shape matching the `search` controller schema.
pub async fn handle_search(
    idx: &PersonalIndex,
    embedder: &Arc<dyn Embedder>,
    text: String,
    k: usize,
) -> Result<RpcOutcome<Value>, String> {
    if text.trim().is_empty() {
        return Err("search text must not be empty".into());
    }
    let k = k.clamp(1, 100);

    // The sqlite-vec column is fixed-width; reject mismatched embedders up
    // front with a clear RPC error instead of a low-level sqlite-vec failure.
    const INDEX_VECTOR_DIM: usize = 1536;
    if embedder.dim() != INDEX_VECTOR_DIM {
        return Err(format!(
            "embedder dim {} does not match index dim {INDEX_VECTOR_DIM}",
            embedder.dim()
        ));
    }

    let mut vecs = embedder
        .embed_batch(&[text.as_str()])
        .await
        .map_err(|e| format!("embed: {e}"))?;
    let query_vec = vecs.pop().ok_or("embedder returned no vectors")?;
    if query_vec.len() != INDEX_VECTOR_DIM {
        return Err(format!(
            "embedding length {} does not match index dim {INDEX_VECTOR_DIM}",
            query_vec.len()
        ));
    }

    let reader = IndexReader::new(idx);
    let q = Query::simple(text, k);
    let hits = reader
        .hybrid_search(&q, &query_vec)
        .await
        .map_err(|e| format!("hybrid_search: {e}"))?;

    let payload: Vec<Value> = hits
        .into_iter()
        .map(|h| {
            json!({
                "item_id": h.item.id.to_string(),
                "score": h.score,
                "snippet": h.snippet,
                "source": serde_json::to_value(h.item.source).unwrap_or(Value::Null),
                "subject": h.item.subject,
                "ts": h.item.ts.timestamp(),
            })
        })
        .collect();

    Ok(RpcOutcome::new(json!({ "hits": payload }), vec![]))
}

/// Ingest a single item: upsert-by-(source, external_id), embed text, replace
/// vector atomically. Returns the canonical item_id plus `replaced: bool` so
/// callers can tell insert from update.
pub async fn handle_ingest(
    idx: &PersonalIndex,
    embedder: &Arc<dyn Embedder>,
    source: Source,
    external_id: String,
    ts: i64,
    subject: Option<String>,
    text: String,
    metadata: Value,
) -> Result<RpcOutcome<Value>, String> {
    if external_id.trim().is_empty() {
        return Err("external_id must not be empty".into());
    }
    if text.trim().is_empty() {
        return Err("text must not be empty".into());
    }
    let ts = DateTime::<Utc>::from_timestamp(ts, 0)
        .ok_or_else(|| format!("invalid ts (out of range): {ts}"))?;

    let requested_id = uuid::Uuid::new_v4();
    let mut items = vec![Item {
        id: requested_id,
        source,
        external_id,
        ts,
        author: None,
        subject,
        text: text.clone(),
        metadata,
    }];

    let writer = IndexWriter::new(idx);
    writer
        .upsert(&mut items)
        .await
        .map_err(|e| format!("upsert: {e}"))?;
    let canonical_id = items[0].id;
    let replaced = canonical_id != requested_id;

    let mut vecs = embedder
        .embed_batch(&[text.as_str()])
        .await
        .map_err(|e| format!("embed: {e}"))?;
    let vector = vecs.pop().ok_or("embedder returned no vectors")?;
    writer
        .upsert_vector(&canonical_id, &vector)
        .await
        .map_err(|e| format!("upsert_vector: {e}"))?;

    Ok(RpcOutcome::new(
        json!({
            "item_id": canonical_id.to_string(),
            "replaced": replaced,
        }),
        vec![],
    ))
}
