//! Domain RPC handlers for life-capture. Adapter handlers in `schemas.rs`
//! deserialise params and call into these functions; tests can call them
//! directly with a constructed `LifeCaptureRuntime`.

use std::sync::Arc;

use serde_json::{json, Value};
use tracing::{debug, warn};

use crate::openhuman::life_capture::embedder::Embedder;
use crate::openhuman::life_capture::index::{IndexReader, PersonalIndex};
use crate::openhuman::life_capture::types::Query;
use crate::rpc::RpcOutcome;

/// Returns total item count, per-source counts, and the most recent item ts
/// (unix seconds, or null when the index is empty).
pub async fn handle_get_stats(idx: &PersonalIndex) -> Result<RpcOutcome<Value>, String> {
    debug!("[life_capture] handle_get_stats: entry");
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
    .map_err(|e| {
        warn!("[life_capture] handle_get_stats: task panicked: {e}");
        format!("get_stats task panicked: {e}")
    })??;

    debug!(
        "[life_capture] handle_get_stats: total_items={} by_source_count={}",
        stats
            .get("total_items")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        stats
            .get("by_source")
            .and_then(|v| v.as_array())
            .map_or(0, |a| a.len()),
    );
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
    debug!(
        "[life_capture] handle_search: entry text_len={} k={}",
        text.len(),
        k
    );

    if text.trim().is_empty() {
        return Err("search text must not be empty".into());
    }
    let k = k.clamp(1, 100);

    // The sqlite-vec column is fixed-width; reject mismatched embedders up
    // front with a clear RPC error instead of a low-level sqlite-vec failure.
    const INDEX_VECTOR_DIM: usize = 1536;
    if embedder.dim() != INDEX_VECTOR_DIM {
        warn!(
            "[life_capture] handle_search: embedder dim {} != index dim {INDEX_VECTOR_DIM}",
            embedder.dim()
        );
        return Err(format!(
            "embedder dim {} does not match index dim {INDEX_VECTOR_DIM}",
            embedder.dim()
        ));
    }

    let mut vecs = embedder.embed_batch(&[text.as_str()]).await.map_err(|e| {
        warn!("[life_capture] handle_search: embed_batch failed: {e}");
        format!("embed: {e}")
    })?;
    let query_vec = vecs.pop().ok_or_else(|| {
        warn!("[life_capture] handle_search: embedder returned no vectors");
        "embedder returned no vectors"
    })?;
    if query_vec.len() != INDEX_VECTOR_DIM {
        warn!(
            "[life_capture] handle_search: embedding len {} != index dim {INDEX_VECTOR_DIM}",
            query_vec.len()
        );
        return Err(format!(
            "embedding length {} does not match index dim {INDEX_VECTOR_DIM}",
            query_vec.len()
        ));
    }

    let reader = IndexReader::new(idx);
    let q = Query::simple(text, k);
    let hits = reader.hybrid_search(&q, &query_vec).await.map_err(|e| {
        warn!("[life_capture] handle_search: hybrid_search failed: {e}");
        format!("hybrid_search: {e}")
    })?;

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

    debug!(
        "[life_capture] handle_search: {} hits returned",
        payload.len()
    );
    Ok(RpcOutcome::new(json!({ "hits": payload }), vec![]))
}
