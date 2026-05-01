//! Ambient `thread_id` propagation for outbound provider requests.
//!
//! The web channel keys runtime sessions by `(client_id, thread_id)` and the
//! backend's `/openai/v1/chat/completions` endpoint accepts an optional
//! `thread_id` field so it can group inference logs and align KV-cache keys
//! with the same logical chat the user sees on screen.
//!
//! Threading the identifier through every layer (`Agent` → tool loop →
//! sub-agent runner → `Provider` impl) would touch dozens of call sites
//! and tests. Instead, the channel sets a [`tokio::task_local`] before
//! invoking the agent loop, and the OpenAI-compatible provider reads it
//! when serializing the request body. Other call paths see `None` and
//! omit the field — backward-compatible with backends that don't accept
//! it.
//!
//! ```ignore
//! use crate::openhuman::providers::thread_context::{with_thread_id, current_thread_id};
//!
//! with_thread_id("abc123", async {
//!     // any provider.chat() call inside this future sees thread_id=Some("abc123")
//!     assert_eq!(current_thread_id().as_deref(), Some("abc123"));
//! }).await;
//! ```

use std::future::Future;

tokio::task_local! {
    static THREAD_ID: Option<String>;
}

/// Run `fut` with the given `thread_id` available to any descendant task
/// that calls [`current_thread_id`]. Empty / whitespace-only ids are
/// normalized to `None` so callers can pass through user input without
/// guarding for it.
pub async fn with_thread_id<F, T>(thread_id: impl Into<String>, fut: F) -> T
where
    F: Future<Output = T>,
{
    let id = thread_id.into();
    let trimmed = id.trim();
    let value = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    };
    log::debug!(
        "[thread-context] entering scope thread_id={}",
        value.as_deref().unwrap_or("<none>")
    );
    THREAD_ID.scope(value, fut).await
}

/// Return the ambient `thread_id` set by an enclosing [`with_thread_id`]
/// scope, or `None` when called outside one (tests, CLI, sub-systems
/// that don't participate in chat sessions).
pub fn current_thread_id() -> Option<String> {
    THREAD_ID
        .try_with(|v| v.clone())
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn scope_sets_and_clears_thread_id() {
        assert!(current_thread_id().is_none(), "baseline outside scope");
        with_thread_id("thread-123", async {
            assert_eq!(current_thread_id().as_deref(), Some("thread-123"));
        })
        .await;
        assert!(
            current_thread_id().is_none(),
            "thread_id must not leak past scope"
        );
    }

    #[tokio::test]
    async fn empty_or_whitespace_id_normalizes_to_none() {
        with_thread_id("   ", async {
            assert!(current_thread_id().is_none());
        })
        .await;
        with_thread_id("", async {
            assert!(current_thread_id().is_none());
        })
        .await;
    }

    #[tokio::test]
    async fn nested_scope_overrides_outer() {
        with_thread_id("outer", async {
            assert_eq!(current_thread_id().as_deref(), Some("outer"));
            with_thread_id("inner", async {
                assert_eq!(current_thread_id().as_deref(), Some("inner"));
            })
            .await;
            assert_eq!(current_thread_id().as_deref(), Some("outer"));
        })
        .await;
    }

    #[tokio::test]
    async fn spawned_task_inherits_via_explicit_propagation() {
        // tokio::task_local does not propagate across spawn by default.
        // Document the expected pattern: capture before spawning.
        with_thread_id("propagated", async {
            let captured = current_thread_id();
            let handle = tokio::spawn(async move {
                with_thread_id(captured.unwrap_or_default(), async {
                    current_thread_id()
                })
                .await
            });
            let observed = handle.await.unwrap();
            assert_eq!(observed.as_deref(), Some("propagated"));
        })
        .await;
    }
}
