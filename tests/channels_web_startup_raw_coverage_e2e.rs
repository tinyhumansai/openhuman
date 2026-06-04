//! Raw integration coverage for channel web-provider and startup paths.
//!
//! These tests intentionally drive debug/test-support seams with loopback or
//! in-memory inputs so coverage reaches production branches without real
//! channel credentials or external inference providers.

use std::time::Duration;

use openhuman_core::openhuman::channels::start_channels;
use openhuman_core::openhuman::channels::test_support::{
    run_dispatch_harness, DispatchHarnessOptions, TestMemoryEntry,
};
use openhuman_core::openhuman::channels::web::{
    all_web_channel_controller_schemas, all_web_channel_registered_controllers, channel_web_cancel,
    channel_web_chat, schemas, start_chat, subscribe_web_channel_events,
    test_support as web_test_support,
};
use openhuman_core::openhuman::config::Config;
use tempfile::tempdir;
use tokio::time::timeout;

fn isolated_config() -> (tempfile::TempDir, Config) {
    let tmp = tempdir().expect("tempdir");
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");

    let mut config = Config::default();
    config.workspace_dir = workspace;
    config.config_path = tmp.path().join("config.toml");
    config.api_key = None;
    config.api_url = Some("http://127.0.0.1:9".to_string());
    config.inference_url = None;
    config.memory.auto_save = false;
    config.browser.enabled = false;
    (tmp, config)
}

#[test]
fn web_error_debug_export_covers_provider_config_and_retry_branches() {
    let rate_limited = web_test_support::classify_error_for_test(
        r#"openrouter API error (429 Too Many Requests): {"retry_after": 1.2}"#,
    );
    assert_eq!(rate_limited.error_type, "rate_limited");
    assert_eq!(rate_limited.source, "provider");
    assert_eq!(rate_limited.retry_after_ms, Some(2_000));
    assert_eq!(rate_limited.provider.as_deref(), Some("openrouter"));
    assert!(rate_limited.retryable);

    let action_budget = web_test_support::classify_error_for_test(
        "Rate limit exceeded: action budget exhausted while running web_fetch",
    );
    assert_eq!(action_budget.error_type, "action_budget_exceeded");
    assert_eq!(action_budget.source, "openhuman_budget");
    assert!(action_budget.provider.is_none());

    let non_retryable = web_test_support::classify_error_for_test(
        "zai API error (429 Too Many Requests): code=1311 insufficient balance",
    );
    assert_eq!(non_retryable.error_type, "rate_limited");
    assert!(!non_retryable.retryable);
    assert_eq!(non_retryable.provider.as_deref(), Some("zai"));

    let exhausted = web_test_support::classify_error_for_test(
        "All providers/models failed. Attempts: openhuman API error (503 Service Unavailable)",
    );
    assert_eq!(exhausted.error_type, "provider_error");
    assert_eq!(exhausted.fallback_available, Some(false));

    let detail = web_test_support::extracted_provider_detail_for_test(
        r#"custom_openai API error (404 Not Found): {"error":{"message":"Model `missing-model` does not exist"}}"#,
    )
    .expect("provider detail");
    assert!(detail.contains("missing-model"));

    assert_eq!(
        web_test_support::retry_after_secs_for_test("retry-after: 0"),
        Some(0)
    );
    assert!(web_test_support::is_non_retryable_rate_limit_for_test(
        "package not active"
    ));
}

/// Serialize tests that run a web chat task whose outcome depends on the
/// process-global forced-error seam (`set_forced_run_chat_task_error_for_test`).
/// Without this, the forced error one test installs leaks into another's chat
/// run, crossing their expected error types (e.g. `rate_limited` vs
/// `cancelled`) under cargo-llvm-cov's multi-threaded execution.
fn web_chat_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[tokio::test]
async fn web_controllers_validate_inputs_and_emit_structured_forced_errors() {
    let _chat_lock = web_chat_lock();
    let controller_schemas = all_web_channel_controller_schemas();
    assert_eq!(controller_schemas.len(), 4);
    assert!(controller_schemas
        .iter()
        .any(|schema| schema.function == "web_chat"));
    assert!(controller_schemas
        .iter()
        .any(|schema| schema.function == "web_cancel"));
    assert!(controller_schemas
        .iter()
        .any(|schema| schema.function == "web_queue_status"));
    assert!(controller_schemas
        .iter()
        .any(|schema| schema.function == "web_queue_clear"));
    assert_eq!(all_web_channel_registered_controllers().len(), 4);
    assert_eq!(schemas("missing").function, "unknown");

    let err = channel_web_chat("client", "thread", "   ", None, None, None, None, None)
        .await
        .expect_err("blank messages are rejected");
    assert!(err.contains("message is required"));

    let cancel = channel_web_cancel("client", "missing-thread")
        .await
        .expect("cancel without in-flight request is ok")
        .into_cli_compatible_json()
        .expect("json");
    assert_eq!(cancel["result"]["cancelled"], false);

    web_test_support::set_forced_run_chat_task_error_for_test(Some(
        "openrouter API error (429 Too Many Requests): Retry-After: 7",
    ))
    .await;

    let mut rx = subscribe_web_channel_events();
    let accepted = channel_web_chat(
        "client-a",
        "thread-a",
        "Summarize this safely.",
        Some(" hint:reasoning ".to_string()),
        Some(0.2),
        None,
        Some("zh-CN".to_string()),
        None,
    )
    .await
    .expect("chat request accepted")
    .into_cli_compatible_json()
    .expect("json");
    let request_id = accepted["result"]["request_id"]
        .as_str()
        .expect("request id")
        .to_string();

    let event = timeout(Duration::from_secs(10), async {
        loop {
            let event = rx.recv().await.expect("web channel event");
            if event.request_id == request_id && event.event == "chat_error" {
                break event;
            }
        }
    })
    .await
    .expect("chat_error event");

    assert_eq!(event.error_type.as_deref(), Some("rate_limited"));
    assert_eq!(event.error_source.as_deref(), Some("provider"));
    assert_eq!(event.error_retry_after_ms, Some(7_000));
    assert_eq!(event.error_provider.as_deref(), Some("openrouter"));
    web_test_support::set_forced_run_chat_task_error_for_test(None).await;
}

#[tokio::test]
async fn web_chat_cancel_aborts_in_flight_thread_without_real_provider() {
    let _chat_lock = web_chat_lock();
    // Clear any forced error a sibling test may have leaked before this chat
    // runs, so the cancellation path produces a real `cancelled` error rather
    // than inheriting a stale forced `rate_limited`/`inference` one.
    web_test_support::set_forced_run_chat_task_error_for_test(None).await;
    let mut rx = subscribe_web_channel_events();
    let request_id = start_chat(
        "cancel-client",
        "cancel-thread",
        "This request should be cancelled before inference completes.",
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("start chat");

    let cancel = channel_web_cancel("cancel-client", "cancel-thread")
        .await
        .expect("cancel")
        .into_cli_compatible_json()
        .expect("json");
    assert_eq!(cancel["result"]["cancelled"], true);
    assert_eq!(cancel["result"]["request_id"], request_id);

    let event = timeout(Duration::from_secs(10), async {
        loop {
            let event = rx.recv().await.expect("web channel event");
            if event.request_id == request_id && event.event == "chat_error" {
                break event;
            }
        }
    })
    .await
    .expect("cancel event");

    assert_eq!(event.error_type.as_deref(), Some("cancelled"));
    assert_eq!(event.message.as_deref(), Some("Cancelled"));
}

#[tokio::test]
async fn startup_no_channels_initializes_runtime_and_exits_cleanly() {
    let (_tmp, config) = isolated_config();
    timeout(Duration::from_secs(20), start_channels(config))
        .await
        .expect("startup should not hang")
        .expect("no-channel startup should be ok");
}

#[tokio::test]
async fn dispatch_harness_covers_streaming_history_timeout_and_memory_paths() {
    let streaming = run_dispatch_harness(DispatchHarnessOptions {
        channel_name: "web".to_string(),
        content: "please inspect the remembered project".to_string(),
        streaming: true,
        supports_reactions: true,
        seed_history_len: 3,
        memory_entries: vec![TestMemoryEntry {
            key: "project".to_string(),
            content: "The project uses a loopback mock.".to_string(),
            score: Some(0.95),
        }],
        response_text: Some("streamed dispatch response".to_string()),
        ..DispatchHarnessOptions::default()
    })
    .await;
    assert!(streaming.handler_had_progress);
    assert!(streaming.start_typing_calls >= 1);
    assert!(streaming.stop_typing_calls >= 1);
    assert!(streaming
        .sends
        .iter()
        .any(|send| send.kind == "finalize_draft" || send.kind == "send"));
    assert!(streaming.handler_history_text.contains("loopback mock"));
    assert!(streaming.retained_history_len >= 1);

    let failed = run_dispatch_harness(DispatchHarnessOptions {
        channel_name: "telegram".to_string(),
        content: "force a handler failure".to_string(),
        thread_ts: Some("thread-42".to_string()),
        handler_error: Some("handler failed deliberately".to_string()),
        timeout_secs: 1,
        ..DispatchHarnessOptions::default()
    })
    .await;
    assert!(failed
        .sends
        .iter()
        .any(|send| send.content.contains("handler failed deliberately")));
}
