mod event_bus;
mod ops;
mod progress_bridge;
mod run_task;
mod schemas;
mod session;
mod types;

#[path = "../web_errors.rs"]
mod web_errors;
pub(crate) use web_errors::{
    classify_inference_error, inference_budget_exceeded_user_message,
    is_inference_budget_exceeded_error,
};
#[cfg(any(test, debug_assertions))]
#[allow(unused_imports)]
pub(crate) use web_errors::{
    extract_provider_error_detail, extract_provider_name, generic_inference_error_user_message,
    is_action_budget_exhausted, is_fallback_chain_exhausted, is_non_retryable_rate_limit_text,
    parse_retry_after_secs_from_str, retry_after_hint, with_provider_detail, ClassifiedError,
};

// Public API — event bus
pub use event_bus::{
    publish_web_channel_event, register_approval_surface_subscriber,
    register_artifact_surface_subscriber, subscribe_web_channel_events,
};

// Test-only: OnceLock-bypassing approval bridge for per-runtime integration tests.
// Compiled only in debug builds so it cannot be linked into a release binary.
#[cfg(debug_assertions)]
pub use event_bus::fresh_approval_surface_subscription;

// Public API — operations
#[cfg(any(test, debug_assertions))]
pub use ops::parallel_in_flight_entries_for_test;
pub use ops::{
    cancel_chat, channel_web_cancel, channel_web_chat, channel_web_queue_clear,
    channel_web_queue_status, in_flight_entries_for_test, invalidate_thread_sessions, start_chat,
};
pub use types::ChatRequestMetadata;

// Public API — schemas / controllers
pub use schemas::{
    all_web_channel_controller_schemas, all_web_channel_registered_controllers, schemas,
};

// Helpers re-exported for tests
pub(crate) use ops::{event_session_id_for, key_for};
pub(crate) use progress_bridge::spawn_progress_bridge;
pub(crate) use session::{compose_system_prompt_suffix, locale_reply_directive};

// Schema field helpers re-exported for tests
pub(crate) use schemas::{
    json_output, optional_bool, optional_f64, optional_string, optional_u64, required_string,
};

// Test helpers (debug/test builds only)
#[cfg(any(test, debug_assertions))]
pub use ops::set_test_forced_run_chat_task_error;
#[cfg(any(test, debug_assertions))]
pub use ops::{set_test_run_chat_task_block, TestRunChatTaskBlock};

#[cfg(any(test, debug_assertions))]
pub(crate) use ops::THREAD_SESSIONS;
#[cfg(any(test, debug_assertions))]
pub(crate) use session::{normalize_model_override, provider_role_for_model_override};
#[cfg(any(test, debug_assertions))]
pub(crate) use types::WebChatParams;

#[cfg(any(test, debug_assertions))]
pub mod test_support {
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ClassifiedErrorSnapshot {
        pub error_type: &'static str,
        pub message: String,
        pub source: &'static str,
        pub retryable: bool,
        pub retry_after_ms: Option<u64>,
        pub provider: Option<String>,
        pub fallback_available: Option<bool>,
    }

    pub fn classify_error_for_test(err: &str) -> ClassifiedErrorSnapshot {
        let classified = super::classify_inference_error(err);
        ClassifiedErrorSnapshot {
            error_type: classified.error_type,
            message: classified.message,
            source: classified.source,
            retryable: classified.retryable,
            retry_after_ms: classified.retry_after_ms,
            provider: classified.provider,
            fallback_available: classified.fallback_available,
        }
    }

    pub fn extracted_provider_detail_for_test(err: &str) -> Option<String> {
        super::extract_provider_error_detail(err)
    }

    pub fn retry_after_secs_for_test(err: &str) -> Option<u64> {
        super::parse_retry_after_secs_from_str(err)
    }

    pub fn is_non_retryable_rate_limit_for_test(lower: &str) -> bool {
        super::is_non_retryable_rate_limit_text(lower)
    }

    pub fn key_for_test(thread_id: &str) -> String {
        super::key_for(thread_id)
    }

    pub fn event_session_id_for_test(client_id: &str, thread_id: &str) -> String {
        super::event_session_id_for(client_id, thread_id)
    }

    pub async fn set_forced_run_chat_task_error_for_test(message: Option<&str>) {
        super::set_test_forced_run_chat_task_error(message).await;
    }
}

#[cfg(test)]
pub(crate) use types::SessionCacheFingerprint;

#[cfg(test)]
#[path = "../web_tests.rs"]
mod tests;
