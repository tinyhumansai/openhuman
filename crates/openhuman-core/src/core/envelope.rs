//! The `{data, error, meta}` response envelope a few JSON-RPC surfaces share
//! (`threads`, desktop provider surfaces), and the helpers that build it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::Outcome;

/// Standard error structure for API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    /// A machine-readable error code.
    pub code: String,
    /// A human-readable error message.
    pub message: String,
    /// Optional additional error details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Pagination metadata for list-based responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationMeta {
    /// Maximum number of items requested.
    pub limit: usize,
    /// Number of items skipped.
    pub offset: usize,
    /// Total number of items available in the backend.
    pub count: usize,
}

/// General metadata included in all API envelopes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMeta {
    /// Unique identifier for the request.
    pub request_id: String,
    /// Time taken to process the request in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_seconds: Option<f64>,
    /// Whether the response was served from a cache.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached: Option<bool>,
    /// Optional counts of various items (e.g., by category).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counts: Option<BTreeMap<String, usize>>,
    /// Optional pagination information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination: Option<PaginationMeta>,
}

/// Generic envelope for all API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEnvelope<T> {
    /// The actual payload of the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    /// Error information if the request failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
    /// Metadata about the request and response.
    pub meta: ApiMeta,
}

/// An empty request body for methods that don't require parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyRequest {}

/// A fresh request id for an envelope's `meta`.
pub(crate) fn request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Named counts for an envelope's `meta.counts`.
pub(crate) fn counts(
    entries: impl IntoIterator<Item = (&'static str, usize)>,
) -> BTreeMap<String, usize> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

/// Wraps `data` in a successful envelope.
pub(crate) fn envelope<T: Serialize>(
    data: T,
    counts: Option<BTreeMap<String, usize>>,
    pagination: Option<PaginationMeta>,
) -> Outcome<ApiEnvelope<T>> {
    Outcome::new(
        ApiEnvelope {
            data: Some(data),
            error: None,
            meta: ApiMeta {
                request_id: request_id(),
                latency_seconds: None,
                cached: None,
                counts,
                pagination,
            },
        },
        vec![],
    )
}

#[cfg(test)]
#[path = "envelope_tests.rs"]
mod tests;
