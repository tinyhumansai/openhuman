use super::*;
// Production code now routes managed construction through
// `create_embedding_provider_with_config`; this low-level custom-endpoint
// regression test still drives the credentialed factory directly.
use super::super::create_embedding_provider_with_credentials;
use tempfile::TempDir;

/// The seam the memory factory depends on (TAURI-RUST-52S fix): the three
/// `create_memory_with_local_ai` call sites resolve the user's stored BYO
/// embedding credential via `resolve_api_key` and thread it into the
/// provider. If this lookup silently returns "" for a configured key —
/// wrong cred slug, encryption mismatch, profile-store regression — the
/// memory pipeline reverts to sending an empty bearer and Cohere 401s on
/// every embed. Lock the round-trip: store under `embeddings:<slug>`, read
/// it back; an unrelated provider must stay empty (no cross-bleed).
#[test]
fn resolve_api_key_returns_stored_embeddings_credential() {
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");

    // Nothing stored yet → empty (the empty-key guard's "" input).
    assert_eq!(resolve_api_key(&config, "cohere"), "");

    // Store a Cohere embeddings key exactly as `set_api_key` does.
    AuthService::from_config(&config)
        .store_provider_token(
            "embeddings:cohere",
            "default",
            "sk-cohere-test",
            HashMap::new(),
            true,
        )
        .unwrap();

    // Resolve returns it; a provider with no stored key stays empty.
    assert_eq!(resolve_api_key(&config, "cohere"), "sk-cohere-test");
    assert_eq!(resolve_api_key(&config, "voyage"), "");
}

/// `get_settings` must report the embedder ingestion will **actually** use
/// alongside the picker's own setting (#5402). The two disagree whenever
/// the user enabled local embeddings through Local AI Settings: that path
/// never rewrites `memory.embedding_provider`, so `provider` still reads
/// `"cloud"` while nothing bills the managed budget. A consumer that gated
/// a "your memory has stopped growing" banner on `provider` would fire it
/// at a user whose memory is growing fine.
#[tokio::test]
async fn get_settings_reports_effective_provider_separately_from_the_setting() {
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().to_path_buf();
    config.memory.embedding_provider = "cloud".to_string();
    // A managed session exists, so the ladder would resolve to cloud …
    std::fs::write(tmp.path().join("auth-profiles.json"), "{}").unwrap();
    // … except a local Ollama route wins. As of tinymemory v1.0.1 the
    // effective-embedder ladder no longer treats the `embeddings_provider`
    // string alone as authoritative for local routing — local Ollama is
    // resolved from an explicit `memory_tree.embedding_endpoint` override or
    // the unified `workload_local_model` setting. Drive the explicit
    // endpoint rung here: it resolves deterministically without an installed
    // embedding host, and still exercises the point of the test — that
    // `provider` (the picker) stays `cloud` while `effective_provider`
    // reports the local route that bills nothing (#5402).
    config.embeddings_provider = Some("ollama:all-minilm:latest".into());
    config.memory_tree.embedding_endpoint = Some("http://localhost:11434".into());
    config.memory_tree.embedding_model = Some("all-minilm".into());

    let out = get_settings(&config)
        .await
        .expect("get_settings must succeed");
    assert_eq!(
        out.value["provider"], "cloud",
        "the picker setting is unchanged"
    );
    assert_eq!(
        out.value["effective_provider"], "ollama",
        "the effective embedder is local, so nothing bills the managed budget"
    );
}

/// `custom:<url>` providers must look up under the `embeddings:custom`
/// slug (the inline URL is not part of the credential key), mirroring the
/// slug normalization in `embed`/`set_api_key`.
#[test]
fn resolve_api_key_normalizes_custom_prefix_to_custom_slug() {
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");

    AuthService::from_config(&config)
        .store_provider_token(
            "embeddings:custom",
            "default",
            "sk-custom-test",
            HashMap::new(),
            true,
        )
        .unwrap();

    assert_eq!(
        resolve_api_key(&config, "custom:http://localhost:1234"),
        "sk-custom-test"
    );
}

/// Issue #4056: after a successful probe we adopt the endpoint's real
/// returned length for auto-detected models, but keep the requested size for
/// `text-embedding-3-*` (the server returned exactly that). A zero actual
/// (defensive — empty vectors are already rejected upstream) falls back to
/// the configured value.
#[test]
fn final_probe_dims_adopts_actual_for_auto_detected_models() {
    // Auto-detected model → adopt the real length, ignoring the guess.
    assert_eq!(final_probe_dims("bge-m3", 1024, 1024), 1024);
    assert_eq!(final_probe_dims("bge-m3", 1024, 768), 768);
    assert_eq!(final_probe_dims("nomic-embed-text", 1024, 768), 768);
    // text-embedding-3-* → keep the requested size (param was honoured).
    assert_eq!(final_probe_dims("text-embedding-3-large", 1024, 3072), 1024);
    // Defensive: zero actual falls back to the configured value.
    assert_eq!(final_probe_dims("bge-m3", 1024, 0), 1024);
}

#[test]
fn normalize_embed_model_id_strips_prefix_and_tag() {
    assert_eq!(normalize_embed_model_id("text-embedding-bge-m3"), "bge-m3");
    assert_eq!(normalize_embed_model_id("bge-m3"), "bge-m3");
    assert_eq!(normalize_embed_model_id("bge-m3:latest"), "bge-m3");
    assert_eq!(normalize_embed_model_id("TEXT-EMBEDDING-BGE-M3"), "bge-m3");
    // Exact-after-strip: must not collapse a different model onto bge-m3.
    assert_ne!(normalize_embed_model_id("bge-m3-distill"), "bge-m3");
}

#[test]
fn reject_model_not_served_suggests_normalized_match() {
    // User entered `bge-m3`; LM Studio serves `text-embedding-bge-m3` —
    // the feedback names the exact served id to select (issue #3761).
    let served = vec!["text-embedding-bge-m3".to_string(), "qwen-chat".to_string()];
    let out = reject_model_not_served("bge-m3", &served);
    assert_eq!(out.value["error"], "EMBEDDINGS_NO_MODEL_LOADED");
    assert_eq!(out.value["suggested_model"], "text-embedding-bge-m3");
    let msg = out.value["message"].as_str().unwrap();
    assert!(msg.contains("text-embedding-bge-m3"));
}

#[test]
fn reject_model_not_served_without_match_lists_available() {
    let served = vec!["qwen-chat".to_string(), "llama-3".to_string()];
    let out = reject_model_not_served("bge-m3", &served);
    assert_eq!(out.value["error"], "EMBEDDINGS_NO_MODEL_LOADED");
    assert!(out.value.get("suggested_model").is_none());
    let msg = out.value["message"].as_str().unwrap();
    assert!(msg.contains("qwen-chat") && msg.contains("llama-3"));
}

#[test]
fn check_requested_model_served_decisions() {
    // Served exactly → accept (None).
    assert!(check_requested_model_served(
        "text-embedding-bge-m3",
        &["text-embedding-bge-m3".to_string()],
    )
    .is_none());
    // Empty/unknown list → defer to probe (None), never block.
    assert!(check_requested_model_served("bge-m3", &[]).is_none());
    // Non-empty list without the model → reject with feedback.
    let reject = check_requested_model_served("bge-m3", &["text-embedding-bge-m3".to_string()]);
    assert_eq!(reject.unwrap().value["error"], "EMBEDDINGS_NO_MODEL_LOADED");
}

#[tokio::test]
async fn fetch_served_model_ids_parses_openai_models_list() {
    use axum::{routing::get, Json, Router};
    let app = Router::new().route(
        "/v1/models",
        get(|| async {
            Json(serde_json::json!({
                "object": "list",
                "data": [
                    { "id": "text-embedding-bge-m3", "object": "model" },
                    { "id": "qwen-chat", "object": "model" }
                ]
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let ids = fetch_served_model_ids(&format!("{base}/v1"), "")
        .await
        .expect("models list");
    assert_eq!(ids, vec!["text-embedding-bge-m3", "qwen-chat"]);
}

/// Helper: pull the `error` code out of a reject payload.
fn reject_code(outcome: EmbedProbe) -> Option<String> {
    classify_embed_probe(outcome).map(|rpc| {
        rpc.value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    })
}

/// A usable vector is the ONLY thing that passes the setup-time gate — the
/// config is then accepted and persisted.
#[test]
fn classify_embed_probe_accepts_only_usable_vector() {
    assert!(
        classify_embed_probe(EmbedProbe::Returned(vec![vec![0.1, 0.2, 0.3]])).is_none(),
        "a non-empty vector must verify the endpoint"
    );
}

/// Reachable but empty/zero-dim response is a failed verification, not a
/// valid embedder — never persist it.
#[test]
fn classify_embed_probe_rejects_empty_vectors() {
    assert_eq!(
        reject_code(EmbedProbe::Returned(vec![])).as_deref(),
        Some("EMBEDDINGS_VERIFICATION_FAILED")
    );
    assert_eq!(
        reject_code(EmbedProbe::Returned(vec![vec![]])).as_deref(),
        Some("EMBEDDINGS_VERIFICATION_FAILED")
    );
}

/// LM Studio idle ("No models loaded") must reject the save with the
/// one-step remediation code so the doomed config is never persisted — the
/// fix is verifying at setup, not suppressing the later flood.
#[test]
fn classify_embed_probe_rejects_no_model_loaded() {
    let body = r#"Embedding API error (400 Bad Request): {"error":"No models loaded. Please load a model in the developer page or use the 'lms load' command."}"#;
    let rpc = classify_embed_probe(EmbedProbe::Failed(body.to_string())).unwrap();
    assert_eq!(
        rpc.value.get("error").and_then(|v| v.as_str()),
        Some("EMBEDDINGS_NO_MODEL_LOADED")
    );
    // The raw provider detail is preserved for the UI.
    assert_eq!(rpc.value.get("detail").and_then(|v| v.as_str()), Some(body));
}

/// A 404/405 (no `/embeddings` route) keeps its dedicated code.
#[test]
fn classify_embed_probe_rejects_endpoint_absent() {
    for detail in [
        "Embedding API error (404 Not Found): no route",
        "openai embeddings returned HTTP 404 Not Found: no route",
    ] {
        assert_eq!(
            reject_code(EmbedProbe::Failed(detail.into())).as_deref(),
            Some("EMBEDDINGS_ENDPOINT_NO_API")
        );
    }
}

/// An unclassified 5xx still rejects with the generic code — it's a real
/// server fault, not one of the actionable user-config shapes.
#[test]
fn classify_embed_probe_rejects_unclassified_5xx_generically() {
    assert_eq!(
        reject_code(EmbedProbe::Failed(
            "openai embeddings returned HTTP 500 Internal Server Error: boom".into()
        ))
        .as_deref(),
        Some("EMBEDDINGS_VERIFICATION_FAILED")
    );
}

/// Issue #5017 — the #5017 reporter's exact case: a chat/reasoning model id
/// (`gpt-5-mini`) that works for chat is pasted into the embeddings model
/// field. The endpoint IS an embeddings API but rejects the model with a 400;
/// before the fix this collapsed into the generic "test embed failed", so the
/// user couldn't tell the model wasn't an embeddings model. Now it maps to a
/// dedicated, actionable code — across both wire shapes.
#[test]
fn classify_embed_probe_distinguishes_incompatible_model() {
    for detail in [
        r#"openai embeddings returned HTTP 400 Bad Request: {"error":{"message":"gpt-5-mini does not support embeddings"}}"#,
        r#"Embedding API error (400 Bad Request): {"error":{"message":"Model gpt-5-mini does not exist"}}"#,
        r#"openai embeddings returned HTTP 400 Bad Request: {"error":"this is not an embedding model"}"#,
    ] {
        assert_eq!(
            reject_code(EmbedProbe::Failed(detail.into())).as_deref(),
            Some("EMBEDDINGS_MODEL_INCOMPATIBLE"),
            "detail should classify as incompatible model: {detail}"
        );
    }
}

/// Issue #5017 — a rejected/absent API key (401/403) is its own actionable
/// cause: the embeddings key is stored separately from the Chat BYOK key, so
/// "works for chat" does not imply the embeddings key is set. Both wire
/// shapes map to the dedicated auth code, not the generic failure.
#[test]
fn classify_embed_probe_distinguishes_auth_failure() {
    for detail in [
        "openai embeddings returned HTTP 401 Unauthorized: {\"error\":\"invalid api key\"}",
        "Embedding API error (403 Forbidden): no access",
        // Bare-status host shape (no parentheses) — the form the observability
        // classifier covers; must map to auth, not the generic failure (#5017).
        "Embedding API error 401 Unauthorized: {\"error\":\"invalid token\"}",
    ] {
        assert_eq!(
            reject_code(EmbedProbe::Failed(detail.into())).as_deref(),
            Some("EMBEDDINGS_AUTH_FAILED"),
            "detail should classify as auth failure: {detail}"
        );
    }
}

/// Issue #5116 — a **chat** model used as an embeddings model. OpenAI answers
/// *HTTP 403* "You are not allowed to generate embeddings from this model".
/// Before the fix this fell through to the 401/403 auth branch and told the
/// user to "enter a valid key" even though the key was fine — the model is the
/// problem. It must classify as MODEL_INCOMPATIBLE, ahead of the auth branch.
#[test]
fn classify_embed_probe_403_not_an_embeddings_model_is_model_incompatible_not_auth() {
    for detail in [
        r#"openai embeddings returned HTTP 403 Forbidden: {"error":{"message":"You are not allowed to generate embeddings from this model","type":"invalid_request_error","param":null,"code":null}}"#,
        r#"Embedding API error (403 Forbidden): {"error":{"message":"This is not an embedding model"}}"#,
        r#"openai embeddings returned HTTP 403 Forbidden: {"error":{"message":"unsupported model for embedding"}}"#,
    ] {
        assert_eq!(
            reject_code(EmbedProbe::Failed(detail.into())).as_deref(),
            Some("EMBEDDINGS_MODEL_INCOMPATIBLE"),
            "403 model-rejection must be model-incompatible, not auth: {detail}"
        );
    }
}

/// Issue #5116 (security) — a genuine bad key (401 "Incorrect API key
/// provided: sk-…") must STILL classify as auth, but the surfaced payload must
/// never carry the key: neither the message nor the redacted detail may
/// contain an `sk-` substring.
#[test]
fn classify_embed_probe_401_bad_key_is_auth_and_redacts_key() {
    let detail = r#"openai embeddings returned HTTP 401 Unauthorized: {"error":{"message":"Incorrect API key provided: sk-proj-ABC123def456GHI789jkl012MNO. You can find your API key at https://platform.openai.com/account/api-keys.","type":"invalid_request_error","param":null,"code":"invalid_api_key"}}"#;
    let rpc = classify_embed_probe(EmbedProbe::Failed(detail.into()))
        .expect("bad key must reject the save");
    assert_eq!(
        rpc.value.get("error").and_then(|v| v.as_str()),
        Some("EMBEDDINGS_AUTH_FAILED"),
        "a genuine 401 bad key must stay classified as auth"
    );
    // Nothing in the surfaced payload may leak the key.
    let surfaced = serde_json::to_string(&rpc.value).unwrap();
    assert!(
        !surfaced.contains("sk-"),
        "surfaced payload must not contain any sk- key material: {surfaced}"
    );
    assert!(
        rpc.value
            .get("detail")
            .and_then(|v| v.as_str())
            .map(|d| d.contains("[redacted-key]"))
            .unwrap_or(false),
        "the detail should keep a redaction marker for support diagnosis"
    );
}

/// The redaction helper strips whole OpenAI-style keys (incl. the modern
/// `sk-proj-…` form) and bearer tokens, leaving no `sk-` prefix behind.
#[test]
fn redact_secrets_removes_key_and_bearer_material() {
    let redacted =
        redact_secrets("key sk-proj-ABC123_def-456 and Authorization: Bearer tok-xyz.789");
    assert!(
        !redacted.contains("sk-"),
        "no sk- prefix survives: {redacted}"
    );
    assert!(!redacted.contains("tok-xyz.789"), "bearer token stripped");
    assert!(redacted.contains("[redacted-key]"));
    assert!(redacted.contains("Bearer [redacted]"));
    // Non-secret text is preserved.
    assert!(redacted.contains("Authorization:"));
}

/// Issue #5017 — a transport-level failure (DNS / refused connection) is a
/// reachability problem, distinct from a server that answered. Timeouts fall
/// in the same bucket.
#[test]
fn classify_embed_probe_distinguishes_unreachable() {
    for detail in [
        "openai embeddings request to http://127.0.0.1:9/v1/embeddings failed: connection refused",
        "error trying to connect: dns error: failed to lookup address information",
    ] {
        assert_eq!(
            reject_code(EmbedProbe::Failed(detail.into())).as_deref(),
            Some("EMBEDDINGS_ENDPOINT_UNREACHABLE"),
            "detail should classify as unreachable: {detail}"
        );
    }
    // A timeout is a reachability problem too.
    assert_eq!(
        reject_code(EmbedProbe::TimedOut).as_deref(),
        Some("EMBEDDINGS_ENDPOINT_UNREACHABLE")
    );
}

/// Issue #5017 — a length guard trip (endpoint ignored the `dimensions`
/// param and returned its native size) is a dimension problem, not a generic
/// failure, so the user knows to fix the dimensions field.
#[test]
fn classify_embed_probe_distinguishes_dimension_mismatch() {
    assert_eq!(
        reject_code(EmbedProbe::Failed(
            "openai embed dimension mismatch: expected 1024, got 3072".into()
        ))
        .as_deref(),
        Some("EMBEDDINGS_DIMENSION_MISMATCH")
    );
}

/// Issue #5017 regression — the request the app sends is correct: a conformant
/// OpenAI-compatible `POST /v1/embeddings` host (right path, the user's model,
/// Bearer key, `{"input":[…],"model":…}` body) verifies successfully. Builds
/// the live custom provider with the endpoint's known width and drives it
/// against a mock that echoes the OpenAI embeddings wire shape, asserting the
/// captured request AND that the probe classifies it as a pass.
#[tokio::test]
async fn conformant_custom_endpoint_verifies_and_sends_expected_request() {
    use std::sync::{Arc, Mutex};

    use axum::{
        extract::State,
        routing::{get, post},
        Json, Router,
    };

    #[derive(Clone, Default)]
    struct Captured {
        auth: Arc<Mutex<Option<String>>>,
        body: Arc<Mutex<Option<serde_json::Value>>>,
    }

    let captured = Captured::default();
    let app = Router::new()
        .route(
            "/v1/embeddings",
            post(
                |State(cap): State<Captured>,
                 headers: axum::http::HeaderMap,
                 Json(body): Json<serde_json::Value>| async move {
                    *cap.auth.lock().unwrap() = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    *cap.body.lock().unwrap() = Some(body);
                    Json(serde_json::json!({
                        "object": "list",
                        "data": [{
                            "object": "embedding",
                            "index": 0,
                            "embedding": [0.1_f32, 0.2, 0.3, 0.4]
                        }],
                        "model": "my-embed",
                    }))
                },
            ),
        )
        .route(
            "/v1/models",
            get(|| async { Json(serde_json::json!({"data": []})) }),
        )
        .with_state(captured.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "http://127.0.0.1:{}/v1",
        listener.local_addr().unwrap().port()
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // The live provider validates returned vectors against its configured width.
    // Use the mock endpoint's known width; setup-time custom probing discovers
    // that width separately before saving the live configuration.
    let embedder = create_embedding_provider_with_credentials(
        "custom",
        "gpt-5-mini",
        4,
        "sk-secret-key",
        Some(&base),
    )
    .expect("provider builds");

    let vectors = embedder
        .embed(&["connection test"])
        .await
        .expect("conformant endpoint must verify");
    let probe_dims = vectors.first().map(|v| v.len()).unwrap_or(0);
    assert_eq!(
        probe_dims, 4,
        "auto-detected the endpoint's real vector length"
    );

    // The probe policy accepts the returned vector.
    assert!(
        classify_embed_probe(EmbedProbe::Returned(vectors)).is_none(),
        "a usable vector from a conformant endpoint must pass verification"
    );

    // The exact request: user's model forwarded + Bearer key present.
    assert_eq!(
        captured.auth.lock().unwrap().as_deref(),
        Some("Bearer sk-secret-key"),
        "API key must be sent on the test-embed request"
    );
    let body = captured
        .body
        .lock()
        .unwrap()
        .clone()
        .expect("body captured");
    assert_eq!(
        body["model"], "gpt-5-mini",
        "user-supplied model must be forwarded"
    );
    assert_eq!(
        body["input"],
        serde_json::json!(["connection test"]),
        "input is the OpenAI array-of-strings shape"
    );
}

/// #5356: the managed paths build the cloud embedder through the
/// config-aware factory, so the bearer resolver reads the config-scoped
/// credential store. With no stored `app-session` token the resolver
/// short-circuits to the backend-session error *before* any network call
/// (privacy defaults to `Standard`, so egress is allowed and the failure is
/// the missing session, not a local-only block). Also covers the rerouted
/// `test_connection` construction line.
#[tokio::test]
async fn test_connection_managed_without_session_reports_no_backend_session() {
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    let out = test_connection(&config, Some("managed"), Some("voyage-3-large"), Some(1024))
        .await
        .expect("rpc returns Ok carrying a success flag");
    assert_eq!(
        out.value["success"],
        serde_json::json!(false),
        "managed test with no session must not pass"
    );
    let err = out.value["error"].as_str().unwrap_or_default();
    assert!(
        err.contains("No backend session"),
        "managed test with no session must report the backend-session error, got: {err}"
    );
}

/// Live `embed` (RPC) for managed also routes through the config-aware
/// factory; with no session it surfaces the same backend-session error.
#[tokio::test]
async fn embed_managed_without_session_errors_with_no_backend_session() {
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.memory.embedding_provider = "managed".to_string();
    config.memory.embedding_model = "voyage-3-large".to_string();
    let err = embed(&config, &["hello".to_string()])
        .await
        .expect_err("managed embed with no session must error");
    assert!(
        err.contains("No backend session"),
        "expected backend-session error, got: {err}"
    );
}

/// `provider_from_config` (reused by other domains for direct embedding)
/// routes managed construction through the config-aware factory too — pure
/// construction, so it builds the cloud provider without a network call.
#[test]
fn provider_from_config_managed_builds_cloud() {
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.memory.embedding_provider = "managed".to_string();
    config.memory.embedding_model = "voyage-3-large".to_string();
    config.memory.embedding_dimensions = 1024;
    let provider = provider_from_config(&config).expect("managed provider must build");
    assert_eq!(provider.name(), "cloud");
}
