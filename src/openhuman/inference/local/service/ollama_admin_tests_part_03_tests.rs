use super::*;

// ── GH #5055: one-shot /api/tags fallback on a /v1/models 404 ───────────────
//
// Discovery is still chosen by provider *type* (`model_discovery_api`); these
// cover the recovery path taken when the chosen OpenAI-compatible endpoint
// answers 404, so a runtime that only speaks the Ollama listing is not left
// with an empty catalog.

/// A `/v1/models` 404 falls back to the host-rooted `/api/tags` exactly once
/// and returns that catalog.
#[tokio::test]
async fn v1_models_404_falls_back_to_ollama_api_tags() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let app = Router::new()
        .route(
            "/v1/models",
            get(|| async { (axum::http::StatusCode::NOT_FOUND, "404 page not found") }),
        )
        .route(
            "/api/tags",
            get(|| async {
                Json(json!({
                    "models": [
                        { "name": "local-model", "size": 42, "modified_at": "2026-01-01T00:00:00Z" }
                    ]
                }))
            }),
        );
    let base = spawn_mock(app).await;
    let config = lm_studio_config(&base);
    let service = LocalAiService::new(&config);

    let models = service
        .list_lm_studio_models(&config)
        .await
        .expect("the /api/tags fallback should recover discovery");

    assert_eq!(models.len(), 1);
    assert_eq!(models[0].name, "local-model");
}

/// When neither endpoint serves a catalog, the caller sees the original
/// `/v1/models` failure — not a second, more confusing error from the fallback.
#[tokio::test]
async fn v1_models_404_reports_the_original_error_when_fallback_also_fails() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    // The fallback returns a DISTINCT status (503, not 404). Without that,
    // Axum's implicit fallback route also answers 404 and the assertion below
    // would pass even if the implementation surfaced the /api/tags failure.
    let app = Router::new()
        .route(
            "/v1/models",
            get(|| async {
                (
                    axum::http::StatusCode::NOT_FOUND,
                    "404 page not found: /v1/models",
                )
            }),
        )
        .route(
            "/api/tags",
            get(|| async {
                (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "503 fallback unavailable",
                )
            }),
        );
    let base = spawn_mock(app).await;
    let config = lm_studio_config(&base);
    let service = LocalAiService::new(&config);

    let err = service
        .list_lm_studio_models(&config)
        .await
        .expect_err("no catalog anywhere must fail");

    assert!(
        err.contains("404"),
        "expected the original /v1/models status, got: {err}"
    );
    assert!(
        !err.contains("503"),
        "the fallback failure must not replace the original error, got: {err}"
    );
}

/// LM Studio answers unknown paths with `200 {"error": …}` and no models
/// (GH #5053). That ERROR ENVELOPE must not be treated as a recovery,
/// otherwise discovery silently "succeeds" with zero models.
#[tokio::test]
async fn error_envelope_api_tags_fallback_is_not_treated_as_recovery() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let app = Router::new()
        .route(
            "/v1/models",
            get(|| async { (axum::http::StatusCode::NOT_FOUND, "404 page not found") }),
        )
        .route(
            "/api/tags",
            get(|| async { Json(json!({ "error": "Unexpected endpoint or method." })) }),
        );
    let base = spawn_mock(app).await;
    let config = lm_studio_config(&base);
    let service = LocalAiService::new(&config);

    let err = service
        .list_lm_studio_models(&config)
        .await
        .expect_err("an error envelope is not a recovery");

    assert!(err.contains("404"), "got: {err}");
}

/// A fresh Ollama with nothing pulled answers `{"models":[]}`. That is a
/// REACHABLE runtime, not a failure: rejecting it hid the server behind the
/// original 404 so the UI could not offer the model-download action.
#[tokio::test]
async fn empty_api_tags_fallback_recovers_as_zero_models() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let app = Router::new()
        .route(
            "/v1/models",
            get(|| async { (axum::http::StatusCode::NOT_FOUND, "404 page not found") }),
        )
        .route("/api/tags", get(|| async { Json(json!({ "models": [] })) }));
    let base = spawn_mock(app).await;
    let config = lm_studio_config(&base);
    let service = LocalAiService::new(&config);

    let models = service
        .list_lm_studio_models(&config)
        .await
        .expect("a reachable runtime with no models is not an error");

    assert!(
        models.is_empty(),
        "expected an empty catalog, got {models:?}"
    );
}

/// Any non-404 failure must NOT trigger the fallback: a 500 is a server fault,
/// not a wrong-endpoint signal, and probing a second path would mask it.
#[tokio::test]
async fn non_404_status_does_not_trigger_the_fallback() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hits_route = std::sync::Arc::clone(&hits);
    let app = Router::new()
        .route(
            "/v1/models",
            get(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
        )
        .route(
            "/api/tags",
            get(move || {
                let hits = std::sync::Arc::clone(&hits_route);
                async move {
                    hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Json(json!({ "models": [{ "name": "should-not-be-used" }] }))
                }
            }),
        );
    let base = spawn_mock(app).await;
    let config = lm_studio_config(&base);
    let service = LocalAiService::new(&config);

    let err = service
        .list_lm_studio_models(&config)
        .await
        .expect_err("a 500 must surface, not fall back");

    assert!(err.contains("500"), "got: {err}");
    assert_eq!(
        hits.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the /api/tags fallback must not run for a non-404 status"
    );
}

// ── #5146 P1: never pull a model the user did not choose ────────────────────

/// A blank model id must fail immediately with a message about configuration,
/// never becoming a `POST /api/pull` for a nameless model.
///
/// `effective_*_model_id` returns an empty string when a role has no usable
/// model, and several callers feed that straight into
/// `ensure_ollama_model_available`. Before this guard that produced a nameless
/// pull retried three times before failing opaquely — and it is the same path
/// that silently downloaded a ~1.7 GB vision substitute.
///
/// No mock server is needed: the guard must reject before any network work, so
/// a test that reaches the network would itself be the failure.
#[tokio::test]
async fn ensure_ollama_model_available_rejects_a_blank_model_id() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    let config = Config::default();
    let service = LocalAiService::new(&config);

    for blank in ["", "   ", "\t"] {
        let err = service
            .ensure_ollama_model_available(&config, blank, "vision")
            .await
            .expect_err("a blank model id must not be pulled");
        assert!(
            err.contains("vision"),
            "error must name the role that is unconfigured: {err}"
        );
        assert!(
            err.contains("nothing to download"),
            "error must say no download will happen: {err}"
        );
    }

    // The label is carried through, so embedding reads as an embedding problem.
    let err = service
        .ensure_ollama_model_available(&config, "", "embedding")
        .await
        .expect_err("a blank model id must not be pulled");
    assert!(err.contains("embedding"), "got: {err}");
}

/// A vision model the user misconfigured must not take the whole local runtime
/// down with it.
///
/// `bootstrap()` returns on the first `ensure_models_available` error, so
/// propagating a chat-only vision model out of the `Bundled` branch left the
/// service `degraded` and skipped the remaining preloads and the ready state —
/// punishing chat for a vision-only mistake. The reason is recorded on the
/// status instead, and `resolve_vision_model_id` raises it again, actionably,
/// at request time.
#[tokio::test]
async fn bundled_vision_misconfiguration_does_not_abort_the_rest_of_bootstrap() {
    let _guard = crate::openhuman::inference::inference_test_guard();

    // The chat model is present, so only vision can fail here.
    let app = Router::new().route(
        "/api/tags",
        get(|| async {
            Json(json!({
                "models": [
                    {"name": "gemma3:1b-it-qat", "modified_at": "", "size": 1u64, "digest": "d"}
                ]
            }))
        }),
    );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let mut config = Config::default();
    config.local_ai.runtime_enabled = true;
    config.local_ai.selected_tier = Some("custom".to_string());
    config.local_ai.chat_model_id = "gemma3:1b-it-qat".to_string();
    // A valid chat model on Ollama, but it cannot accept images.
    config.local_ai.vision_model_id = "gemma3n:e4b-it-q8_0".to_string();
    config.local_ai.preload_vision_model = true;
    // `preload_embedding_model` defaults to TRUE, and the mock serves no
    // `/api/pull`, so leaving it on made the embedding preload 404 and this
    // test fail for a reason that has nothing to do with vision. The
    // later-preload interaction is covered by its own test below.
    config.local_ai.preload_embedding_model = false;

    let service = LocalAiService::new(&config);
    let result = service.ensure_models_available(&config).await;

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }

    result.expect("a chat-only vision model must not fail the whole bootstrap");

    let status = service.status.lock();
    assert_eq!(
        status.vision_state, "missing",
        "the unusable vision model must be visible as missing, not ready"
    );
    let warning = status
        .warning
        .clone()
        .expect("the reason vision is unavailable must be surfaced");
    assert!(
        warning.contains("gemma3n:e4b-it-q8_0"),
        "the warning must name the model the user configured: {warning}"
    );
    assert!(
        warning.contains("not vision-capable"),
        "the warning must say what is wrong with it: {warning}"
    );
}

/// The vision reason must still be readable after a later preload runs.
///
/// `ensure_ollama_model_available` writes `status.warning` for its own transient
/// "Pulling …" progress, so publishing the vision failure at the point it
/// happens let the embedding pull bury it — leaving `vision_state = "missing"`
/// with no explanation of why. The reason is published after every other
/// preload for exactly this reason.
#[tokio::test]
async fn a_later_preload_does_not_bury_the_vision_failure_reason() {
    use axum::routing::post;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let _guard = crate::openhuman::inference::inference_test_guard();

    // The embedding model appears only once it has been pulled, so the preload
    // takes the full download path — the one that writes `status.warning`.
    let pulled = Arc::new(AtomicBool::new(false));
    let tags_flag = pulled.clone();
    let pull_flag = pulled.clone();
    let app = Router::new()
        .route(
            "/api/tags",
            get(move || {
                let pulled = tags_flag.clone();
                async move {
                    let mut models = vec![
                        json!({"name": "gemma3:1b-it-qat", "modified_at": "", "size": 1u64, "digest": "d"}),
                    ];
                    if pulled.load(Ordering::SeqCst) {
                        models.push(
                            json!({"name": "bge-m3", "modified_at": "", "size": 2u64, "digest": "d"}),
                        );
                    }
                    Json(json!({ "models": models }))
                }
            }),
        )
        .route(
            "/api/pull",
            post(move || {
                let pulled = pull_flag.clone();
                async move {
                    pulled.store(true, Ordering::SeqCst);
                    Json(json!({ "status": "success" }))
                }
            }),
        );
    let base = spawn_mock(app).await;
    unsafe {
        std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
    }

    let mut config = Config::default();
    config.local_ai.runtime_enabled = true;
    config.local_ai.selected_tier = Some("custom".to_string());
    config.local_ai.chat_model_id = "gemma3:1b-it-qat".to_string();
    config.local_ai.vision_model_id = "gemma3n:e4b-it-q8_0".to_string();
    config.local_ai.preload_vision_model = true;
    config.local_ai.embedding_model_id = "bge-m3".to_string();
    config.local_ai.preload_embedding_model = true;

    let service = LocalAiService::new(&config);
    let result = service.ensure_models_available(&config).await;

    unsafe {
        std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
    }

    result.expect("a chat-only vision model must not fail the whole bootstrap");
    assert!(
        pulled.load(Ordering::SeqCst),
        "the embedding preload must have pulled"
    );

    let status = service.status.lock();
    assert_eq!(status.vision_state, "missing");
    assert_eq!(status.embedding_state, "ready");
    let warning = status
        .warning
        .clone()
        .expect("the vision reason must survive the embedding preload");
    assert!(
        warning.contains("gemma3n:e4b-it-q8_0"),
        "the embedding pull's progress text must not have buried it: {warning}"
    );
}

// #6032 — two-phase health probe classification. `ollama_health_status_at`
// returns `Running` on a fast 200, `Degraded` when the 2s fast probe times out
// but the 8s retry succeeds, and `Stopped` otherwise.

#[tokio::test]
async fn ollama_health_status_running_on_fast_200() {
    use super::super::health::OllamaHealthStatus;
    let _guard = crate::openhuman::inference::inference_test_guard();

    let app = Router::new().route("/api/tags", get(|| async { Json(json!({ "models": [] })) }));
    let base = spawn_mock(app).await;
    let config = Config::default();
    let service = LocalAiService::new(&config);
    assert_eq!(
        service.ollama_health_status_at(&base).await,
        OllamaHealthStatus::Running
    );
}

#[tokio::test]
async fn ollama_health_status_stopped_when_unreachable() {
    use super::super::health::OllamaHealthStatus;
    let _guard = crate::openhuman::inference::inference_test_guard();

    // Unbound port → connect refused (not a timeout) → Stopped.
    let config = Config::default();
    let service = LocalAiService::new(&config);
    assert_eq!(
        service.ollama_health_status_at("http://127.0.0.1:1").await,
        OllamaHealthStatus::Stopped
    );
}

#[tokio::test]
async fn ollama_health_status_degraded_on_slow_then_fast() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use axum::extract::State;

    use super::super::health::OllamaHealthStatus;
    let _guard = crate::openhuman::inference::inference_test_guard();

    // Stateful mock: the FIRST /api/tags call sleeps past the 2s fast-probe
    // timeout; every later call answers instantly. This is exactly a
    // momentarily-busy daemon — the fast probe times out, the 8s retry
    // succeeds → Degraded.
    let calls: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/api/tags",
            get(|State(calls): State<Arc<AtomicUsize>>| async move {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(3000)).await;
                }
                Json(json!({ "models": [] }))
            }),
        )
        .with_state(calls);
    let base = spawn_mock(app).await;
    let config = Config::default();
    let service = LocalAiService::new(&config);
    assert_eq!(
        service.ollama_health_status_at(&base).await,
        OllamaHealthStatus::Degraded
    );
}

#[tokio::test]
async fn diagnostics_degraded_ollama_stays_running_and_lists_models() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use axum::extract::State;
    use axum::routing::post;

    let _guard = crate::openhuman::inference::inference_test_guard();

    // Stateful mock reproducing a busy daemon AND keeping the *model-discovery*
    // request slow, so this test actually exercises the widened 8s discovery
    // timeout — not just the health probe. `diagnostics()` issues /api/tags in
    // this order:
    //   #0 health fast probe (2s)  — must time out to reach Degraded
    //   #1 health retry      (8s)  — must be fast so we classify Degraded
    //   #2 runner probe      (3s)  — reachability check
    //   #3 model discovery   (8s)  — MUST stay slow: reverting it to the 5s
    //                                default would time out here and empty the
    //                                catalog, hiding local models (the #6032 bug)
    // So every call EXCEPT the health retry (#1) sleeps 6s — above the 2s fast
    // probe and the 5s default, below the 8s degraded budget. Keying off "not
    // the retry" rather than a fixed index keeps this correct even if the number
    // of pre-discovery /api/tags calls changes.
    let tags_calls: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/api/tags",
            get(|State(calls): State<Arc<AtomicUsize>>| async move {
                if calls.fetch_add(1, Ordering::SeqCst) != 1 {
                    tokio::time::sleep(std::time::Duration::from_millis(6000)).await;
                }
                Json(json!({
                    "models": [
                        {"name": "llama3:latest", "modified_at": "", "size": 1u64, "digest": "d"}
                    ]
                }))
            }),
        )
        .route("/api/show", post(|| async { Json(json!({})) }))
        .with_state(tags_calls);
    let base = spawn_mock(app).await;

    let mut config = Config::default();
    config.local_ai.provider = "ollama".to_string();
    config.local_ai.base_url = Some(base);
    let service = LocalAiService::new(&config);

    let report = service.diagnostics(&config).await.expect("diagnostics ok");
    assert_eq!(
        report["ollama_status"], "degraded",
        "a slow-but-alive daemon must report degraded, not stopped"
    );
    assert_eq!(
        report["ollama_running"], true,
        "degraded still counts as running so local models stay selectable"
    );
    let models = report["installed_models"]
        .as_array()
        .expect("installed_models array");
    assert!(
        !models.is_empty(),
        "model discovery must survive the degraded timeout window (non-empty catalog)"
    );
}
