//! Core operations for provider assistive surfaces.
//!
//! This initial cut keeps state in-memory so the RPC contract and UI wiring
//! can land before the SQLite-backed store arrives.

use crate::openhuman::memory::{ApiEnvelope, ApiMeta, EmptyRequest};
use crate::rpc::RpcOutcome;
use serde::Serialize;
use std::collections::BTreeMap;

use super::store;
use super::types::{ProviderEvent, RespondQueueItem, RespondQueueListResponse};

fn request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn counts(entries: impl IntoIterator<Item = (&'static str, usize)>) -> BTreeMap<String, usize> {
    entries
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

fn envelope<T: Serialize>(
    data: T,
    counts: Option<BTreeMap<String, usize>>,
) -> RpcOutcome<ApiEnvelope<T>> {
    RpcOutcome::new(
        ApiEnvelope {
            data: Some(data),
            error: None,
            meta: ApiMeta {
                request_id: request_id(),
                latency_seconds: None,
                cached: None,
                counts,
                pagination: None,
            },
        },
        vec![],
    )
}

pub async fn ingest_event(
    request: ProviderEvent,
) -> Result<RpcOutcome<ApiEnvelope<RespondQueueItem>>, String> {
    tracing::debug!(
        provider = %request.provider,
        account_id = %request.account_id,
        event_kind = %request.event_kind,
        entity_id = %request.entity_id,
        requires_attention = request.requires_attention,
        "[provider-surfaces] ingest_event"
    );
    let item = store::upsert_queue_item(request);
    Ok(envelope(item, Some(counts([("queue_items", 1)]))))
}

pub async fn list_queue(
    _request: EmptyRequest,
) -> Result<RpcOutcome<ApiEnvelope<RespondQueueListResponse>>, String> {
    let items = store::list_queue_items();
    let count = items.len();
    tracing::debug!(count, "[provider-surfaces] list_queue");
    Ok(envelope(
        RespondQueueListResponse { items, count },
        Some(counts([("queue_items", count)])),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serializes tests that mutate the process-global RESPOND_QUEUE so cargo's
    /// default parallel test runner cannot interleave clear/insert/assert cycles.
    static TEST_MUTEX: Mutex<()> = Mutex::new(());

    fn sample_event(entity_id: &str) -> ProviderEvent {
        ProviderEvent {
            provider: "linkedin".into(),
            account_id: "acct-1".into(),
            event_kind: "message".into(),
            entity_id: entity_id.into(),
            thread_id: Some("thread-1".into()),
            title: Some("New message".into()),
            snippet: Some("Can we talk tomorrow?".into()),
            sender_name: Some("Taylor".into()),
            sender_handle: Some("taylor".into()),
            timestamp: "2026-04-22T16:55:00Z".into(),
            deep_link: Some("https://www.linkedin.com/messaging/thread-1".into()),
            requires_attention: true,
            raw_payload: None,
        }
    }

    #[tokio::test]
    async fn ingest_event_upserts_queue_item() {
        let _lock = TEST_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        store::clear_queue();
        let first = ingest_event(sample_event("entity-1")).await.unwrap();
        let second = ingest_event(sample_event("entity-1")).await.unwrap();

        let first_value = first.into_cli_compatible_json().unwrap();
        let second_value = second.into_cli_compatible_json().unwrap();
        let first_result = first_value.get("data").unwrap_or(&first_value);
        let second_result = second_value.get("data").unwrap_or(&second_value);

        assert_eq!(first_result["provider"], "linkedin");
        assert_eq!(second_result["entity_id"], "entity-1");

        let queue = list_queue(EmptyRequest {}).await.unwrap();
        let queue_json = queue.into_cli_compatible_json().unwrap();
        let data = queue_json.get("data").unwrap_or(&queue_json);
        assert_eq!(data["count"], 1);
    }

    #[tokio::test]
    async fn list_queue_returns_newest_first() {
        let _lock = TEST_MUTEX.lock().unwrap_or_else(|p| p.into_inner());
        store::clear_queue();
        ingest_event(sample_event("entity-1")).await.unwrap();
        ingest_event(sample_event("entity-2")).await.unwrap();

        let queue = list_queue(EmptyRequest {}).await.unwrap();
        let queue_json = queue.into_cli_compatible_json().unwrap();
        let data = queue_json.get("data").unwrap_or(&queue_json);
        let items = data["items"].as_array().unwrap();

        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["entity_id"], "entity-2");
        assert_eq!(items[1]["entity_id"], "entity-1");
    }
}
