//! Web/desktop channel host.
//!
//! A thin host over two shared engines. TinyAgents' turn runner owns the
//! lifecycle of a chat turn (ingress order, per-thread serialization, the
//! queue modes, cancellation, the idle and wall-clock guards, reply
//! persistence, one terminal event), and TinyChannels' compiled web module owns
//! the projection of a turn's output onto the web wire. This module is what
//! only OpenHuman can supply: the ports the runner calls (`ingress.rs`,
//! `turn_host.rs`, `turn_events.rs`, state in `runner.rs`), the session
//! checkout and fingerprint (`session.rs`), the single progress projection and
//! its ledger, turn-state and tracing side effects (`progress*.rs`), reply
//! delivery and persistence (`presentation.rs`, `reply_persistence.rs`), error
//! copy (`web_errors*`), the surface subscribers on the domain bus
//! (`event_bus.rs`) and the `channel.web_*` RPC (`schemas.rs`). See `README.md`
//! for the lifecycle and wiring.
//!
//! The wire types are `tinychannels-bus`'s. OpenHuman adds only a host-private
//! routing envelope around those typed events for its process-wide bus.

mod channel_event;
pub(crate) use channel_event::event_context;
pub(crate) mod citations;
mod egress_surface;
mod event_bus;
mod event_bus_channel;
pub(crate) mod failure_details;
mod ingress;
mod ops;
pub(crate) mod proactive;
// Response delivery/segmentation for the web surface (folded in from the former
// standalone `presentation` provider — it is the web channel's delivery formatter).
pub mod presentation;
pub(crate) mod progress;
mod progress_ledger;
mod progress_map;
mod reply_persistence;
pub(crate) use reply_persistence::persist_delivered_reply;
mod runner;
mod schemas;
mod session;
mod suggestions;
mod surface;
pub(crate) use surface::{Surface, TurnIds};
mod turn_activity;
mod turn_close;
mod turn_events;
pub(crate) mod turn_guards;
mod turn_host;
mod types;

mod voice_notes;
mod web_errors;
pub(crate) use web_errors::{
    classify_inference_error, turn_idle_error_message, turn_timeout_error_message,
};
#[cfg(any(test, debug_assertions))]
#[allow(unused_imports)]
pub(crate) use web_errors::{
    generic_inference_error_user_message, is_action_budget_exhausted,
    is_non_retryable_rate_limit_text, retry_after_hint, ClassifiedError,
};

// Public API — event bus
pub use channel_event::{
    ChatSuggestion, GuardrailPayload, GuardrailReason, QueueItemPayload, ScopedWebEvent,
    SubagentProgressDetail, SubagentUsagePayload, TurnTimingPayload, TurnUsagePayload,
    WebChannelEvent, WebEventContext,
};
pub use citations::{TurnCitation, TURN_CITATION_SNIPPET_CHARS};
pub use egress_surface::register_egress_surface_subscriber;
pub use event_bus::{
    approval_request_event, plan_review_request_event, publish_web_channel_event,
    publish_web_channel_event_in, register_agent_surface_subscriber,
    register_approval_surface_subscriber, register_artifact_surface_subscriber,
    register_memory_activity_surface_subscriber, subscribe_web_channel_events,
};
pub use progress::unix_epoch_ms;

// Test-only: OnceLock-bypassing approval bridge for per-runtime integration tests.
// Compiled only in debug builds so it cannot be linked into a release binary.
#[cfg(debug_assertions)]
pub use event_bus::fresh_approval_surface_subscription;

// Public API — operations
pub(crate) use ops::channel_web_chat_with_stored_attachments;
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
#[cfg(test)]
pub(crate) use ops::{last_run_chat_task_error_for_test, set_last_run_chat_task_error_for_test};
pub use types::ChatRequestMetadata;

// Public API — schemas / controllers
pub use schemas::{
    all_web_channel_controller_schemas, all_web_channel_registered_controllers, schemas,
};

// Helpers re-exported for tests
pub(crate) use ops::{event_session_id_for, key_for};
#[cfg(test)]
pub(crate) use ops::{key_in, unscope_in};
#[cfg(feature = "flows")]
pub(crate) use progress::spawn_progress_bridge;
#[cfg(any(test, debug_assertions))]
#[allow(unused_imports)]
pub(crate) use turn_events::sentry_suppression_reason;

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
