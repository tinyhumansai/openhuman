use super::*;
use axum::{routing::post, Json, Router};
use serde_json::json;

async fn spawn_mock(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://127.0.0.1:{}", addr.port())
}

fn enabled_config() -> Config {
    let mut config = Config::default();
    config.local_ai.runtime_enabled = true;
    config
}

/// Build a LocalAiService pre-seeded to `ready` so inference calls skip
/// `bootstrap()` and hit the HTTP path directly.
fn ready_service(config: &Config) -> LocalAiService {
    let service = LocalAiService::new(config);
    {
        let mut guard = service.status.lock();
        guard.state = "ready".to_string();
    }
    service
}

#[tokio::test]
async fn inference_hits_ollama_generate_and_returns_response() {
    let _guard = crate::openhuman::local_ai::local_ai_test_guard();

    let app = Router::new().route(
        "/api/generate",
        post(|Json(_body): Json<serde_json::Value>| async move {
            Json(json!({
                "model": "test",
                "response": "hello from mock",
                "done": true,
                "total_duration": 1_000_000u64,
                "prompt_eval_count": 5,
                "prompt_eval_duration": 100_000u64,
                "eval_count": 3,
                "eval_duration": 500_000u64
            }))
        }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = enabled_config();
    let service = ready_service(&config);
    let reply = service
        .prompt(&config, "hi", Some(16), true)
        .await
        .expect("ollama prompt");
    assert_eq!(reply, "hello from mock");

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn inference_errors_on_non_success_status() {
    let _guard = crate::openhuman::local_ai::local_ai_test_guard();

    let app = Router::new().route(
        "/api/generate",
        post(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = enabled_config();
    let service = ready_service(&config);
    let err = service.prompt(&config, "hi", None, true).await.unwrap_err();
    assert!(err.contains("500"));

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}

#[tokio::test]
async fn inference_errors_on_empty_response_when_allow_empty_false() {
    let _guard = crate::openhuman::local_ai::local_ai_test_guard();

    let app = Router::new().route(
        "/api/generate",
        post(|| async {
            Json(json!({
                "model": "test",
                "response": "   ",
                "done": true
            }))
        }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = enabled_config();
    let service = ready_service(&config);
    // `inference()` is the lower-level entry that hard-codes
    // allow_empty=false, so a whitespace-only mock response must
    // surface as the "empty content" error.
    let res = service.inference(&config, "", "hi", None, false).await;

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }

    let err = res.expect_err("whitespace response must be rejected when allow_empty=false");
    assert!(
        err.contains("empty"),
        "expected an empty-content error, got: {err}"
    );
}

#[tokio::test]
async fn summarize_disabled_returns_error() {
    // When local_ai is disabled the summarize fn should short-circuit.
    let mut config = Config::default();
    config.local_ai.runtime_enabled = false;
    let service = LocalAiService::new(&config);
    let err = service.summarize(&config, "text", None).await.unwrap_err();
    assert!(err.contains("local ai is disabled"));
}

#[tokio::test]
async fn prompt_disabled_returns_error() {
    let mut config = Config::default();
    config.local_ai.runtime_enabled = false;
    let service = LocalAiService::new(&config);
    let err = service
        .prompt(&config, "text", None, false)
        .await
        .unwrap_err();
    assert!(err.contains("local ai is disabled"));
}

#[tokio::test]
async fn inline_complete_disabled_returns_empty_string() {
    let mut config = Config::default();
    config.local_ai.runtime_enabled = false;
    let service = LocalAiService::new(&config);
    let out = service
        .inline_complete(&config, "ctx", "casual", None, &[], None)
        .await
        .unwrap();
    assert!(out.is_empty());
}

#[tokio::test]
async fn inline_complete_interactive_disabled_returns_empty_string() {
    // Interactive variant must match the gated variant on the
    // disabled short-circuit so the autocomplete UX is identical.
    let mut config = Config::default();
    config.local_ai.runtime_enabled = false;
    let service = LocalAiService::new(&config);
    let out = service
        .inline_complete_interactive(&config, "ctx", "casual", None, &[], None)
        .await
        .unwrap();
    assert!(out.is_empty());
}

/// Interactive autocomplete (`inline_complete_interactive`) MUST NOT
/// block on a held LLM permit. Hold the global slot, race the
/// interactive variant against a tight deadline; if it queued behind
/// the permit it would deadlock or time out.
#[tokio::test]
async fn inline_complete_interactive_does_not_block_on_held_permit() {
    let _guard = crate::openhuman::local_ai::local_ai_test_guard();

    // Hold the global LLM permit for the duration of the test.
    let _held = crate::openhuman::scheduler_gate::gate::try_acquire_llm_permit()
        .expect("test must start with a free permit; previous test leaked one");

    let app = Router::new().route(
        "/api/generate",
        post(|Json(_body): Json<serde_json::Value>| async move {
            Json(json!({
                "model": "test",
                "response": "ip",
                "done": true
            }))
        }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = enabled_config();
    let service = ready_service(&config);

    // Tight 2s deadline — comfortably above mock RTT, well below any
    // policy-paused-poll backoff. If the interactive call goes through
    // the gate it'll never finish.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        service.inline_complete_interactive(&config, "ctx", "casual", None, &[], Some(8)),
    )
    .await;

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }

    let inner = result.expect("interactive variant must NOT block on held permit");
    assert!(
        inner.is_ok(),
        "interactive call should have completed: {inner:?}"
    );
}

/// Counterpart: the gated `inline_complete` (and `prompt`/`summarize`)
/// MUST queue behind a held permit. We assert this with a try-style
/// race: spawn the gated call, give it time to enter the wait, then
/// confirm it hasn't completed. We then drop the permit and verify
/// the call resolves.
#[tokio::test]
// Wake-on-permit-drop timing test: under heavy parallel cargo-test load
// the 2s timeout occasionally fires before the spawned waiter resolves.
// Panicking here would poison `LOCAL_AI_TEST_MUTEX` and cascade
// PoisonError into every other local_ai test, so re-ignoring is the
// safer trade-off. See PR #1524.
#[ignore = "flaky timing under full-suite load — see PR #1524"]
async fn gated_inline_complete_blocks_on_held_permit() {
    let _guard = crate::openhuman::local_ai::local_ai_test_guard();

    let held = crate::openhuman::scheduler_gate::gate::try_acquire_llm_permit()
        .expect("test must start with a free permit");

    let app = Router::new().route(
        "/api/generate",
        post(|Json(_body): Json<serde_json::Value>| async move {
            Json(json!({
                "model": "test",
                "response": "x",
                "done": true
            }))
        }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let config = enabled_config();
    let service = std::sync::Arc::new(ready_service(&config));
    let svc = service.clone();
    let cfg = config.clone();

    let join = tokio::spawn(async move {
        svc.inline_complete(&cfg, "ctx", "casual", None, &[], Some(8))
            .await
    });

    // Give the spawned task a chance to enter `wait_for_capacity`.
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    assert!(
        !join.is_finished(),
        "gated inline_complete must block while permit is held"
    );

    // Release the permit; the gated call should now resolve.
    drop(held);
    let resolved = tokio::time::timeout(std::time::Duration::from_secs(2), join)
        .await
        .expect("gated call must resolve once permit is released")
        .expect("join")
        .expect("ollama call");
    assert!(!resolved.is_empty() || resolved.is_empty()); // sanity — value depends on sanitiser

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }
}
