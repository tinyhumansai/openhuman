use super::{
    all_web_channel_controller_schemas, all_web_channel_registered_controllers, cancel_chat,
    classify_inference_error, compose_system_prompt_suffix, event_session_id_for,
    extract_provider_error_detail, generic_inference_error_user_message,
    in_flight_entries_for_test, inference_budget_exceeded_user_message,
    is_inference_budget_exceeded_error, json_output, key_for, locale_reply_directive,
    normalize_model_override, optional_f64, optional_string, parallel_in_flight_entries_for_test,
    provider_role_for_model_override, required_string, schemas, sentry_suppression_reason,
    set_test_forced_run_chat_task_error, set_test_run_chat_task_block, start_chat,
    subscribe_web_channel_events, ChatRequestMetadata, ClassifiedError, TestRunChatTaskBlock,
    WebChatParams,
};
use crate::core::TypeSchema;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::time::{timeout, Duration};

// Serializes every test that drives `start_chat` with the process-global
// `run_chat_task` test hooks (forced error / forced block) or the
// `OPENHUMAN_WEB_TURN_TIMEOUT_SECS` override. Those toggles and the per-thread
// session cache are process-global, so two such tests running concurrently can
// clobber each other before `run_chat_task` reads them — leading to flaky
// asserts where one test observes another test's state. Holding this mutex for
// the duration of each test body restores isolation without disabling
// `cargo test`'s default parallelism for the rest of the suite.
//
// This is the *shared* lock defined at the hook boundary
// (`web::ops::RUN_CHAT_TASK_TEST_LOCK`), not a file-local one, so tests in other
// modules that exercise `start_chat`/`run_chat_task` serialize on the same lock
// (CodeRabbit review on #4746).
use super::RUN_CHAT_TASK_TEST_LOCK as FORCED_ERROR_TEST_LOCK;

/// The verbatim LM Studio body from the #5291 user log.
const LMSTUDIO_TEMPLATE_400: &str = "lmstudio returned: Engine protocol predict request \
     returned 400: {\"error\":{\"code\":400,\"message\":\"Unable to generate parser for this \
     template. Automatic parser generation failed: While executing CallExpression at line 79, \
     column 24 in source: ...multi_step_tool %}  {{- raise_exception('No user query found in \
     messages.') }}...Error: Jinja Exception: No user query found in messages.\",\
     \"type\":\"invalid_request_error\"}}";

// ── #870 managed-backend errorCode classification (F2/F3/F4/F6/F8) ──

/// Build a flattened managed-backend error string the way it reaches
/// `classify_inference_error` after the typed provider error is collapsed
/// to a `String` (the `"OpenHuman API error (<status>): <body>"` envelope
/// from `inference::provider::ops::api_error`).
fn managed_error(status: &str, body: &str) -> String {
    format!("OpenHuman API error ({status}): {body}")
}

// ── SessionCacheFingerprint (thread-session cache invalidation) ───────

use super::SessionCacheFingerprint;

fn fp(
    model_override: Option<&str>,
    temperature: Option<f64>,
    target: &str,
    provider_binding: &str,
) -> SessionCacheFingerprint {
    SessionCacheFingerprint {
        model_override: model_override.map(String::from),
        temperature,
        target_agent_id: target.to_string(),
        provider_binding: provider_binding.to_string(),
        autonomy_signature: "sig-default".to_string(),
        model_registry_signature: "registry-default".to_string(),
        profile_signature: "profile-default".to_string(),
    }
}

/// Helper: poll the global in-flight table until `pred` holds (or time out).
async fn wait_for_in_flight<F: Fn(&[(String, String)]) -> bool>(pred: F) -> Vec<(String, String)> {
    timeout(Duration::from_secs(5), async {
        loop {
            let entries = in_flight_entries_for_test().await;
            if pred(&entries) {
                return entries;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("in-flight condition not met before timeout")
}

/// Helper: poll an `AtomicBool` until it is `true` (or time out).
async fn wait_for_flag(flag: &Arc<AtomicBool>, what: &str) {
    timeout(Duration::from_secs(5), async {
        while !flag.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("flag '{what}' was not set before timeout"));
}

fn make_block() -> TestRunChatTaskBlock {
    TestRunChatTaskBlock {
        started: Arc::new(AtomicBool::new(false)),
        dropped: Arc::new(AtomicBool::new(false)),
    }
}

/// Helper: poll the parallel in-flight lane until `pred` holds (or time out).
async fn wait_for_parallel<F: Fn(&[(String, String)]) -> bool>(pred: F) -> Vec<(String, String)> {
    timeout(Duration::from_secs(5), async {
        loop {
            let entries = parallel_in_flight_entries_for_test().await;
            if pred(&entries) {
                return entries;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("parallel in-flight condition not met before timeout")
}

#[path = "web_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "web_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "web_tests_part_03_tests.rs"]
mod part_03_tests;
#[path = "web_tests_part_04_tests.rs"]
mod part_04_tests;
