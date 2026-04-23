//! Domain RPC handlers for chronicle. Adapters in `schemas.rs` deserialise
//! params and call these; tests invoke them directly with a
//! `PersonalIndex`.

use serde_json::{json, Value};

use crate::openhuman::life_capture::chronicle::tables;
use crate::openhuman::life_capture::index::PersonalIndex;
use crate::rpc::RpcOutcome;

/// List recent chronicle events, newest first. `limit` is clamped to a sane
/// upper bound so an over-eager caller can't drag the whole table into RAM.
pub async fn handle_list_recent(
    idx: &PersonalIndex,
    limit: u32,
) -> Result<RpcOutcome<Value>, String> {
    let clamped = limit.clamp(1, 1000) as i64;
    let rows = tables::list_recent(idx, clamped)
        .await
        .map_err(|e| format!("list_recent: {e}"))?;

    let events: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.id,
                "ts_ms": r.ts_ms,
                "focused_app": r.focused_app,
                "focused_element": r.focused_element,
                "visible_text": r.visible_text,
                "url": r.url,
            })
        })
        .collect();
    Ok(RpcOutcome::new(json!({ "events": events }), vec![]))
}

pub async fn handle_get_watermark(
    idx: &PersonalIndex,
    source: String,
) -> Result<RpcOutcome<Value>, String> {
    if source.trim().is_empty() {
        return Err("source must not be empty".into());
    }
    let last = tables::get_watermark(idx, source)
        .await
        .map_err(|e| format!("get_watermark: {e}"))?;
    Ok(RpcOutcome::new(json!({ "last_ts_ms": last }), vec![]))
}

pub async fn handle_set_watermark(
    idx: &PersonalIndex,
    source: String,
    ts_ms: i64,
) -> Result<RpcOutcome<Value>, String> {
    if source.trim().is_empty() {
        return Err("source must not be empty".into());
    }
    tables::set_watermark(idx, source, ts_ms)
        .await
        .map_err(|e| format!("set_watermark: {e}"))?;
    Ok(RpcOutcome::new(json!({ "ok": true }), vec![]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::life_capture::chronicle::parser::ChronicleEvent;

    fn event(app: &str, ts: i64) -> ChronicleEvent {
        ChronicleEvent {
            focused_app: app.into(),
            focused_element: None,
            visible_text: None,
            url: None,
            ts_ms: ts,
        }
    }

    #[tokio::test]
    async fn list_recent_returns_events_shape() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        tables::insert_event(&idx, event("a", 100)).await.unwrap();
        tables::insert_event(&idx, event("b", 200)).await.unwrap();

        let out = handle_list_recent(&idx, 10).await.unwrap();
        let v = out.value;
        let events = v.get("events").unwrap().as_array().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].get("focused_app").unwrap(), "b");
    }

    #[tokio::test]
    async fn watermark_get_and_set_round_trip() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();

        let initial = handle_get_watermark(&idx, "focus".into()).await.unwrap();
        assert_eq!(initial.value.get("last_ts_ms").unwrap(), &Value::Null);

        handle_set_watermark(&idx, "focus".into(), 42)
            .await
            .unwrap();
        let after = handle_get_watermark(&idx, "focus".into()).await.unwrap();
        assert_eq!(after.value.get("last_ts_ms").unwrap(), 42);
    }

    #[tokio::test]
    async fn empty_source_rejected() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        assert!(handle_get_watermark(&idx, "".into()).await.is_err());
        assert!(handle_set_watermark(&idx, "   ".into(), 1).await.is_err());
    }
}
