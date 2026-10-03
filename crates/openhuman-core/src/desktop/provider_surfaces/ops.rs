//! Core operations for provider assistive surfaces.
//!
//! This initial cut keeps state in-memory so the RPC contract and UI wiring
//! can land before the SQLite-backed store arrives.

use crate::core::envelope::{ApiEnvelope, EmptyRequest};
use crate::core::Outcome;
use serde::Serialize;
use std::collections::BTreeMap;

use super::store;
use super::types::{ProviderEvent, RespondQueueItem, RespondQueueListResponse};

use crate::core::envelope::{counts, envelope as shared_envelope};

fn envelope<T: Serialize>(
    data: T,
    counts: Option<BTreeMap<String, usize>>,
) -> Outcome<ApiEnvelope<T>> {
    shared_envelope(data, counts, None)
}

pub async fn ingest_event(
    request: ProviderEvent,
) -> Result<Outcome<ApiEnvelope<RespondQueueItem>>, String> {
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
) -> Result<Outcome<ApiEnvelope<RespondQueueListResponse>>, String> {
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
