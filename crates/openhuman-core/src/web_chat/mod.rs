//! Web/desktop channel turn runner.
//!
//! Drives a chat turn from a Socket.IO `chat:start` event (or an inbound
//! `channels/` provider message) through the agent harness to a delivered,
//! durably-persisted reply: `start_chat` (`ops/start_chat.rs`) validates and
//! preprocesses the message, applies the queue mode against the thread's
//! in-flight state, and spawns `run_task::run_chat_task` under the cancel
//! token / idle guard / wall-clock backstop / approval-context hand-off
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
//! `progress_bridge.rs` (progress → socket event bridge), `turn_activity.rs`
//! (per-turn inactivity record behind the idle guard), `presentation.rs`
//! (reply delivery), `reply_persistence.rs` (durable reply write),
//! `event_bus.rs` (broadcast + `DomainEvent` surface subscribers),
//! `web_errors*.rs` (provider error classification), `schemas.rs` (RPC
//! contract), `types.rs` (shared param/state types).

mod channel_event;
mod egress_surface;
mod event_bus;
mod event_bus_channel;
mod journal_shadow;
mod ops;
// Response delivery/segmentation for the web surface (folded in from the former
// standalone `presentation` provider — it is the web channel's delivery formatter).
pub mod presentation;
pub(crate) mod progress_bridge;
mod reply_persistence;
pub(crate) use reply_persistence::persist_delivered_reply;
mod run_task;
mod schemas;
mod session;
mod suggestions;
mod turn_activity;
mod turn_close;
mod turn_timing;
mod types;

mod web_errors;
pub(crate) use web_errors::classify_inference_error;
#[cfg(any(test, debug_assertions))]
#[allow(unused_imports)]
pub(crate) use web_errors::{
    generic_inference_error_user_message, is_action_budget_exhausted,
    is_non_retryable_rate_limit_text, retry_after_hint, ClassifiedError,
};

// Public API — event bus
pub use channel_event::{
    ChatSuggestion, GuardrailPayload, GuardrailReason, QueueItemPayload, SubagentProgressDetail,
    SubagentUsagePayload, TurnTimingPayload, TurnUsagePayload, WebChannelEvent,
};
pub use egress_surface::register_egress_surface_subscriber;
pub use event_bus::{
    approval_request_event, plan_review_request_event, publish_web_channel_event,
    publish_web_channel_event_in, register_agent_surface_subscriber,
    register_approval_surface_subscriber, register_artifact_surface_subscriber,
    register_memory_activity_surface_subscriber, subscribe_web_channel_events,
};
pub use progress_bridge::unix_epoch_ms;

// Test-only: OnceLock-bypassing approval bridge for per-runtime integration tests.
// Compiled only in debug builds so it cannot be linked into a release binary.
#[cfg(debug_assertions)]
pub use event_bus::fresh_approval_surface_subscription;

// Public API — operations
#[cfg(test)]
pub use ops::drain_queued_turns_for_test;
pub(crate) use ops::invalidate_thread_sessions_for_tenant;
#[cfg(test)]
pub(crate) use ops::lock_thread_sessions_for_test;
#[cfg(any(test, debug_assertions))]
pub use ops::parallel_in_flight_entries_for_test;
#[cfg(test)]
pub(crate) use ops::track_parallel_turn_for_test;
pub use ops::{
    cancel_all_turns, cancel_chat_scoped, cancel_should_target, channel_web_cancel,
    channel_web_chat, channel_web_queue_clear, channel_web_queue_remove, channel_web_queue_status,
    in_flight_entries_for_test, invalidate_thread_sessions, run_system_turn_on_thread, start_chat,
    StartChatError, SESSION_CHECKOUT_FAILURE, SYSTEM_CLIENT_ID,
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
#[cfg(test)]
pub(crate) use ops::{key_in, unscope_in};
#[cfg(feature = "flows")]
pub(crate) use progress_bridge::spawn_progress_bridge;

// Schema field helpers + session/error helpers re-exported for the `web_tests`
// integration module (they moved into submodules during the module split but
// the sibling test file still imports them via `super::`).
#[cfg(any(test, debug_assertions))]
#[allow(unused_imports)]
pub(crate) use schemas::{
    json_output, optional_bool, optional_f64, optional_string, optional_u64, required_string,
};
// The cron scheduler routes origin deliveries through this in every build.
pub(crate) use session::pick_target_agent_id;
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
pub use ops::{set_test_park_after_run, set_test_run_chat_task_block, TestRunChatTaskBlock};

#[cfg(any(test, debug_assertions))]
#[path = "mod_test_support_tests.rs"]
pub mod test_support;

#[cfg(test)]
pub(crate) use types::SessionCacheFingerprint;

#[cfg(test)]
#[path = "web_tests.rs"]
mod tests;
