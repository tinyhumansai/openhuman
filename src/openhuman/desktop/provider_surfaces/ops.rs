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
#[path = "ops_tests.rs"]
mod tests;
