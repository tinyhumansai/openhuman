//! Web/desktop channel turn runner.
//!
//! Drives a chat turn from a Socket.IO `chat:start` event (or an inbound
//! `channels/` provider message) through the agent harness to a delivered,
//! durably-persisted reply: `start_chat` (`ops/start_chat.rs`) validates and
//! preprocesses the message, applies the queue mode against the thread's
//! in-flight state, and spawns `run_task::run_chat_task` under the cancel
//! token / wall-clock backstop / `APPROVAL_CHAT_CONTEXT` scope
//! (`run_turn_under_cancel_and_deadline`, `ops/turn_guards.rs`). `run_chat_task`
//! resolves or builds the cached session `Agent` (`session.rs`), spawns
//! `spawn_progress_bridge` alongside the run, and returns the result; the
//! spawning task then hands an `Ok` to `presentation::deliver_response`,
//! which persists the reply (`reply_persistence.rs`) before announcing it,
//! and an `Err` to `web_errors::classify_inference_error`. Exposes the
//! `channel.web_*` RPC namespace (`schemas.rs`) and the `WebChannelEvent`
//! broadcast bus (`event_bus.rs`) that both Socket.IO and the JSON-RPC
//! `/events` SSE stream consume. See `README.md` for the full lifecycle and
//! wiring.
//!
//! Submodules: `ops*.rs` (request surface, session/in-flight state),
//! `run_task.rs` (turn execution), `session.rs` (agent build/fingerprint),
//! `progress_bridge.rs` (progress → socket event bridge), `presentation.rs`
//! (reply delivery), `reply_persistence.rs` (durable reply write),
//! `event_bus.rs` (broadcast + `DomainEvent` surface subscribers),
//! `web_errors*.rs` (provider error classification), `schemas.rs` (RPC
//! contract), `types.rs` (shared param/state types).

mod event_bus;
mod ops;
// Response delivery/segmentation for the web surface (folded in from the former
// standalone `presentation` provider — it is the web channel's delivery formatter).
pub mod presentation;
mod progress_bridge;
mod reply_persistence;
mod run_task;
mod schemas;
mod session;
mod types;

mod web_errors;
pub(crate) use web_errors::classify_inference_error;
#[cfg(any(test, debug_assertions))]
#[allow(unused_imports)]
pub(crate) use web_errors::{
    extract_provider_error_detail, extract_provider_name, generic_inference_error_user_message,
    is_action_budget_exhausted, is_fallback_chain_exhausted, is_non_retryable_rate_limit_text,
    parse_retry_after_secs_from_str, retry_after_hint, with_provider_detail, ClassifiedError,
};

// Public API — event bus
pub use event_bus::{
    approval_request_event, publish_web_channel_event, register_approval_surface_subscriber,
    register_artifact_surface_subscriber, register_egress_surface_subscriber,
    subscribe_web_channel_events,
};

// Test-only: OnceLock-bypassing approval bridge for per-runtime integration tests.
// Compiled only in debug builds so it cannot be linked into a release binary.
#[cfg(debug_assertions)]
pub use event_bus::fresh_approval_surface_subscription;

// Public API — operations
#[cfg(test)]
pub use ops::drain_queued_turns_for_test;
#[cfg(any(test, debug_assertions))]
pub use ops::parallel_in_flight_entries_for_test;
pub use ops::{
    cancel_chat, cancel_chat_scoped, cancel_should_target, channel_web_cancel, channel_web_chat,
    channel_web_queue_clear, channel_web_queue_status, in_flight_entries_for_test,
    invalidate_thread_sessions, start_chat,
};
pub use types::ChatRequestMetadata;

// Public API — schemas / controllers
pub use schemas::{
    all_web_channel_controller_schemas, all_web_channel_registered_controllers, schemas,
};

// Helpers re-exported for tests
#[cfg(any(test, debug_assertions))]
#[allow(unused_imports)]
pub(crate) use ops::sentry_suppression_reason;
pub(crate) use ops::{event_session_id_for, key_for};
pub(crate) use progress_bridge::spawn_progress_bridge;

// Schema field helpers + session/error helpers re-exported for the `web_tests`
// integration module (they moved into submodules during the module split but
// the sibling test file still imports them via `super::`).
#[cfg(any(test, debug_assertions))]
#[allow(unused_imports)]
pub(crate) use schemas::{
    json_output, optional_bool, optional_f64, optional_string, optional_u64, required_string,
};
#[cfg(any(test, debug_assertions))]
#[allow(unused_imports)]
pub(crate) use session::{
    locale_reply_directive, normalize_model_override, provider_role_for_model_override,
};
#[cfg(any(test, debug_assertions))]
#[allow(unused_imports)]
pub(crate) use types::WebChatParams;
#[cfg(any(test, debug_assertions))]
#[allow(unused_imports)]
pub(crate) use web_errors::{
    inference_budget_exceeded_user_message, is_inference_budget_exceeded_error,
};

// Test helpers (debug/test builds only)
#[cfg(any(test, debug_assertions))]
pub use ops::set_test_forced_run_chat_task_error;
#[cfg(any(test, debug_assertions))]
pub use ops::RUN_CHAT_TASK_TEST_LOCK;
#[cfg(any(test, debug_assertions))]
pub use ops::{set_test_run_chat_task_block, TestRunChatTaskBlock};

#[cfg(any(test, debug_assertions))]
#[path = "mod_test_support_tests.rs"]
pub mod test_support;

#[cfg(test)]
pub(crate) use types::SessionCacheFingerprint;

#[cfg(test)]
#[path = "web_tests.rs"]
mod tests;
