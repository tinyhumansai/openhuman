use super::{
    all_web_channel_controller_schemas, all_web_channel_registered_controllers, cancel_chat_scoped,
    channel_web_cancel, channel_web_queue_clear, channel_web_queue_remove,
    channel_web_queue_status, classify_inference_error, drain_queued_turns_for_test,
    event_session_id_for, generic_inference_error_user_message, in_flight_entries_for_test,
    inference_budget_exceeded_user_message, is_inference_budget_exceeded_error, key_for,
    locale_reply_directive, normalize_model_override, parallel_in_flight_entries_for_test,
    provider_role_for_model_override, schemas, sentry_suppression_reason,
    set_test_forced_run_chat_task_error, set_test_run_chat_task_block, start_chat,
    subscribe_web_channel_events, ChatRequestMetadata, ClassifiedError, StartChatError,
    TestRunChatTaskBlock, WebChatParams,
};
use crate::core::TypeSchema;
use crate::inference::registry::{
    register_backend_error_classifier, BackendErrorClassification, BackendErrorKind,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tinyagents_harness::run_queue::QueueLane;
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

/// Run a web-copy assertion with explicit provider-neutral metadata. Hosted
/// token/envelope parsing belongs to TinyHumans; core tests inject the semantic
/// classification that the host registry supplies at runtime.
fn classify_with_backend_metadata(
    message: &str,
    kind: BackendErrorKind,
    malformed_bad_request: bool,
) -> ClassifiedError {
    static FIXTURES: OnceLock<Arc<Mutex<HashMap<String, BackendErrorClassification>>>> =
        OnceLock::new();
    let fixtures = Arc::clone(FIXTURES.get_or_init(|| Arc::new(Mutex::new(HashMap::new()))));
    register_backend_error_classifier(Arc::new(move |_provider, error| {
        fixtures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(error)
            .cloned()
            .unwrap_or(BackendErrorClassification {
                provider_owned: false,
                backend_owned: false,
                reportable: true,
                token: None,
                kind: None,
                malformed_bad_request: false,
            })
    }));
    FIXTURES
        .get()
        .expect("fixture map initialized")
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(
            message.to_string(),
            BackendErrorClassification {
                provider_owned: true,
                backend_owned: true,
                reportable: malformed_bad_request
                    || matches!(
                        kind,
                        BackendErrorKind::PayloadTooLarge | BackendErrorKind::ContextLengthExceeded
                    ),
                token: None,
                kind: Some(kind),
                malformed_bad_request,
            },
        );
    classify_inference_error(message)
}

fn classify_failure_fixture(case: &str, message: &str) -> ClassifiedError {
    let metadata = match case {
        "be_rate_limited" => Some((BackendErrorKind::RateLimited, false)),
        "be_credits" => Some((BackendErrorKind::InsufficientCredits, false)),
        "be_upstream" => Some((BackendErrorKind::UpstreamUnavailable, false)),
        "be_model_unavailable" => Some((BackendErrorKind::ModelUnavailable, false)),
        "be_payload" => Some((BackendErrorKind::PayloadTooLarge, false)),
        "be_context" => Some((BackendErrorKind::ContextLengthExceeded, false)),
        "be_bad_request" | "be_bad_request_history" => Some((BackendErrorKind::BadRequest, false)),
        "be_bad_request_malformed" => Some((BackendErrorKind::BadRequest, true)),
        "be_internal" => Some((BackendErrorKind::Internal, false)),
        _ => None,
    };
    match metadata {
        Some((kind, malformed)) => classify_with_backend_metadata(message, kind, malformed),
        None => classify_inference_error(message),
    }
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
        effective_model: "sample-model".to_string(),
        temperature,
        target_agent_id: target.to_string(),
        provider_binding: provider_binding.to_string(),
        autonomy_signature: "sig-default".to_string(),
        model_registry_signature: "registry-default".to_string(),
        workspace_dir: std::path::PathBuf::from("/ws/a"),
    }
}

/// Helper: poll the global in-flight table until `pred` holds (or time out).
async fn wait_for_in_flight<F: Fn(&[(String, String)]) -> bool>(pred: F) -> Vec<(String, String)> {
    timeout(Duration::from_secs(5), async {
        loop {
            let entries =
                in_flight_entries_for_test(&crate::core::runtime::DispatchCtx::ambient()).await;
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
        release: Arc::new(tokio::sync::Notify::new()),
        max_park_for: Duration::from_secs(30),
        succeed_in: None,
    }
}

/// Helper: poll the parallel in-flight lane until `pred` holds (or time out).
async fn wait_for_parallel<F: Fn(&[(String, String)]) -> bool>(pred: F) -> Vec<(String, String)> {
    timeout(Duration::from_secs(5), async {
        loop {
            let entries =
                parallel_in_flight_entries_for_test(&crate::core::runtime::DispatchCtx::ambient())
                    .await;
            if pred(&entries) {
                return entries;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("parallel in-flight condition not met before timeout")
}

#[path = "web_tests_cancel_scoping_tests.rs"]
mod cancel_scoping_tests;
#[path = "web_tests_error_code_classification_tests.rs"]
mod error_code_classification_tests;
#[path = "web_tests_error_copy_fixture_tests.rs"]
mod error_copy_fixture_tests;
#[path = "web_tests_queue_acceptance_tests.rs"]
mod queue_acceptance_tests;
#[path = "web_tests_rate_limit_classification_tests.rs"]
mod rate_limit_classification_tests;
#[path = "web_tests_session_and_concurrency_tests.rs"]
mod session_and_concurrency_tests;
#[path = "web_tests_start_chat_ingress_tests.rs"]
mod start_chat_ingress_tests;
#[path = "web_tests_turn_lifecycle_tests.rs"]
mod turn_lifecycle_tests;

#[cfg(feature = "voice")]
#[path = "web_tests_ptt_source_tests.rs"]
mod ptt_source_tests;
