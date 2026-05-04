//! Response envelope helpers shared across memory RPC handlers.
//!
//! These helpers standardise the `ApiEnvelope`/`ApiError` wrapping used by the
//! envelope-style memory RPC methods (init, list_documents, query_namespace,
//! recall_*, ai_*_memory_file).

use std::collections::BTreeMap;

use serde::Serialize;

use crate::openhuman::memory::{ApiEnvelope, ApiError, ApiMeta, PaginationMeta};
use crate::rpc::RpcOutcome;

/// Generates a unique request ID for memory operations.
///
/// This ID is used for tracing and logging purposes in the API response metadata.
pub(crate) fn memory_request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Converts an iterator of memory counts into a BTreeMap.
///
/// This is a convenience helper for populating the `counts` field in the API metadata.
pub(crate) fn memory_counts(
    entries: impl IntoIterator<Item = (&'static str, usize)>,
) -> BTreeMap<String, usize> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

/// Wraps data in an RPC API envelope.
///
/// This standardises the response format for memory-related RPC methods.
pub(crate) fn envelope<T: Serialize>(
    data: T,
    counts: Option<BTreeMap<String, usize>>,
    pagination: Option<PaginationMeta>,
) -> RpcOutcome<ApiEnvelope<T>> {
    RpcOutcome::new(
        ApiEnvelope {
            data: Some(data),
            error: None,
            meta: ApiMeta {
                request_id: memory_request_id(),
                latency_seconds: None,
                cached: None,
                counts,
                pagination,
            },
        },
        vec![],
    )
}

/// Wraps an error in an RPC API envelope.
///
/// This provides a consistent error reporting format for the memory system.
pub(crate) fn error_envelope<T: Serialize>(
    code: &str,
    message: String,
) -> RpcOutcome<ApiEnvelope<T>> {
    RpcOutcome::new(
        ApiEnvelope {
            data: None,
            error: Some(ApiError {
                code: code.to_string(),
                message,
                details: None,
            }),
            meta: ApiMeta {
                request_id: memory_request_id(),
                latency_seconds: None,
                cached: None,
                counts: None,
                pagination: None,
            },
        },
        vec![],
    )
}
