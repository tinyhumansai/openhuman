//! Error taxonomy for the conversation threads RPC surface.

use serde_json::json;

use crate::rpc::StructuredRpcError;

/// Stable JSON-RPC discriminator used by the frontend to handle stale thread
/// references without string matching or user-facing error toasts.
pub const THREAD_NOT_FOUND_KIND: &str = "ThreadNotFound";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ThreadsError {
    #[error("thread {thread_id} not found")]
    NotFound { thread_id: String },
    #[error("{0}")]
    Message(String),
}

impl ThreadsError {
    pub fn not_found(thread_id: impl Into<String>) -> Self {
        Self::NotFound {
            thread_id: thread_id.into(),
        }
    }

    pub fn from_thread_scoped_store_error(thread_id: &str, err: String) -> Self {
        // Only promote to `NotFound` when the parsed id matches the requested
        // `thread_id`. If the store reports a *different* missing id we return
        // `Message` so the caller sees the real store error rather than
        // clearing the wrong stale thread on the frontend.
        match parse_thread_not_found_message(&err) {
            Some(parsed_id) if parsed_id == thread_id => Self::not_found(thread_id),
            _ => Self::Message(err),
        }
    }

    /// Builds the structured RPC envelope that the JSON-RPC boundary will
    /// surface to the frontend. The transport layer decodes this without
    /// any domain-specific branching — it just sees a generic typed error.
    fn to_structured_rpc_error(&self) -> Option<StructuredRpcError> {
        match self {
            Self::NotFound { thread_id } => Some(StructuredRpcError {
                message: self.to_string(),
                data: Some(json!({
                    "kind": THREAD_NOT_FOUND_KIND,
                    "thread_id": thread_id,
                })),
                // `ThreadNotFound` is a routine user-state condition (the UI
                // is holding a stale reference after a delete / purge). The
                // boundary must NOT forward this to Sentry — it's noise, not
                // an internal failure.
                expected_user_state: true,
            }),
            Self::Message(_) => None,
        }
    }
}

impl From<String> for ThreadsError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

impl From<&str> for ThreadsError {
    fn from(value: &str) -> Self {
        Self::Message(value.to_string())
    }
}

impl From<ThreadsError> for String {
    /// Conversion at the controller-handler boundary: structured variants
    /// (currently `NotFound`) emit a [`StructuredRpcError`] envelope encoded
    /// as a sentinel-prefixed string; plain variants degrade to `Display`.
    ///
    /// This is the ONLY place where `ThreadNotFound` becomes a wire-shaped
    /// RPC error. The JSON-RPC transport layer never inspects the method
    /// name to recover that shape.
    fn from(value: ThreadsError) -> Self {
        if let Some(structured) = value.to_structured_rpc_error() {
            structured.encode()
        } else {
            value.to_string()
        }
    }
}

/// Parse the canonical display form (`thread <id> not found`) and the legacy
/// store form (`thread <id> does not exist`) so this module can classify
/// store-layer errors into a typed `NotFound` variant. Private — the JSON-RPC
/// transport layer no longer string-sniffs error messages.
fn parse_thread_not_found_message(message: &str) -> Option<&str> {
    let thread_id = message
        .strip_prefix("thread ")
        .and_then(|rest| rest.strip_suffix(" not found"))
        .or_else(|| {
            message
                .strip_prefix("thread ")
                .and_then(|rest| rest.strip_suffix(" does not exist"))
        })?;
    if thread_id.trim().is_empty() {
        None
    } else {
        Some(thread_id)
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
