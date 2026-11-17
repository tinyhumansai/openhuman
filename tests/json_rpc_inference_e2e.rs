//! JSON-RPC E2E cases for the inference namespace.

#[path = "support/rpc_e2e_harness.rs"]
mod harness;
use harness::*;

#[tokio::test]
async fn json_rpc_inference_status_reports_unreachable_without_spawning_a_runtime() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    // Every runtime binary OpenHuman used to launch is a stub that leaves a
    // marker when executed.
    let bin_dir = tmp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("bin dir");
    let spawn_marker = tmp.path().join("runtime-spawned.marker");
    for name in ["ollama", "lms", "mlx_lm.server", "piper"] {
        let path = bin_dir.join(name);
        std::fs::write(
            &path,
            format!("#!/bin/sh\ntouch '{}'\nexit 0\n", spawn_marker.display()),
        )
        .expect("write stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod stub");
        }
    }
    let original_path = std::env::var_os("PATH").unwrap_or_default();
    let mut path_entries = vec![bin_dir.clone()];
    path_entries.extend(std::env::split_paths(&original_path));
    let joined_path = std::env::join_paths(path_entries).expect("join PATH");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _ollama_bin_guard = EnvVarGuard::unset("OLLAMA_BIN");
    let _ollama_host_guard = EnvVarGuard::unset("OLLAMA_HOST");
    // Port 1 is never listening: the probe is refused immediately.
    let _ollama_url_guard = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", "http://127.0.0.1:1");
    let _path_guard = EnvVarGuard::set_to_path("PATH", Path::new(&joined_path));

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Saving local settings resets the cached probe verdict, so this test is
    // independent of whatever endpoint an earlier test left the singleton on.
    let update = post_json_rpc(
        &rpc_base,
        30,
        "openhuman.inference_update_local_settings",
        json!({
            "runtime_enabled": true,
            "opt_in_confirmed": true,
            "provider": "ollama",
            "base_url": "http://127.0.0.1:1",
            "chat_model_id": "llama3.1:8b"
        }),
    )
    .await;
    assert_no_jsonrpc_error(&update, "update_local_settings closed port");

    // The first poll schedules the probe; poll until it settles.
    let mut last = Value::Null;
    let mut state = String::new();
    for attempt in 0..50 {
        let status = post_json_rpc(
            &rpc_base,
            31 + attempt,
            "openhuman.inference_status",
            json!({}),
        )
        .await;
        let result = assert_no_jsonrpc_error(&status, "inference_status");
        let payload = result.get("result").unwrap_or(result).clone();
        state = payload
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        last = payload;
        if state == "unreachable" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(state, "unreachable", "status never settled: {last}");
    assert_eq!(
        last.get("error_category").and_then(Value::as_str),
        Some("server")
    );
    assert!(
        last.get("warning")
            .and_then(Value::as_str)
            .is_some_and(|w| w.contains("does not install or launch")),
        "warning should tell the user to start their own runtime: {last}"
    );
    // The configured model passes through unchanged (no allowlist redirect).
    assert_eq!(
        last.get("chat_model_id").and_then(Value::as_str),
        Some("llama3.1:8b")
    );
    for removed in [
        "download_progress",
        "downloaded_bytes",
        "total_bytes",
        "quantization",
    ] {
        assert!(
            last.get(removed).is_none(),
            "`{removed}` left on status: {last}"
        );
    }
    assert!(
        !spawn_marker.exists(),
        "OpenHuman must never launch a local runtime binary"
    );

    // The removed download / preset / device-profile RPCs are gone, legacy
    // names included.
    for (idx, method) in [
        "openhuman.inference_presets",
        "openhuman.inference_apply_preset",
        "openhuman.inference_device_profile",
        "openhuman.inference_assets_status",
        "openhuman.inference_downloads_progress",
        "openhuman.inference_download_asset",
        "openhuman.inference_install_piper",
        "openhuman.inference_piper_install_status",
        "openhuman.local_ai_presets",
        "openhuman.local_ai_download_asset",
    ]
    .into_iter()
    .enumerate()
    {
        let response = post_json_rpc(&rpc_base, 200 + idx as i64, method, json!({})).await;
        assert!(
            response.get("error").is_some(),
            "{method} should no longer be registered: {response}"
        );
    }
    assert!(!spawn_marker.exists());

    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_local_ai_lm_studio_settings_and_prompt() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _lm_env_guard = EnvVarGuard::unset("OPENHUMAN_LM_STUDIO_BASE_URL");
    let _lm_alias_env_guard = EnvVarGuard::unset("LM_STUDIO_BASE_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let lm_app = Router::new()
        .route(
            "/v1/models",
            get(|| async {
                Json(json!({
                    "object": "list",
                    "data": [
                        { "id": "local-model", "object": "model", "owned_by": "lm-studio" }
                    ]
                }))
            }),
        )
        .route(
            "/v1/chat/completions",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(
                    body.get("model").and_then(Value::as_str),
                    Some("local-model")
                );
                let roles: Vec<&str> = body
                    .get("messages")
                    .and_then(Value::as_array)
                    .map(|messages| {
                        messages
                            .iter()
                            .filter_map(|message| message.get("role").and_then(Value::as_str))
                            .collect()
                    })
                    .unwrap_or_default();
                assert_eq!(roles, vec!["system", "user"]);
                Json(json!({
                    "id": "chatcmpl-e2e",
                    "object": "chat.completion",
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "hello from lm studio e2e"
                        },
                        "finish_reason": "stop"
                    }],
                    "usage": {
                        "prompt_tokens": 7,
                        "completion_tokens": 5,
                        "total_tokens": 12
                    }
                }))
            }),
        );
    let (lm_addr, lm_join) = serve_on_ephemeral(lm_app).await;
    let lm_base = format!("http://{lm_addr}/v1");

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let update = post_json_rpc(
        &rpc_base,
        36,
        "openhuman.inference_update_local_settings",
        json!({
            "runtime_enabled": true,
            "opt_in_confirmed": true,
            "provider": "lm_studio",
            "base_url": lm_base,
            "model_id": "local-model",
            "chat_model_id": "local-model"
        }),
    )
    .await;
    let update_result = assert_no_jsonrpc_error(&update, "update_local_ai_settings");
    let config = update_result
        .get("result")
        .and_then(|value| value.get("config"))
        .expect("config snapshot should be wrapped with logs");
    assert_eq!(
        config
            .get("local_ai")
            .and_then(|local_ai| local_ai.get("provider"))
            .and_then(Value::as_str),
        Some("lm_studio")
    );
    assert_eq!(
        config
            .get("local_ai")
            .and_then(|local_ai| local_ai.get("opt_in_confirmed"))
            .and_then(Value::as_bool),
        Some(true)
    );

    let prompt = post_json_rpc(
        &rpc_base,
        38,
        "openhuman.inference_prompt",
        json!({
            "prompt": "hello",
            "max_tokens": 16,
            "no_think": true
        }),
    )
    .await;
    let prompt_result = assert_no_jsonrpc_error(&prompt, "lm_studio_prompt");
    assert_eq!(
        extract_string_outcome(prompt_result),
        "hello from lm studio e2e"
    );

    lm_join.abort();
    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_local_ai_ollama_settings_can_be_cleared_over_rpc() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let update = post_json_rpc(
        &rpc_base,
        390,
        "openhuman.inference_update_local_settings",
        json!({
            "provider": "ollama",
            "base_url": "http://127.0.0.1:11434"
        }),
    )
    .await;
    assert_no_jsonrpc_error(&update, "set ollama base_url over RPC");

    let clear = post_json_rpc(
        &rpc_base,
        391,
        "openhuman.inference_update_local_settings",
        json!({ "base_url": null }),
    )
    .await;
    let clear_result = assert_no_jsonrpc_error(&clear, "clear ollama base_url");
    assert!(clear_result
        .get("result")
        .and_then(|value| value.get("config"))
        .and_then(|config| config.get("local_ai"))
        .and_then(|local_ai| local_ai.get("base_url"))
        .is_none_or(Value::is_null));

    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_inference_namespace_lm_studio_prompt_and_status() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _lm_env_guard = EnvVarGuard::unset("OPENHUMAN_LM_STUDIO_BASE_URL");
    let _lm_alias_env_guard = EnvVarGuard::unset("LM_STUDIO_BASE_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let lm_app = Router::new()
        .route(
            "/v1/models",
            get(|| async {
                Json(json!({
                    "object": "list",
                    "data": [
                        { "id": "local-model", "object": "model", "owned_by": "lm-studio" }
                    ]
                }))
            }),
        )
        .route(
            "/v1/chat/completions",
            post(|Json(_body): Json<Value>| async move {
                Json(json!({
                    "id": "chatcmpl-inference-e2e",
                    "object": "chat.completion",
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "hello from inference namespace"
                        },
                        "finish_reason": "stop"
                    }],
                    "usage": {
                        "prompt_tokens": 7,
                        "completion_tokens": 5,
                        "total_tokens": 12
                    }
                }))
            }),
        );
    let (lm_addr, lm_join) = serve_on_ephemeral(lm_app).await;
    let lm_base = format!("http://{lm_addr}/v1");

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let update = post_json_rpc(
        &rpc_base,
        360,
        "openhuman.inference_update_local_settings",
        json!({
            "runtime_enabled": true,
            "opt_in_confirmed": true,
            "provider": "lm_studio",
            "base_url": lm_base,
            "model_id": "local-model",
            "chat_model_id": "local-model"
        }),
    )
    .await;
    assert_no_jsonrpc_error(&update, "update_local_ai_settings for inference namespace");

    let status = post_json_rpc(&rpc_base, 361, "openhuman.inference_status", json!({})).await;
    let status_result = assert_no_jsonrpc_error(&status, "inference_status");
    let status_payload = status_result.get("result").unwrap_or(status_result);
    assert_eq!(
        status_payload.get("provider").and_then(Value::as_str),
        Some("lm_studio")
    );

    let prompt = post_json_rpc(
        &rpc_base,
        362,
        "openhuman.inference_prompt",
        json!({
            "prompt": "hello",
            "max_tokens": 16,
            "no_think": true
        }),
    )
    .await;
    let prompt_result = assert_no_jsonrpc_error(&prompt, "inference_prompt");
    assert_eq!(
        extract_string_outcome(prompt_result),
        "hello from inference namespace"
    );

    let summarize = post_json_rpc(
        &rpc_base,
        363,
        "openhuman.inference_summarize",
        json!({
            "text": "summarize me",
            "max_tokens": 16
        }),
    )
    .await;
    let summarize_result = assert_no_jsonrpc_error(&summarize, "inference_summarize");
    assert_eq!(
        extract_string_outcome(summarize_result),
        "hello from inference namespace"
    );

    // openhuman.inference_update_model_settings — mutate `default_model`
    // through the RPC transport so a controller-registration or param-shape
    // regression surfaces here instead of in the settings-save UI flow.
    // (We assert on `default_model` because that field is exposed by
    // `inference_get_client_config`; `default_temperature` is not.)
    let model_update = post_json_rpc(
        &rpc_base,
        366,
        "openhuman.inference_update_model_settings",
        json!({ "default_model": "e2e-updated-model" }),
    )
    .await;
    assert_no_jsonrpc_error(&model_update, "inference_update_model_settings");
    let client_cfg = post_json_rpc(
        &rpc_base,
        367,
        "openhuman.inference_get_client_config",
        json!({}),
    )
    .await;
    let client_cfg_result = assert_no_jsonrpc_error(&client_cfg, "inference_get_client_config");
    let updated_model = client_cfg_result
        .pointer("/result/default_model")
        .or_else(|| client_cfg_result.get("default_model"))
        .and_then(Value::as_str);
    assert_eq!(
        updated_model,
        Some("e2e-updated-model"),
        "inference_get_client_config did not reflect updated default_model: {client_cfg_result}"
    );

    // openhuman.inference_list_models — no cloud provider configured for this
    // local-only test, so we expect a structured error rather than a panic.
    // Asserting an error here proves the controller is registered and reaches
    // its handler over the RPC transport (the empty-picker symptom CodeRabbit
    // flagged would surface as a controller-not-found error instead).
    let list_models = post_json_rpc(
        &rpc_base,
        368,
        "openhuman.inference_list_models",
        json!({ "provider_id": "does-not-exist" }),
    )
    .await;
    let _ = assert_jsonrpc_error(
        &list_models,
        "inference_list_models with unknown provider id",
    );

    lm_join.abort();
    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_inference_prompt_requires_external_ollama_runtime_when_unreachable() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _ollama_url_guard = EnvVarGuard::set("OPENHUMAN_OLLAMA_BASE_URL", "http://127.0.0.1:1");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let update = post_json_rpc(
        &rpc_base,
        364,
        "openhuman.inference_update_local_settings",
        json!({
            "runtime_enabled": true,
            "opt_in_confirmed": true,
            "provider": "ollama",
            "model_id": "gemma3:1b-it-qat",
            "chat_model_id": "gemma3:1b-it-qat"
        }),
    )
    .await;
    assert_no_jsonrpc_error(&update, "update_local_ai_settings for unreachable ollama");

    let prompt = post_json_rpc(
        &rpc_base,
        365,
        "openhuman.inference_prompt",
        json!({
            "prompt": "hello",
            "max_tokens": 16,
            "no_think": true
        }),
    )
    .await;
    let prompt_err = assert_jsonrpc_error(&prompt, "inference_prompt unreachable ollama");
    let prompt_err_message = prompt_err
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        prompt_err_message.contains("external Ollama endpoint is unavailable"),
        "unexpected error: {prompt_err}"
    );

    mock_join.abort();
    rpc_join.abort();
}

// ── Billing & Team E2E tests ──────────────────────────────────────────────────

/// End-to-end test for billing RPC methods.
///
/// Spins up an in-process Axum mock backend and a real JSON-RPC server, stores a
/// session JWT, then exercises every billing controller through the RPC surface
/// exactly as the desktop app or a CI script would.

#[tokio::test]
async fn voice_status_returns_availability() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _piper_guard = EnvVarGuard::unset("PIPER_BIN");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // voice_status does not require auth — it only checks filesystem availability
    let status = post_json_rpc(&rpc_base, 1, "openhuman.voice_status", json!({})).await;
    let result = assert_no_jsonrpc_error(&status, "voice_status");

    // Without piper installed in the test env, TTS should be unavailable. STT
    // is a different story since the whisper.cpp engine was deleted: it is a
    // hosted call now, and the default `backend` engine always resolves, so it
    // reports available with nothing installed locally.
    assert!(
        result.get("stt_available").is_some(),
        "expected stt_available field: {result}"
    );
    assert!(
        result.get("tts_available").is_some(),
        "expected tts_available field: {result}"
    );
    assert!(
        result.get("stt_model_id").is_some(),
        "expected stt_model_id field: {result}"
    );
    assert!(
        result.get("tts_voice_id").is_some(),
        "expected tts_voice_id field: {result}"
    );

    // Hosted STT: the default engine is routable with no local install, so
    // this is now true. It asserted `false` while a whisper.cpp binary was
    // required.
    assert_eq!(
        result.get("stt_available").and_then(Value::as_bool),
        Some(true),
        "hosted stt should be available with no local binaries"
    );
    assert_eq!(
        result.get("tts_available").and_then(Value::as_bool),
        Some(false),
        "tts should be unavailable without piper binary"
    );

    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn voice_stt_dispatch_reaches_handler_and_rejects_invalid_audio_structurally() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _piper_guard = EnvVarGuard::unset("PIPER_BIN");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);
    for config_path in [
        openhuman_home.join("config.toml"),
        openhuman_home.join("users/local/config.toml"),
    ] {
        use std::io::Write as _;
        let mut config = std::fs::OpenOptions::new()
            .append(true)
            .open(config_path)
            .expect("open seeded config");
        writeln!(
            config,
            "\n[[voice_providers]]\nslug = \"e2e-stt\"\nendpoint = \"http://127.0.0.1:9\""
        )
        .expect("append test STT provider");
    }

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let response = post_json_rpc(
        &rpc_base,
        1,
        "openhuman.voice_stt_dispatch",
        json!({ "provider": "e2e-stt", "audio_base64": "not base64", "mime_type": "audio/wav" }),
    )
    .await;
    assert_eq!(response["error"]["code"], json!(-32000), "{response}");
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("[voice-factory] base64 decode error")),
        "voice STT handler must reject malformed audio before making a provider request: {response}"
    );

    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn voice_live_settings_round_trip_over_json_rpc() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let providers = post_json_rpc(&rpc_base, 1, "openhuman.voice_live_providers", json!({})).await;
    let providers = assert_no_jsonrpc_error(&providers, "voice_live_providers");
    assert_eq!(providers["default_provider"], "gemini-hosted");
    let ids: Vec<&str> = providers["providers"]
        .as_array()
        .expect("providers array")
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["gemini-hosted", "elevenlabs-hosted", "gemini", "sarvam"]
    );
    let sarvam = providers["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "sarvam")
        .unwrap();
    assert_eq!(sarvam["kind"], "byok");
    assert_eq!(sarvam["configured"], false);
    assert_eq!(sarvam["key_slug"], "sarvam");

    let set = post_json_rpc(
        &rpc_base,
        2,
        "openhuman.voice_live_settings_set",
        json!({ "default_provider": "sarvam", "sarvam": { "language": "hi-IN", "speaker": "priya" } }),
    )
    .await;
    let set = assert_no_jsonrpc_error(&set, "voice_live_settings_set");
    assert_eq!(set["default_provider"], "sarvam");

    let got = post_json_rpc(&rpc_base, 3, "openhuman.voice_live_settings_get", json!({})).await;
    let got = assert_no_jsonrpc_error(&got, "voice_live_settings_get");
    assert_eq!(got["default_provider"], "sarvam");
    assert_eq!(got["sarvam"]["language"], "hi-IN");
    assert_eq!(got["sarvam"]["speaker"], "priya");

    let bad = post_json_rpc(
        &rpc_base,
        4,
        "openhuman.voice_live_settings_set",
        json!({ "default_provider": "nope" }),
    )
    .await;
    assert!(
        bad.get("error").is_some(),
        "unknown provider must be rejected: {bad}"
    );

    // No Sarvam key is stored, so a test reports why without touching the network.
    let tested = post_json_rpc(
        &rpc_base,
        5,
        "openhuman.voice_live_test_provider",
        json!({ "provider": "sarvam" }),
    )
    .await;
    let tested = assert_no_jsonrpc_error(&tested, "voice_live_test_provider");
    assert_eq!(tested["ok"], false);
    assert!(tested["error"]
        .as_str()
        .unwrap_or_default()
        .contains("not_configured"));

    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn voice_cloud_transcribe_registered_e2e() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // ── 1. /schema must list openhuman.voice_cloud_transcribe ───────────────
    let schema = reqwest::get(format!("{rpc_base}/schema"))
        .await
        .expect("GET /schema")
        .json::<Value>()
        .await
        .expect("schema json");
    let methods = schema["methods"]
        .as_array()
        .unwrap_or_else(|| panic!("/schema must expose methods array: {schema}"));
    let names: Vec<&str> = methods
        .iter()
        .filter_map(|m| m.get("method").and_then(Value::as_str))
        .collect();
    assert!(
        names.contains(&"openhuman.voice_cloud_transcribe"),
        "voice_cloud_transcribe must appear in /schema dump (got {} methods)",
        names.len()
    );

    // ── 2. RPC dispatch must NOT return "unknown method" ───────────────────
    // Send a minimal payload — it'll fail downstream (no upstream STT
    // configured in the mock), but the dispatcher should reach the
    // handler, not the unknown-method branch.
    let resp = post_json_rpc(
        &rpc_base,
        9101,
        "openhuman.voice_cloud_transcribe",
        json!({ "audio_base64": "" }),
    )
    .await;
    // Inspect the full error blob, not just `error.message`. A future
    // server-shape change that moves the dispatcher's unknown-method
    // string into `error.data` would otherwise let this regression
    // guard silently pass.
    let err_blob = resp
        .get("error")
        .map(|e| e.to_string().to_ascii_lowercase())
        .unwrap_or_default();
    assert!(
        !err_blob.contains("unknown method"),
        "voice_cloud_transcribe must be a known method; full response: {resp}"
    );

    mock_join.abort();
    rpc_join.abort();
}

// ── MCP Clients lifecycle ─────────────────────────────────────────────────────
//
// Tests the install → installed_list → uninstall flow over real JSON-RPC.
// We do NOT test connect/tool_call here because that requires a real MCP
// server subprocess — the `FakeMcpTransport` in `client/mod.rs` covers that
// path at unit level. The spawn path is guarded behind a trait so tests can
// inject fakes without touching the filesystem.

/// #5947 — `validate_only` is a dry run for **stt**, not only for tts.
///
/// It used to be checked inside the `"tts"` arm alone. Every provider the
/// settings modal can reach maps to `"stt"`, so the modal's dry-run request
/// fell through to a live transcription — the opposite of what "Test Key"
/// promises. A reserved slug is used because it is the one dry-run answer that
/// is fully determined with no network: there is no user-held key to check, so
/// the handler reports success without calling a provider.
#[tokio::test]
async fn voice_test_provider_honours_validate_only_for_the_stt_workload() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _piper_guard = EnvVarGuard::unset("PIPER_BIN");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let dry_run = post_json_rpc(
        &rpc_base,
        9_947,
        "openhuman.voice_test_provider",
        json!({ "workload": "stt", "provider": "cloud", "validate_only": true }),
    )
    .await;
    // Asserted on the whole envelope rather than through
    // `assert_no_jsonrpc_error` so that BOTH shapes a regression can take —
    // an error response from `create_stt_provider`, or an ok response
    // carrying a live-transcription verdict — fail on this assertion and say
    // why, instead of tripping a generic harness helper first.
    let detail = dry_run
        .get("result")
        .and_then(|r| r.get("detail"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        detail.contains("managed by OpenHuman"),
        "validate_only must short-circuit an stt request before any provider call: \
         expected a managed-provider answer, got detail {detail:?} in {dry_run}. \
         An error here, or an \"STT test …\" verdict, means validate_only was \
         ignored for the stt workload (#5947)."
    );
    assert_eq!(
        dry_run
            .get("result")
            .and_then(|r| r.get("ok"))
            .and_then(Value::as_bool),
        Some(true),
        "a managed provider has no key to check, which is not a failed check: {dry_run}"
    );

    mock_join.abort();
    rpc_join.abort();
}
