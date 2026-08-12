//! Ambient chat-thread id for the current async scope.
//!
//! The definition **moved to `tinymemory_core::thread_context`** during the
//! memory extraction. The memory store is its heaviest consumer — recall has to
//! exclude the thread it is being called from — and the task-local cannot be
//! two different task-locals in two crates or the scope would not be visible
//! across the seam.
//!
//! It could not go in `tinymemory-api`, which must not depend on an async
//! runtime; `tokio::task_local!` needs one. Every existing
//! `agent::tinyagents::thread_context::…` path keeps resolving and keeps
//! naming the same task-local.

pub use tinymemory_core::thread_context::{current_thread_id, with_thread_id};
