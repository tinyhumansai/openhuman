//! Domain RPC handlers for life-capture. Adapter handlers in `schemas.rs`
//! deserialise params and call into these functions; tests can call them
//! directly with a constructed `LifeCaptureRuntime`.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::openhuman::life_capture::embedder::Embedder;
use crate::openhuman::life_capture::index::{IndexReader, PersonalIndex};
use crate::openhuman::life_capture::types::Query;
use crate::rpc::RpcOutcome;

/// Returns total item count, per-source counts, and the most recent item ts
/// (unix seconds, or null when the index is empty).
pub async fn handle_get_stats(idx: &PersonalIndex) -> Result<RpcOutcome<Value>, String> {
    let conn = idx.conn.clone();
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
            .query_row("SELECT max(ts) FROM items", [], |r| r.get::<_, Option<i64>>(0))
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

    let mut vecs = embedder
        .embed_batch(&[text.as_str()])
        .await
        .map_err(|e| format!("embed: {e}"))?;
    let query_vec = vecs.pop().ok_or("embedder returned no vectors")?;

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

    Ok(RpcOutcome::new(Value::Array(payload), vec![]))
}
