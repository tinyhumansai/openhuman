//! Ambient chat-thread id for the current async scope.
//!
//! The web channel keys runtime sessions by `(client_id, thread_id)` and the
//! backend's `/openai/v1/chat/completions` endpoint accepts an optional
//! `thread_id` field so it can group inference logs and align KV-cache keys
//! with the same logical chat the user sees on screen.
//!
//! Threading the identifier through every layer (`Agent` → tool loop →
//! sub-agent runner → `Provider` impl) would touch dozens of call sites and
//! tests. Instead the channel sets a [`tokio::task_local`] before invoking the
//! agent loop, and the OpenAI-compatible provider reads it when serialising the
//! request body. Other call paths see `None` and omit the field — backward
//! compatible with backends that do not accept it.
//!
//! ```ignore
//! use crate::openhuman::agent::tinyagents::thread_context::{current_thread_id, with_thread_id};
//!
//! with_thread_id("abc123", async {
//!     // any provider.chat() call inside this future sees thread_id=Some("abc123")
//!     assert_eq!(current_thread_id().as_deref(), Some("abc123"));
//! }).await;
//! ```
//!
//! # Why the definition is here again (#5560)
//!
//! It was `pub use tinymemory_core::thread_context::{…}` — the task-local was
//! defined in the memory engine because the engine's own recall read it, and
//! two `tokio::task_local!` invocations are two distinct keys. The previous
//! revision of these docs said the move back "comes with the engine's removal,
//! in the same commit — not before it", because leaving the engine's key unset
//! would make its self-echo exclusion fail **open**: recall would start echoing
//! the caller's own thread back at it, with no error anywhere.
//!
//! That condition is met, and by a stronger route than the docs anticipated.
//! The engine reads this task-local in exactly one place —
//! `store::recall_policy::current_self_echo_exclusion`, reached only from
//! `impl Memory for UnifiedMemory`'s `recall`, and only as the fallback when
//! the caller passed no `RecallOpts::exclude_session_id`. Nothing in this
//! binary constructs a `UnifiedMemory` any more: `memory::binding` refuses
//! `DriverClass::Embedded` outright, `memory::mod`'s alias list dropped
//! `MemoryClient` / `UnifiedMemory` for want of consumers, and
//! `tinymemory_core::global::init` survives in `memory::ops::test_support`
//! alone. So the engine-side read is already unreachable in a production build
//! — and where it *is* reachable, from a test that boots an in-process client,
//! that test names the engine crate directly and gets the engine's own key.
//!
//! The host's own call sites are unaffected either way: every one of them
//! reaches this path (`agent::tinyagents::thread_context`) for both `with_` and
//! `current_`, so the scope they set is the scope they read. A caller that
//! needs the exclusion applied by the *module* driver must still pass
//! `RecallOpts::exclude_session_id` explicitly — a task-local set in this
//! process is invisible inside a separately compiled `cdylib`, which is why
//! `agent::tinyagents::host::agent_memory` and `agent::harness::memory_context`
//! already do.

use std::future::Future;

tokio::task_local! {
    static THREAD_ID: Option<String>;
}

/// Run `fut` with the given `thread_id` available to any descendant task that
/// calls [`current_thread_id`]. Empty / whitespace-only ids are normalised to
/// `None` so callers can pass user input through without guarding for it.
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
        "[thread-context] entering scope has_thread_id={}",
        value.is_some()
    );
    THREAD_ID.scope(value, fut).await
}

/// Return the ambient `thread_id` set by an enclosing [`with_thread_id`] scope,
/// or `None` when called outside one (tests, CLI, sub-systems that do not
/// participate in chat sessions).
pub fn current_thread_id() -> Option<String> {
    THREAD_ID
        .try_with(|v| v.clone())
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
#[path = "thread_context_tests.rs"]
mod tests;
