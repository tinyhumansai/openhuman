//! Round 20 raw/E2E coverage for inference compatible/provider-admin leftovers.
//!
//! This suite uses loopback HTTP mocks and temp PATH scripts only. It must not
//! call host Ollama, LM Studio, Python, Piper, Whisper, or model binaries.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::openhuman::config::schema::cloud_providers::{
    AuthStyle as CloudAuthStyle, CloudProviderCreds,
};
use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use openhuman_core::openhuman::inference::local::ops::{
    local_ai_chat, local_ai_should_react, LocalAiChatMessage,
};
use openhuman_core::openhuman::inference::local::LocalAiService;
use openhuman_core::openhuman::inference::provider::compatible::{
    AuthStyle as CompatibleAuthStyle, OpenAiCompatibleProvider,
};
use openhuman_core::openhuman::inference::provider::factory::{
    auth_key_for_slug, create_chat_provider_from_string, provider_for_role,
};
use openhuman_core::openhuman::inference::provider::{
    create_resilient_provider, create_routed_provider, list_configured_models, ChatMessage,
    ChatRequest, Provider, ProviderDelta,
};
use openhuman_core::openhuman::tools::ToolSpec;

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<(String, Option<String>, Value)>>>,
    models: Arc<Mutex<Vec<String>>>,
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: validation runs this integration test with --test-threads=1.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }

    fn unset(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: validation runs this integration test with --test-threads=1.
        unsafe { std::env::remove_var(key) };
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => {
                // SAFETY: validation runs this integration test with --test-threads=1.
                unsafe { std::env::set_var(self.key, value) }
            }
            None => {
                // SAFETY: validation runs this integration test with --test-threads=1.
                unsafe { std::env::remove_var(self.key) }
            }
        }
    }
}

/// Serialize tests in this binary that mutate process-global env
/// (OPENHUMAN_WORKSPACE / OPENHUMAN_OLLAMA_BASE_URL / PATH / OLLAMA_BIN …). The
/// `EnvVarGuard` restores values on drop but provides no mutual exclusion, so
/// under cargo-llvm-cov's default multi-threaded run the tests clobber each
/// other's env (e.g. one test's workspace/ollama base leaking into another),
/// producing order-dependent failures. One lock makes the env sections atomic.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[tokio::test]
async fn compatible_native_leftovers_cover_tool_history_function_call_and_stream_ordering() {
    let (base, state) = serve_mock().await;
    let provider = OpenAiCompatibleProvider::new_with_user_agent(
        "custom_openai",
        &format!("{base}/v1"),
        Some("sk-round20-secret"),
        CompatibleAuthStyle::Bearer,
        "round20-agent",
    );
    let tools = vec![ToolSpec {
        name: "lookup".to_string(),
        description: "lookup things".to_string(),
        parameters: json!({"type": "object"}),
    }];

    let mut assistant = ChatMessage::assistant("not-json-assistant");
    assistant.extra_metadata = Some(json!({"reasoning_content": "metadata reasoning"}));
    let response = provider
        .chat(
            ChatRequest {
                messages: &[
                    ChatMessage::tool(
                        json!({"tool_call_id":"orphan","content":"drop me"}).to_string(),
                    ),
                    assistant,
                    ChatMessage::assistant(
                        json!({
                            "content": "two calls",
                            "tool_calls": [
                                {"id":"answered","name":"lookup","arguments":"{\"keep\":true}"},
                                {"id":"dangling","name":"lookup","arguments":"{\"drop\":true}"}
                            ]
                        })
                        .to_string(),
                    ),
                    ChatMessage::tool(
                        json!({"tool_call_id":"answered","content":"kept"}).to_string(),
                    ),
                    ChatMessage::tool(
                        json!({"tool_call_id":"stray","content":"dropped"}).to_string(),
                    ),
                    ChatMessage::user("native leftovers"),
                ],
                tools: Some(&tools),
                stream: None,
                max_tokens: None,
            },
            "function-call-model",
            0.4,
        )
        .await
        .expect("function call fallback");
    assert_eq!(response.text.as_deref(), Some("function text"));
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].name, "legacy_lookup");
    assert_eq!(
        response.tool_calls[0].arguments,
        r#"{"from":"function_call"}"#
    );
    let usage = response.usage.expect("standard usage fallback");
    assert_eq!(usage.input_tokens, 11);
    assert_eq!(usage.output_tokens, 7);
    assert_eq!(usage.cached_input_tokens, 3);

    let content_json = provider
        .chat(
            ChatRequest {
                messages: &[ChatMessage::user("content json")],
                tools: Some(&tools),
                stream: None,
                max_tokens: None,
            },
            "content-json-tools",
            0.4,
        )
        .await
        .expect("content encoded tool calls");
    assert_eq!(content_json.text.as_deref(), Some("encoded text"));
    assert_eq!(content_json.tool_calls.len(), 1);
    assert_eq!(content_json.tool_calls[0].name, "lookup");
    assert_eq!(
        content_json.tool_calls[0].arguments,
        r#"{"from":"content"}"#
    );

    let (tx, mut rx) = tokio::sync::mpsc::channel::<ProviderDelta>(16);
    let streamed = provider
        .chat(
            ChatRequest {
                messages: &[ChatMessage::user("stream ordering")],
                tools: Some(&tools),
                stream: Some(&tx),
                max_tokens: None,
            },
            "stream-out-of-order-tool",
            0.4,
        )
        .await
        .expect("out-of-order tool stream");
    drop(tx);
    assert_eq!(streamed.text.as_deref(), Some("done"));
    assert_eq!(streamed.reasoning_content.as_deref(), Some("ponder"));
    assert_eq!(streamed.tool_calls.len(), 1);
    assert_eq!(streamed.tool_calls[0].id, "call_late");
    assert_eq!(streamed.tool_calls[0].arguments, r#"{"a":1,"b":2}"#);
    let deltas = collect_deltas(&mut rx).await;
    assert!(deltas.iter().any(
        |d| matches!(d, ProviderDelta::ToolCallStart { call_id, tool_name }
            if call_id == "call_late" && tool_name == "lookup")
    ));
    assert!(deltas.iter().any(
        |d| matches!(d, ProviderDelta::ToolCallArgsDelta { call_id, delta }
            if call_id == "call_late" && delta.contains("\"a\":1"))
    ));
    assert!(deltas
        .iter()
        .any(|d| matches!(d, ProviderDelta::ThinkingDelta { delta } if delta == "ponder")));

    let raw_chunks = provider
        .stream_chat_with_system(
            Some("policy"),
            "count tokens",
            "raw-stream-two-lines",
            0.2,
            openhuman_core::openhuman::inference::provider::traits::StreamOptions::new(true),
        )
        .collect::<Vec<_>>()
        .await;
    assert!(raw_chunks
        .iter()
        .any(|chunk| chunk.as_ref().is_ok_and(|c| !c.delta.is_empty())));
    assert!(raw_chunks
        .iter()
        .any(|chunk| chunk.as_ref().is_ok_and(|c| c.is_final)));

    let seen = state.requests.lock().expect("requests");
    let native_body = seen
        .iter()
        .find(|(_, _, body)| body["model"] == "function-call-model")
        .expect("native body")
        .2
        .clone();
    let messages = native_body["messages"].as_array().expect("messages");
    assert_ne!(messages[0]["role"], "tool");
    assert_eq!(
        messages
            .iter()
            .filter(|m| m["role"] == "tool")
            .collect::<Vec<_>>()
            .len(),
        1
    );
    assert!(messages
        .iter()
        .any(|message| message["reasoning_content"] == "metadata reasoning"));
}

#[tokio::test]
async fn provider_ops_leftovers_cover_model_listing_error_shapes_and_auth_styles() {
    let _env = env_lock();
    let (base, state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.local_ai.base_url = Some(base.clone());
    config.cloud_providers = vec![
        provider_entry(
            "missing-data-id",
            "missing-data",
            &format!("{base}/missing-data"),
            CloudAuthStyle::None,
            None,
        ),
        provider_entry(
            "wrong-data-id",
            "wrong-data",
            &format!("{base}/wrong-data"),
            CloudAuthStyle::None,
            None,
        ),
        provider_entry(
            "bad-json-id",
            "bad-json",
            &format!("{base}/bad-json"),
            CloudAuthStyle::None,
            None,
        ),
        provider_entry(
            "error-string-id",
            "error-string",
            &format!("{base}/error-string"),
            CloudAuthStyle::None,
            None,
        ),
        provider_entry(
            "not-found-id",
            "not-found",
            &format!("{base}/not-found"),
            CloudAuthStyle::None,
            None,
        ),
        provider_entry(
            "anthropic-id",
            "anthropic-list",
            &format!("{base}/anthropic-list"),
            CloudAuthStyle::Anthropic,
            None,
        ),
        provider_entry(
            "bearer-id",
            "bearer-list",
            &format!("{base}/bearer-list"),
            CloudAuthStyle::Bearer,
            None,
        ),
    ];
    config.save().await.expect("save config");
    let auth = AuthService::from_config(&config);
    auth.store_provider_token(
        &auth_key_for_slug("anthropic-list"),
        DEFAULT_AUTH_PROFILE_NAME,
        "sk-anthropic-list",
        HashMap::new(),
        true,
    )
    .expect("store anthropic key");
    auth.store_provider_token(
        "bearer-list",
        DEFAULT_AUTH_PROFILE_NAME,
        "sk-bearer-list",
        HashMap::new(),
        true,
    )
    .expect("store legacy bearer key");

    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());
    let _ollama_base = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", &base);

    let empty_id = list_configured_models("   ")
        .await
        .expect_err("empty provider id");
    assert_eq!(empty_id, "provider_id must not be empty");

    let unknown = list_configured_models("does-not-exist")
        .await
        .expect_err("unknown provider id");
    assert!(unknown.contains("no cloud provider"));

    // PR #2959 reverted the list_models 404 suppression: a 404 from the
    // /models endpoint no longer returns a synthetic `{models: [], unsupported:
    // true}` success — it surfaces as a real error so the failure fires to
    // Sentry and gets a root-cause fix (e.g. a wrong base URL).
    let not_found_err = list_configured_models("not-found")
        .await
        .expect_err("404 list_models now surfaces as an error");
    assert!(
        not_found_err.contains("provider returned 404"),
        "404 list_models error should surface the status: {not_found_err:?}"
    );

    let missing_data = list_configured_models("missing-data")
        .await
        .expect_err("missing data field");
    assert!(missing_data.contains("missing `data` or `models` field"));

    let wrong_data = list_configured_models("wrong-data")
        .await
        .expect_err("wrong data type");
    assert!(wrong_data.contains("has `data` field but it is object"));

    let bad_json = list_configured_models("bad-json")
        .await
        .expect_err("invalid json body");
    assert!(bad_json.contains("failed to parse JSON"));

    let error_string = list_configured_models("error-string")
        .await
        .expect_err("200 error payload");
    assert!(error_string.contains("provider returned error payload"));
    assert!(!error_string.contains("sk-error-secret"));

    let anthropic = list_configured_models("anthropic-list")
        .await
        .expect("anthropic list")
        .value;
    assert_eq!(anthropic["models"][0]["id"], "anthropic-model");

    let bearer = list_configured_models("bearer-id")
        .await
        .expect("id lookup and legacy key")
        .value;
    assert_eq!(bearer["models"][0]["context_window"], 32768);

    let seen = state.requests.lock().expect("requests");
    assert!(seen.iter().any(|(path, auth, _)| {
        path == "/anthropic-list/models" && auth.as_deref() == Some("sk-anthropic-list")
    }));
    assert!(seen.iter().any(|(path, auth, _)| {
        path == "/bearer-list/models" && auth.as_deref() == Some("Bearer sk-bearer-list")
    }));
    assert!(seen
        .iter()
        .any(|(path, auth, _)| path == "/not-found/models" && auth.is_none()));
}

#[tokio::test]
async fn factory_leftovers_cover_routes_byok_fail_closed_local_and_cloud_edges() {
    let _env = env_lock();
    let (base, _state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.local_ai.base_url = Some(format!("{base}/ollama"));
    config.local_ai.api_key = Some("lmstudio-key".to_string());
    config.default_model = Some("deepseek-v4-pro".to_string());
    config.cloud_providers = vec![
        provider_entry(
            "oh",
            "openhuman",
            "https://api.openhuman.ai/v1",
            CloudAuthStyle::OpenhumanJwt,
            None,
        ),
        provider_entry(
            "custom-id",
            "custom",
            &format!("{base}/custom/v1"),
            CloudAuthStyle::Bearer,
            Some("custom-default"),
        ),
        provider_entry(
            "none-id",
            "noauth",
            &format!("{base}/noauth/v1"),
            CloudAuthStyle::None,
            Some("none-default"),
        ),
        provider_entry(
            "anthropic-id",
            "anthropic",
            &format!("{base}/anthropic/v1"),
            CloudAuthStyle::Anthropic,
            Some("claude-default"),
        ),
        provider_entry(
            "empty-id",
            "empty-default",
            &format!("{base}/empty/v1"),
            CloudAuthStyle::Bearer,
            None,
        ),
    ];
    config.primary_cloud = Some("oh".to_string());
    config.chat_provider = Some("cloud".to_string());
    config.reasoning_provider = Some("custom:reasoning-v1".to_string());
    config.coding_provider = Some("ollama:local-code".to_string());
    config.temperature_unsupported_models = vec!["cold-*".to_string()];
    config.save().await.expect("save config");

    let auth = AuthService::from_config(&config);
    auth.store_provider_token(
        APP_SESSION_PROVIDER,
        DEFAULT_AUTH_PROFILE_NAME,
        "session-token",
        HashMap::new(),
        true,
    )
    .expect("store session");
    auth.store_provider_token(
        &auth_key_for_slug("custom"),
        DEFAULT_AUTH_PROFILE_NAME,
        "sk-custom",
        HashMap::new(),
        true,
    )
    .expect("store provider key");
    auth.store_provider_token(
        &auth_key_for_slug("anthropic"),
        DEFAULT_AUTH_PROFILE_NAME,
        "sk-anthropic",
        HashMap::new(),
        true,
    )
    .expect("store anthropic key");

    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());

    assert_eq!(provider_for_role("chat", &config), "custom:reasoning-v1");
    assert_eq!(provider_for_role("memory", &config), "openhuman");
    assert_eq!(provider_for_role("coding", &config), "ollama:local-code");

    let (ollama, ollama_model) =
        create_chat_provider_from_string("chat", "ollama: local-model @0.25", &config)
            .expect("ollama provider");
    assert_eq!(ollama_model, "local-model");
    assert_eq!(
        ollama
            .chat_with_system(None, "hello", &ollama_model, 0.9)
            .await
            .expect("ollama compatible chat"),
        "factory ollama"
    );

    let (lmstudio, lm_model) =
        create_chat_provider_from_string("chat", "lmstudio: loaded-chat @0.15", &config)
            .expect("lm studio provider");
    assert_eq!(lm_model, "loaded-chat");
    assert_eq!(
        lmstudio
            .chat_with_system(None, "hello", &lm_model, 0.9)
            .await
            .expect("lm studio compatible chat"),
        "factory lmstudio"
    );

    let (cloud, cloud_model) =
        create_chat_provider_from_string("chat", "custom:reasoning-v1@0.2", &config)
            .expect("abstract remapped cloud provider");
    assert_eq!(cloud_model, "custom-default");
    assert_eq!(
        cloud
            .chat_with_system(None, "hello", &cloud_model, 0.9)
            .await
            .expect("cloud compatible chat"),
        "factory cloud"
    );

    let (anthropic, anthropic_model) =
        create_chat_provider_from_string("chat", "anthropic:claude-3", &config)
            .expect("anthropic compatible provider");
    assert_eq!(anthropic_model, "claude-3");
    assert_eq!(
        anthropic
            .chat_with_system(None, "hello", &anthropic_model, 0.9)
            .await
            .expect("anthropic compatible chat"),
        "factory anthropic"
    );

    let (noauth, noauth_model) = create_chat_provider_from_string("chat", "noauth:", &config)
        .expect("empty model falls back to entry default");
    assert_eq!(noauth_model, "none-default");
    assert_eq!(
        noauth
            .chat_with_system(None, "hello", &noauth_model, 0.9)
            .await
            .expect("no-auth compatible chat"),
        "factory noauth"
    );

    let empty_model = err_string(create_chat_provider_from_string(
        "chat",
        "empty-default:",
        &config,
    ));
    assert!(empty_model.to_string().contains("no model configured"));

    let unknown_slug = err_string(create_chat_provider_from_string(
        "chat",
        "missing:model",
        &config,
    ));
    assert!(unknown_slug
        .to_string()
        .contains("no cloud provider configured"));

    let empty_local = err_string(create_chat_provider_from_string(
        "chat",
        "ollama:   ",
        &config,
    ));
    assert!(empty_local.to_string().contains("empty model"));

    let invalid = err_string(create_chat_provider_from_string(
        "chat",
        "not-a-provider",
        &config,
    ));
    assert!(invalid.to_string().contains("unrecognised provider string"));

    let mut byok = config.clone();
    byok.inference_url = Some("https://direct.example.test/v1".to_string());
    byok.primary_cloud = Some("oh".to_string());
    byok.chat_provider = None;
    byok.reasoning_provider = None;
    byok.coding_provider = None;
    assert!(provider_for_role("chat", &byok).contains("__byok_incomplete__"));
    let byok_err = err_string(create_chat_provider_from_string(
        "chat",
        &provider_for_role("chat", &byok),
        &byok,
    ));
    assert!(byok_err.to_string().contains("BYOK_INCOMPLETE"));

    let _fallback = create_resilient_provider(
        Some(&format!("{base}/custom/v1")),
        config.api_url.as_deref(),
        Some("sk-direct"),
        &config.reliability,
    )
    .expect("resilient custom provider");
    let _routed = create_routed_provider(
        None,
        config.api_url.as_deref(),
        None,
        &config.reliability,
        &config.model_routes,
        "reasoning-v1",
    )
    .expect("routed provider without routes");
}

#[tokio::test]
async fn local_admin_leftovers_cover_status_binary_paths_lmstudio_and_ops_skip_branches() {
    let _env = env_lock();
    let (base, _state) = serve_mock().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = temp_config(&tmp);
    config.local_ai.runtime_enabled = true;
    config.local_ai.opt_in_confirmed = true;
    config.local_ai.base_url = Some(base.clone());
    config.local_ai.chat_model_id = "round20-chat".to_string();
    config.local_ai.embedding_model_id = "round20-embed".to_string();
    config.local_ai.vision_model_id = "round20-vision".to_string();
    config.local_ai.selected_tier = Some("custom".to_string());
    config.local_ai.preload_embedding_model = true;
    config.local_ai.preload_vision_model = true;
    config.local_ai.preload_stt_model = false;
    config.local_ai.preload_tts_voice = false;

    let scripts = tempdir().expect("scripts");
    let ollama = write_stub_script(
        scripts.path(),
        "ollama",
        "#!/bin/sh\nprintf 'ollama version mock\\n'\n",
    );
    write_stub_script(scripts.path(), "python", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "python3", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "mlx_lm.generate", "#!/bin/sh\nexit 42\n");
    write_stub_script(scripts.path(), "piper", "#!/bin/sh\nexit 42\n");
    let _path = EnvVarGuard::set("PATH", scripts.path());
    let _workspace = EnvVarGuard::set("OPENHUMAN_WORKSPACE", config.config_path.parent().unwrap());
    let _ollama_base = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", &base);
    let _ollama_bin = EnvVarGuard::set("OLLAMA_BIN", &ollama);
    let _piper_bin = EnvVarGuard::unset("PIPER_BIN");
    let _whisper_bin = EnvVarGuard::unset("WHISPER_BIN");

    let service = LocalAiService::new(&config);
    let status = service.status();
    assert_eq!(status.state, "idle");

    let diagnostics = service.diagnostics(&config).await.expect("diagnostics");
    assert_eq!(
        diagnostics["ollama_binary_path"].as_str(),
        Some(ollama.to_string_lossy().as_ref())
    );
    assert_eq!(diagnostics["expected"]["vision_found"], false);
    assert!(diagnostics["installed_models"]
        .as_array()
        .unwrap()
        .iter()
        .any(|model| model["name"] == "bge-m3" && model["eligibility"]["status"] == "ok"));

    let mut bad_show = config.clone();
    bad_show.local_ai.base_url = Some(format!("{base}/show-bad"));
    let bad_show_diag = service.diagnostics(&bad_show).await.expect("bad show diag");
    // `/show-bad/api/show` returns no `context_length`, so context can't be
    // determined — the verdict is `unknown` (not a rejection), not
    // `below_minimum`, per `evaluate_context(None)`.
    assert!(bad_show_diag["installed_models"]
        .as_array()
        .unwrap()
        .iter()
        .any(|model| model["eligibility"]["status"] == "unknown"));

    let mut lm_reachable_error = config.clone();
    lm_reachable_error.local_ai.provider = "lmstudio".to_string();
    lm_reachable_error.local_ai.base_url = Some(format!("{base}/lm-error-object/v1"));
    lm_reachable_error.local_ai.chat_model_id = "loaded-chat".to_string();
    let lm = service
        .diagnostics(&lm_reachable_error)
        .await
        .expect("lm error object");
    assert_eq!(lm["lm_studio_running"], true);
    let lm_issue = lm["issues"][0].as_str().unwrap();
    assert!(
        lm_issue.contains("Failed to list LM Studio models")
            || lm_issue.contains("no models are loaded")
    );

    let empty_reaction = local_ai_should_react(&config, "   ", "slack")
        .await
        .expect("empty reaction")
        .value;
    assert!(!empty_reaction.should_react);

    let mut disabled = config.clone();
    disabled.local_ai.runtime_enabled = false;
    let skipped_reaction = local_ai_should_react(&disabled, "good news", "discord")
        .await
        .expect("disabled reaction")
        .value;
    assert!(!skipped_reaction.should_react);

    let bad_role = local_ai_chat(
        &config,
        vec![LocalAiChatMessage {
            role: " tool ".to_string(),
            content: "tool output is invalid here".to_string(),
        }],
        None,
    )
    .await
    .expect_err("tool role rejected by local ops");
    assert!(bad_role.contains("unsupported message role"));
}

async fn collect_deltas(rx: &mut tokio::sync::mpsc::Receiver<ProviderDelta>) -> Vec<ProviderDelta> {
    let mut out = Vec::new();
    while let Some(delta) = rx.recv().await {
        out.push(delta);
    }
    out
}

async fn serve_mock() -> (String, MockState) {
    let state = MockState::default();
    *state.models.lock().expect("models") = vec![
        "round20-chat".to_string(),
        "round20-embed".to_string(),
        "round20-vision".to_string(),
        "gemma3:1b-it-qat".to_string(),
        "bge-m3".to_string(),
        "loaded-chat".to_string(),
    ];
    let app = Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/models", get(models))
        .route("/custom/v1/chat/completions", post(factory_chat))
        .route("/noauth/v1/chat/completions", post(factory_chat))
        .route("/anthropic/v1/chat/completions", post(factory_chat))
        .route("/ollama/v1/chat/completions", post(factory_chat))
        .route("/lmstudio/v1/chat/completions", post(factory_chat))
        .route("/missing-data/models", get(missing_data_models))
        .route("/wrong-data/models", get(wrong_data_models))
        .route("/bad-json/models", get(bad_json_models))
        .route("/error-string/models", get(error_string_models))
        .route("/not-found/models", get(not_found_models))
        .route("/anthropic-list/models", get(anthropic_list_models))
        .route("/bearer-list/models", get(bearer_list_models))
        .route("/api/tags", get(ollama_tags))
        .route("/api/show", post(ollama_show))
        .route("/show-bad/api/tags", get(ollama_tags))
        .route("/show-bad/api/show", post(ollama_show_bad))
        .route("/lm-error-object/v1/models", get(lm_error_object_models))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve mock");
    });
    (format!("http://{addr}"), state)
}

async fn chat_completions(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    remember(&state, "/v1/chat/completions", &headers, body.clone());
    match body["model"].as_str().unwrap_or_default() {
        "function-call-model" => Json(json!({
            "choices": [{
                "message": {
                    "content": "function text",
                    "function_call": {
                        "name": "legacy_lookup",
                        "arguments": { "from": "function_call" }
                    }
                }
            }],
            "usage": {
                "prompt_tokens": 11,
                "completion_tokens": 7,
                "prompt_tokens_details": { "cached_tokens": 3 }
            }
        }))
        .into_response(),
        "content-json-tools" => Json(json!({
            "choices": [{
                "message": {
                    "content": "{\"content\":\"encoded text\",\"tool_calls\":[{\"id\":\"call_content\",\"type\":\"function\",\"function\":{\"name\":\"lookup\",\"arguments\":{\"from\":\"content\"}}}]}"
                }
            }]
        }))
        .into_response(),
        "stream-out-of-order-tool" => sse_response([
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"a\":1"}}]}}]}),
            json!({"choices":[{"delta":{"reasoning_content":"ponder","tool_calls":[{"index":0,"id":"call_late","function":{"name":"lookup"}}]}}]}),
            json!({"choices":[{"delta":{"content":"done","tool_calls":[{"index":0,"function":{"arguments":",\"b\":2}"}}]}}]}),
            json!({"choices":[],"usage":{"prompt_tokens":2,"completion_tokens":1}}),
        ]),
        "raw-stream-two-lines" => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .body(Body::from(
                "data: {\"choices\":[{\"delta\":{\"content\":\"abc\"}}]}\n\n\
                 data: {\"choices\":[{\"delta\":{\"content\":\"defgh\"}}]}\n\n\
                 data: [DONE]\n\n",
            ))
            .expect("raw stream")
            .into_response(),
        _ => Json(json!({"choices":[{"message":{"content":"default compatible"}}]})).into_response(),
    }
}

async fn factory_chat(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let path = headers
        .get("x-forwarded-path")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("/factory/chat")
        .to_string();
    remember(&state, &path, &headers, body.clone());
    let model = body["model"].as_str().unwrap_or_default();
    let content = match model {
        "local-model" => "factory ollama",
        "loaded-chat" => "factory lmstudio",
        "custom-default" => "factory cloud",
        "claude-3" => "factory anthropic",
        "none-default" => "factory noauth",
        _ => "factory default",
    };
    Json(json!({"choices":[{"message":{"content":content}}]})).into_response()
}

async fn models(State(state): State<MockState>) -> impl IntoResponse {
    let models = state
        .models
        .lock()
        .expect("models")
        .iter()
        .map(|id| json!({"id": id, "owned_by": "round20", "context_window": 8192}))
        .collect::<Vec<_>>();
    Json(json!({"object": "list", "data": models}))
}

async fn missing_data_models() -> impl IntoResponse {
    Json(json!({"object": "list", "items": []}))
}

async fn wrong_data_models() -> impl IntoResponse {
    Json(json!({"object": "error", "data": {"message": "wrong shape"}}))
}

async fn bad_json_models() -> impl IntoResponse {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("<html>not json</html>"))
        .expect("bad json")
}

async fn error_string_models() -> impl IntoResponse {
    Json(json!({"error": "failed with sk-error-secret"}))
}

async fn not_found_models(State(state): State<MockState>, headers: HeaderMap) -> impl IntoResponse {
    remember(&state, "/not-found/models", &headers, Value::Null);
    (StatusCode::NOT_FOUND, "no model list").into_response()
}

async fn anthropic_list_models(
    State(state): State<MockState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    remember(&state, "/anthropic-list/models", &headers, Value::Null);
    Json(json!({"object":"list","data":[{"id":"anthropic-model"}]}))
}

async fn bearer_list_models(
    State(state): State<MockState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    remember(&state, "/bearer-list/models", &headers, Value::Null);
    Json(json!({"object":"list","data":[{"id":"bearer-model","context_length":32768}]}))
}

async fn ollama_tags(State(state): State<MockState>) -> impl IntoResponse {
    let models = state
        .models
        .lock()
        .expect("models")
        .iter()
        .map(|name| json!({"name": name, "model": name, "size": 1234}))
        .collect::<Vec<_>>();
    Json(json!({"models": models}))
}

async fn ollama_show(Json(body): Json<Value>) -> impl IntoResponse {
    let model = body
        .get("model")
        .or_else(|| body.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let context = match model {
        "round20-embed" | "bge-m3" => 8192,
        "round20-chat" => 4096,
        "round20-vision" => 2048,
        _ => 1024,
    };
    Json(json!({
        "model_info": {
            "general.context_length": context,
            "llama.context_length": context
        }
    }))
    .into_response()
}

async fn ollama_show_bad() -> impl IntoResponse {
    Json(json!({"model_info": {"unrelated": true}}))
}

async fn lm_error_object_models() -> impl IntoResponse {
    Json(json!({"error": {"message": "server says nope"}}))
}

fn sse_response<const N: usize>(events: [Value; N]) -> Response<Body> {
    let mut body = String::new();
    for event in events {
        body.push_str("data: ");
        body.push_str(&event.to_string());
        body.push_str("\n\n");
    }
    body.push_str("data: [DONE]\n\n");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from(body))
        .expect("sse")
}

fn remember(state: &MockState, path: &str, headers: &HeaderMap, body: Value) {
    state
        .requests
        .lock()
        .expect("requests")
        .push((path.to_string(), auth_header(headers), body));
}

fn auth_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .or_else(|| headers.get("x-api-key"))
        .or_else(|| headers.get("x-custom-auth"))
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn provider_entry(
    id: &str,
    slug: &str,
    endpoint: &str,
    auth_style: CloudAuthStyle,
    default_model: Option<&str>,
) -> CloudProviderCreds {
    CloudProviderCreds {
        id: id.to_string(),
        slug: slug.to_string(),
        label: slug.to_string(),
        endpoint: endpoint.to_string(),
        auth_style,
        legacy_type: None,
        default_model: default_model.map(ToString::to_string),
    }
}

fn err_string<T>(result: anyhow::Result<T>) -> String {
    match result {
        Ok(_) => panic!("expected error"),
        Err(err) => err.to_string(),
    }
}

fn temp_config(tmp: &TempDir) -> Config {
    let root = tmp.path().join(".openhuman");
    std::fs::create_dir_all(root.join("workspace")).expect("workspace dir");
    let mut config = Config::default();
    config.config_path = root.join("config.toml");
    config.workspace_dir = root.join("workspace");
    config.secrets.encrypt = false;
    config.api_url = Some("http://127.0.0.1:9".to_string());
    config
}

fn write_stub_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
    }
    path
}
