//! JSON-RPC E2E cases for the web chat namespace.

#[path = "support/rpc_e2e_harness.rs"]
mod harness;
use harness::*;

#[test]
fn json_rpc_web_chat_routing_cases_use_expected_backend_models() {
    run_json_rpc_e2e_on_agent_stack(
        "json_rpc_web_chat_routing_cases",
        json_rpc_web_chat_routing_cases_use_expected_backend_models_inner,
    );
}

async fn json_rpc_web_chat_routing_cases_use_expected_backend_models_inner() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    // Keep this router on its own fixture even if a previous test's server
    // still has an active-user marker while it winds down.
    let _workspace_guard = EnvVarGuard::set_to_path(
        "OPENHUMAN_WORKSPACE",
        &openhuman_home.join("users").join("routing-e2e-user"),
    );
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);

    write_min_config_with_local_ai_disabled(&openhuman_home, &mock_origin);
    let user_scoped_dir = openhuman_home.join("users").join("routing-e2e-user");
    write_min_config_with_local_ai_disabled(&user_scoped_dir, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let store = post_json_rpc(
        &rpc_base,
        1,
        "openhuman.auth_store_session",
        json!({
            "token": "e2e-test-jwt",
            "user_id": "routing-e2e-user"
        }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "store_session");

    // Every managed role — and a retired tier slug — runs on the managed
    // default model; a concrete catalog id is forwarded verbatim.
    let routing_cases = [
        ("hint:reasoning", "openrouter/deepseek/deepseek-v4-flash"),
        ("hint:agentic", "openrouter/deepseek/deepseek-v4-flash"),
        ("hint:coding", "openrouter/deepseek/deepseek-v4-flash"),
        ("reasoning-v1", "openrouter/deepseek/deepseek-v4-flash"),
        (
            "openrouter/deepseek/deepseek-v4-pro",
            "openrouter/deepseek/deepseek-v4-pro",
        ),
        // Web chat forwards lightweight hint overrides as-is for this path,
        // so the upstream model receives the original hint string.
        ("hint:reaction", "hint:reaction"),
    ];

    for (idx, (model_override, expected_model)) in routing_cases.iter().enumerate() {
        with_chat_completion_models(|models| models.clear());

        let client_id = format!("routing-case-client-{idx}");
        let thread_id = format!("routing-case-thread-{idx}");
        let events_url = format!("{}/events?client_id={}", rpc_base, client_id);
        let sse_task =
            tokio::spawn(async move { read_sse_event_by_type(&events_url, "chat_done").await });

        let web_chat = post_json_rpc(
            &rpc_base,
            100 + idx as i64,
            "openhuman.channel_web_chat",
            json!({
                "client_id": client_id,
                "thread_id": thread_id,
                "message": format!("route case {idx}"),
                "model_override": model_override,
            }),
        )
        .await;
        let web_chat_result = assert_no_jsonrpc_error(&web_chat, "channel_web_chat");
        assert_eq!(
            web_chat_result
                .get("result")
                .and_then(|v| v.get("accepted")),
            Some(&json!(true))
        );

        let sse_event = tokio::time::timeout(Duration::from_secs(45), sse_task)
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for chat_done for case {model_override}"))
            .expect("sse task join should succeed");
        assert_eq!(
            sse_event.get("event").and_then(Value::as_str),
            Some("chat_done")
        );

        let request =
            wait_for_chat_completion_request_with_message(&format!("route case {idx}")).await;
        assert_eq!(
            request.get("model").and_then(Value::as_str),
            Some(*expected_model),
            "case={model_override} request={request:?}"
        );
        assert_eq!(
            request.get("path").and_then(Value::as_str),
            Some("/openai/v1/chat/completions"),
            "valid managed route should reach the managed backend: case={model_override} request={request:?}"
        );
        assert_eq!(
            request.get("authorization").and_then(Value::as_str),
            Some("Bearer e2e-test-jwt"),
            "valid managed route should use the stored backend session credential: case={model_override} request={request:?}"
        );
    }

    // An unknown qualified provider must fail during the real core turn
    // setup, rather than silently reusing a previously configured provider.
    with_chat_completion_requests(|requests| requests.clear());
    let unknown_client_id = "routing-unknown-provider-client";
    let unknown_thread_id = "routing-unknown-provider-thread";
    let unknown_events_url = format!("{rpc_base}/events?client_id={unknown_client_id}");
    let unknown_sse_task =
        tokio::spawn(async move { read_terminal_web_chat_event(&unknown_events_url).await });
    let unknown_web_chat = post_json_rpc(
        &rpc_base,
        200,
        "openhuman.channel_web_chat",
        json!({
            "client_id": unknown_client_id,
            "thread_id": unknown_thread_id,
            "message": "unknown provider route",
            "model_override": "unknown-provider:model",
        }),
    )
    .await;
    let unknown_result = assert_no_jsonrpc_error(&unknown_web_chat, "unknown provider turn");
    assert_eq!(
        unknown_result
            .get("result")
            .and_then(|value| value.get("accepted")),
        Some(&json!(true))
    );

    let unknown_event = tokio::time::timeout(Duration::from_secs(12), unknown_sse_task)
        .await
        .expect("timed out waiting for unknown-provider chat_error")
        .expect("unknown-provider SSE task join should succeed");
    assert_eq!(
        unknown_event.get("event").and_then(Value::as_str),
        Some("chat_error"),
        "unknown provider route should fail at the core boundary: {unknown_event}"
    );
    let unknown_turn_requests = with_chat_completion_requests(|requests| {
        requests
            .iter()
            .filter(|request| {
                request["body"]["messages"]
                    .as_array()
                    .is_some_and(|messages| {
                        messages.iter().any(|entry| {
                            entry["content"]
                                .as_str()
                                .is_some_and(|content| content.contains("unknown provider route"))
                        })
                    })
            })
            .cloned()
            .collect::<Vec<_>>()
    });
    assert!(
        unknown_turn_requests.is_empty(),
        "unknown selected route must emit chat_error without an inference request attributable to its user message; captured requests={unknown_turn_requests:?}"
    );

    mock_join.abort();
    rpc_join.abort();
}

#[test]
fn json_rpc_web_chat_custom_chat_provider_uses_stored_key_and_rebuilds_on_route_change() {
    run_json_rpc_e2e_on_agent_stack(
        "json_rpc_web_chat_custom_provider_route_change",
        json_rpc_web_chat_custom_chat_provider_uses_stored_key_and_rebuilds_on_route_change_inner,
    );
}

async fn json_rpc_web_chat_custom_chat_provider_uses_stored_key_and_rebuilds_on_route_change_inner()
{
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    // Keep this router on its own fixture even if a previous test's server
    // still has an active-user marker while it winds down.
    let _workspace_guard = EnvVarGuard::set_to_path(
        "OPENHUMAN_WORKSPACE",
        &openhuman_home
            .join("users")
            .join("custom-provider-e2e-user"),
    );
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);

    write_min_config_with_local_ai_disabled(&openhuman_home, &mock_origin);
    let user_scoped_dir = openhuman_home
        .join("users")
        .join("custom-provider-e2e-user");
    write_min_config_with_local_ai_disabled(&user_scoped_dir, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let store = post_json_rpc(
        &rpc_base,
        6001,
        "openhuman.auth_store_session",
        json!({
            "token": "e2e-test-jwt",
            "user_id": "custom-provider-e2e-user"
        }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "store_session");

    let update = post_json_rpc(
        &rpc_base,
        6002,
        "openhuman.update_model_settings",
        json!({
            "cloud_providers": [{
                "id": "p_openai_1",
                "slug": "openai",
                "label": "OpenAI",
                "endpoint": mock_origin,
                "auth_style": "bearer"
            }],
            "default_model": "openai:gpt-4.1-mini"
        }),
    )
    .await;
    assert_no_jsonrpc_error(&update, "update_model_settings");

    let initial_config_get =
        post_json_rpc(&rpc_base, 6999, "openhuman.config_get", json!({})).await;
    let initial_config_result = assert_no_jsonrpc_error(&initial_config_get, "initial config_get");
    let initial_config_payload = peel_logs_envelope(initial_config_result);
    let initial_config = initial_config_payload
        .get("config")
        .unwrap_or(initial_config_payload);
    assert_eq!(
        initial_config.get("default_model").and_then(Value::as_str),
        Some("openai:gpt-4.1-mini"),
        "config_get must expose the authoritative configured BYOK default before any turn override"
    );

    let store_provider = post_json_rpc(
        &rpc_base,
        6003,
        "openhuman.auth_store_provider_credentials",
        json!({
            "provider": "provider:openai",
            "profile": "default",
            "token": "sk-custom-openai-key",
            "setActive": true
        }),
    )
    .await;
    assert_no_jsonrpc_error(&store_provider, "auth_store_provider_credentials");

    with_chat_completion_models(|models| models.clear());
    with_chat_completion_requests(|requests| requests.clear());

    let client_id = "custom-provider-client";
    let thread_id = "custom-provider-thread";
    let events_url = format!("{}/events?client_id={}", rpc_base, client_id);
    let (sse_task, request_tx) = spawn_ready_terminal_web_chat_event_for_request(&events_url).await;

    let accepted = post_json_rpc(
        &rpc_base,
        6004,
        "openhuman.channel_web_chat",
        json!({
            "client_id": client_id,
            "thread_id": thread_id,
            "message": "Use the custom reasoning provider"
        }),
    )
    .await;
    let accepted_result = assert_no_jsonrpc_error(&accepted, "channel_web_chat first");
    assert_eq!(
        accepted_result
            .get("result")
            .and_then(|v| v.get("accepted")),
        Some(&json!(true))
    );
    signal_accepted_web_chat_request_id(request_tx, accepted_result);
    let sse_event = tokio::time::timeout(Duration::from_secs(45), sse_task)
        .await
        .expect("timed out waiting for first custom-provider chat_done")
        .expect("first custom-provider sse join");
    assert_eq!(
        sse_event.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "unexpected first custom-provider terminal event: {sse_event}; request count={}",
        with_chat_completion_requests(|requests| requests.len())
    );

    let requests = wait_for_chat_completion_requests_len(1).await;
    let custom_request = requests
        .iter()
        .find(|request| request.get("model").and_then(Value::as_str) == Some("gpt-4.1-mini"))
        .unwrap_or_else(|| {
            panic!(
                "expected gpt-4.1-mini among {} provider calls",
                requests.len()
            )
        });
    assert_eq!(
        custom_request.get("path").and_then(Value::as_str),
        Some("/chat/completions")
    );
    assert!(
        custom_request.get("authorization").and_then(Value::as_str)
            == Some("Bearer sk-custom-openai-key"),
        "custom provider call must use its stored credential"
    );

    let update_again = post_json_rpc(
        &rpc_base,
        6005,
        "openhuman.update_model_settings",
        json!({
            "default_model": "openai:gpt-4.1-nano"
        }),
    )
    .await;
    assert_no_jsonrpc_error(&update_again, "update_model_settings second");

    let events_url = format!("{}/events?client_id={}", rpc_base, client_id);
    let (sse_task, request_tx) = spawn_ready_terminal_web_chat_event_for_request(&events_url).await;

    let accepted = post_json_rpc(
        &rpc_base,
        6006,
        "openhuman.channel_web_chat",
        json!({
            "client_id": client_id,
            "thread_id": thread_id,
            "message": "Route the next turn with the updated model"
        }),
    )
    .await;
    let accepted_result = assert_no_jsonrpc_error(&accepted, "channel_web_chat second");
    assert_eq!(
        accepted_result
            .get("result")
            .and_then(|v| v.get("accepted")),
        Some(&json!(true))
    );
    signal_accepted_web_chat_request_id(request_tx, accepted_result);
    let sse_event = tokio::time::timeout(Duration::from_secs(45), sse_task)
        .await
        .expect("timed out waiting for second custom-provider chat_done")
        .expect("second custom-provider sse join");
    assert_eq!(
        sse_event.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "unexpected second custom-provider terminal event: {sse_event}; request count={}",
        with_chat_completion_requests(|requests| requests.len())
    );

    let requests = wait_for_chat_completion_requests_len(2).await;
    let updated_request = requests
        .iter()
        .find(|request| request.get("model").and_then(Value::as_str) == Some("gpt-4.1-nano"))
        .unwrap_or_else(|| {
            panic!(
                "expected gpt-4.1-nano among {} provider calls",
                requests.len()
            )
        });
    assert_eq!(
        updated_request.get("model").and_then(Value::as_str),
        Some("gpt-4.1-nano"),
        "cached web-chat session should rebuild when chat_provider changes"
    );
    assert!(
        updated_request.get("authorization").and_then(Value::as_str)
            == Some("Bearer sk-custom-openai-key"),
        "updated provider call must use its stored credential"
    );

    let events_url = format!("{}/events?client_id={}", rpc_base, client_id);
    let (sse_task, request_tx) = spawn_ready_terminal_web_chat_event_for_request(&events_url).await;

    let accepted = post_json_rpc(
        &rpc_base,
        6007,
        "openhuman.channel_web_chat",
        json!({
            "client_id": client_id,
            "thread_id": thread_id,
            "message": "This turn should stay on the backend agentic route",
            "model_override": "hint:agentic"
        }),
    )
    .await;
    let accepted_result =
        assert_no_jsonrpc_error(&accepted, "channel_web_chat unaffected agentic route");
    assert_eq!(
        accepted_result
            .get("result")
            .and_then(|v| v.get("accepted")),
        Some(&json!(true))
    );
    signal_accepted_web_chat_request_id(request_tx, accepted_result);
    let sse_event = tokio::time::timeout(Duration::from_secs(45), sse_task)
        .await
        .expect("timed out waiting for unaffected agentic chat_done")
        .expect("unaffected agentic sse join");
    assert_eq!(
        sse_event.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "unexpected unaffected-agentic terminal event: {sse_event}; request count={}",
        with_chat_completion_requests(|requests| requests.len())
    );

    let agentic_request = wait_for_chat_completion_request_with_message(
        "This turn should stay on the backend agentic route",
    )
    .await;
    assert_eq!(
        agentic_request.get("path").and_then(Value::as_str),
        Some("/openai/v1/chat/completions"),
        "custom reasoning provider must not hijack unrelated backend routes"
    );
    assert_eq!(
        agentic_request.get("model").and_then(Value::as_str),
        Some("openrouter/deepseek/deepseek-v4-flash")
    );

    async fn assert_managed_turn(
        rpc_base: &str,
        request_id: i64,
        client_id: &str,
        thread_id: &str,
        message: &str,
        expected_model: &str,
        model_override: Option<&str>,
    ) {
        let events_url = format!("{}/events?client_id={}", rpc_base, client_id);
        let (sse_task, request_tx) =
            spawn_ready_terminal_web_chat_event_for_request(&events_url).await;
        let mut params = json!({
            "client_id": client_id,
            "thread_id": thread_id,
            "message": message,
        });
        if let Some(model_override) = model_override {
            params["model_override"] = json!(model_override);
        }
        let accepted =
            post_json_rpc(rpc_base, request_id, "openhuman.channel_web_chat", params).await;
        let accepted_result = assert_no_jsonrpc_error(&accepted, "channel_web_chat managed");
        assert_eq!(
            accepted_result
                .get("result")
                .and_then(|v| v.get("accepted")),
            Some(&json!(true))
        );
        signal_accepted_web_chat_request_id(request_tx, accepted_result);
        let sse_event = tokio::time::timeout(Duration::from_secs(45), sse_task)
            .await
            .expect("timed out waiting for managed chat_done")
            .expect("managed sse join");
        assert_eq!(
            sse_event.get("event").and_then(Value::as_str),
            Some("chat_done"),
            "unexpected managed terminal event: {sse_event}"
        );
        let request = wait_for_chat_completion_request_with_message(message).await;
        assert_eq!(
            request.get("path").and_then(Value::as_str),
            Some("/openai/v1/chat/completions")
        );
        assert_eq!(
            request.get("model").and_then(Value::as_str),
            Some(expected_model)
        );
        assert_eq!(
            request.get("authorization").and_then(Value::as_str),
            Some("Bearer e2e-test-jwt"),
            "managed request must use the backend session credential"
        );
    }

    let picker_model = "openrouter/author/picker-model:free";
    assert_managed_turn(
        &rpc_base,
        6008,
        client_id,
        thread_id,
        "Explicit managed picker model with warm BYOK session",
        picker_model,
        Some(picker_model),
    )
    .await;
    let config_get = post_json_rpc(&rpc_base, 6009, "openhuman.config_get", json!({})).await;
    let config_result = assert_no_jsonrpc_error(&config_get, "config_get after managed override");
    let config_payload = peel_logs_envelope(config_result);
    let config = config_payload.get("config").unwrap_or(config_payload);
    assert_eq!(
        config.get("default_model").and_then(Value::as_str),
        Some("openai:gpt-4.1-nano"),
        "a per-turn managed selection must not replace the configured BYOK provider"
    );

    let first_managed_default = "openrouter/author/first-default-model:free";
    let update_first_default = post_json_rpc(
        &rpc_base,
        6010,
        "openhuman.update_model_settings",
        json!({ "default_model": first_managed_default }),
    )
    .await;
    assert_no_jsonrpc_error(
        &update_first_default,
        "update_model_settings first managed default",
    );
    assert_managed_turn(
        &rpc_base,
        6011,
        client_id,
        thread_id,
        "Managed default first route cache turn",
        first_managed_default,
        None,
    )
    .await;

    let second_managed_default = "openrouter/author/second-default-model:free";
    let update_second_default = post_json_rpc(
        &rpc_base,
        6012,
        "openhuman.update_model_settings",
        json!({ "default_model": second_managed_default }),
    )
    .await;
    assert_no_jsonrpc_error(
        &update_second_default,
        "update_model_settings second managed default",
    );
    assert_managed_turn(
        &rpc_base,
        6013,
        client_id,
        thread_id,
        "Managed default second route cache turn",
        second_managed_default,
        None,
    )
    .await;
    mock_join.abort();
    rpc_join.abort();
}

#[test]
fn json_rpc_web_chat_custom_chat_provider_with_auth_none_omits_auth_header() {
    run_json_rpc_e2e_on_agent_stack(
        "json_rpc_web_chat_custom_provider_auth_none",
        json_rpc_web_chat_custom_chat_provider_with_auth_none_omits_auth_header_inner,
    );
}

async fn json_rpc_web_chat_custom_chat_provider_with_auth_none_omits_auth_header_inner() {
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

    write_min_config_with_local_ai_disabled(&openhuman_home, &mock_origin);
    let user_scoped_dir = openhuman_home.join("users").join("e2e-user");
    write_min_config_with_local_ai_disabled(&user_scoped_dir, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let store = post_json_rpc(
        &rpc_base,
        6101,
        "openhuman.auth_store_session",
        json!({
            "token": "e2e-test-jwt",
            "user_id": "e2e-user"
        }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "store_session");

    let update = post_json_rpc(
        &rpc_base,
        6102,
        "openhuman.update_model_settings",
        json!({
            "cloud_providers": [{
                "id": "p_proxy_1",
                "slug": "proxy",
                "label": "Proxy",
                "endpoint": mock_origin,
            "auth_style": "none",
            "ca_cert_pem": include_str!("../crates/openhuman-core/src/util/tls/test-ca.pem")
            }, {
                "id": "p_openhuman_builtin",
                "slug": "openhuman",
                "label": "OpenHuman",
                "endpoint": "https://api.openhuman.ai/v1",
                "auth_style": "openhuman_jwt",
                "ca_cert_pem": include_str!("../crates/openhuman-core/src/util/tls/test-ca.pem")
            }],
            "chat_provider": "proxy:gpt-oss"
        }),
    )
    .await;
    assert_no_jsonrpc_error(&update, "update_model_settings");
    let update_without_builtin = post_json_rpc(
        &rpc_base,
        6103,
        "openhuman.update_model_settings",
        json!({
            "cloud_providers": [{
                "id": "p_proxy_1",
                "slug": "proxy",
                "label": "Proxy",
                "endpoint": mock_origin,
                "auth_style": "none"
            }],
            "chat_provider": "proxy:gpt-oss"
        }),
    )
    .await;
    assert_no_jsonrpc_error(
        &update_without_builtin,
        "update_model_settings without reserved entry",
    );
    let cfg = post_json_rpc(&rpc_base, 61_021, "openhuman.config_get", json!({})).await;
    let cfg_outer = assert_no_jsonrpc_error(&cfg, "config_get auth-none");
    let cfg_payload = cfg_outer.get("result").unwrap_or(cfg_outer);
    let config = cfg_payload.get("config").unwrap_or(cfg_payload);
    assert_eq!(
        config.get("chat_provider").and_then(Value::as_str),
        Some("proxy:gpt-oss")
    );
    // The user's "proxy" entry must survive the update. We intentionally
    // assert by slug rather than by raw length — the
    // `unify_ai_provider_settings` migration seeds a built-in "openhuman"
    // cloud_providers entry on fresh configs, and `apply_model_settings`
    // preserves reserved-slug built-ins across updates (Sentry TAURI-RUST-5
    // fix). A raw length assertion would couple this test to the count of
    // built-ins, which is a separate concern.
    let providers = config
        .get("cloud_providers")
        .and_then(Value::as_array)
        .expect("cloud_providers should be present in config snapshot");
    assert!(
        providers
            .iter()
            .any(|e| e.get("slug").and_then(Value::as_str) == Some("proxy")),
        "user's auth-none 'proxy' entry must survive the update: {providers:?}"
    );
    let loaded_config = openhuman_core::config::load_config_with_timeout()
        .await
        .expect("load_config after auth-none update");
    assert_eq!(
        loaded_config
            .cloud_provider_ca_certs
            .get("proxy")
            .map(String::as_str),
        Some(include_str!(
            "../crates/openhuman-core/src/util/tls/test-ca.pem"
        )),
        "config settings RPC must persist ca_cert_pem by provider slug"
    );
    assert_eq!(
        loaded_config
            .cloud_provider_ca_certs
            .get("openhuman")
            .map(String::as_str),
        Some(include_str!(
            "../crates/openhuman-core/src/util/tls/test-ca.pem"
        )),
        "reinjecting a reserved provider must retain its CA bundle"
    );
    let (model, model_id) =
        openhuman_core::inference::provider::create_chat_model_with_model_id_in(
            &openhuman_core::core::runtime::DispatchCtx::host_default(),
            "chat",
            &loaded_config,
            0.0,
        )
        .await
        .expect("custom auth-none model should build");
    let direct = model
        .invoke(
            &(),
            ModelRequest::new(vec![Message::user("direct custom-provider smoke test")])
                .with_model(model_id),
        )
        .await
        .expect("direct custom auth-none model call should succeed");
    assert!(
        direct.text().contains("Hello from custom provider"),
        "unexpected direct custom-provider response: {direct:?}"
    );

    with_chat_completion_models(|models| models.clear());
    with_chat_completion_requests(|requests| requests.clear());

    let client_id = "auth-none-client";
    let thread_id = "auth-none-thread";
    let events_url = format!("{}/events?client_id={}", rpc_base, client_id);
    let sse_task = tokio::spawn(async move { read_terminal_web_chat_event(&events_url).await });

    let accepted = post_json_rpc(
        &rpc_base,
        6103,
        "openhuman.channel_web_chat",
        json!({
            "client_id": client_id,
            "thread_id": thread_id,
            "message": "Use the auth-none provider"
        }),
    )
    .await;
    let accepted_result = assert_no_jsonrpc_error(&accepted, "channel_web_chat auth-none");
    assert_eq!(
        accepted_result
            .get("result")
            .and_then(|v| v.get("accepted")),
        Some(&json!(true))
    );
    let sse_event = tokio::time::timeout(Duration::from_secs(45), sse_task)
        .await
        .expect("timed out waiting for auth-none chat_done")
        .expect("auth-none sse join");
    assert_eq!(
        sse_event.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "unexpected auth-none terminal event: {sse_event}; requests={:?}",
        with_chat_completion_requests(|requests| requests.clone())
    );

    let requests = wait_for_chat_completion_requests_len(1).await;
    let auth_none_request = requests
        .iter()
        .find(|request| request.get("model").and_then(Value::as_str) == Some("gpt-oss"))
        .unwrap_or_else(|| panic!("expected auth-none provider call: {requests:?}"));
    assert_eq!(
        auth_none_request.get("path").and_then(Value::as_str),
        Some("/chat/completions")
    );
    assert!(
        auth_none_request.get("authorization").is_none()
            || auth_none_request
                .get("authorization")
                .is_some_and(Value::is_null),
        "auth_style=none must not emit Authorization: {:?}",
        auth_none_request.get("authorization")
    );
    assert!(
        auth_none_request.get("x_api_key").is_none()
            || auth_none_request
                .get("x_api_key")
                .is_some_and(Value::is_null),
        "auth_style=none must not emit x-api-key: {:?}",
        auth_none_request.get("x_api_key")
    );

    mock_join.abort();
    rpc_join.abort();
}

#[test]
fn json_rpc_channel_web_chat_with_speak_reply_invokes_reply_speech() {
    run_json_rpc_e2e_on_agent_stack(
        "json_rpc_speak_reply_e2e",
        json_rpc_channel_web_chat_with_speak_reply_invokes_reply_speech_inner,
    );
}

async fn json_rpc_channel_web_chat_with_speak_reply_invokes_reply_speech_inner() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    // The hosted `cloud` TTS provider (and its test seam) is registered by the
    // TinyHumans layer, not built into the core.
    tinyhumans_boot::boot();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    // Activate the reply_speech test seam so synthesize_reply records and
    // short-circuits instead of calling the hosted backend.
    let _seam_guard = EnvVarGuard::set(
        openhuman_tinyhumans::voice::reply_speech::TEST_SEAM_ENV,
        "1",
    );

    openhuman_tinyhumans::voice::reply_speech::test_seam::clear();

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);

    write_min_config(&openhuman_home, &mock_origin);
    let user_scoped_dir = openhuman_home.join("users").join("e2e-user");
    write_min_config(&user_scoped_dir, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Authenticate so the agent loop has a session token available.
    let store = post_json_rpc(
        &rpc_base,
        9300,
        "openhuman.auth_store_session",
        json!({
            "token": "e2e-test-jwt",
            "user_id": "e2e-user"
        }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "store_session");

    let client_id = "ptt-e2e-client";
    let thread_id = "ptt-e2e-thread";
    let events_url = format!("{}/events?client_id={}", rpc_base, client_id);
    let sse_task = tokio::spawn(async move { read_terminal_web_chat_event(&events_url).await });

    // PTT-style chat send: speak_reply=true, source=ptt, session_id=1.
    let web_chat = post_json_rpc(
        &rpc_base,
        9301,
        "openhuman.channel_web_chat",
        json!({
            "client_id": client_id,
            "thread_id": thread_id,
            "message": "Hello from PTT",
            "model_override": "e2e-mock-model",
            "speak_reply": true,
            "source": "ptt",
            "session_id": 1,
        }),
    )
    .await;
    let web_chat_result = assert_no_jsonrpc_error(&web_chat, "channel_web_chat");
    assert_eq!(
        web_chat_result
            .get("result")
            .and_then(|v| v.get("accepted")),
        Some(&json!(true))
    );

    let sse_event = tokio::time::timeout(Duration::from_secs(45), sse_task)
        .await
        .expect("timed out waiting for chat_done with speak_reply=true")
        .expect("sse task join should succeed");
    assert_eq!(
        sse_event.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "speak-reply chat must finish successfully; terminal event: {sse_event:?}"
    );

    // The bridge should have buffered the streamed assistant text and
    // routed it through synthesize_reply on TurnCompleted. Poll briefly
    // because the bridge task may finish slightly after chat_done.
    let mut observed: Vec<String> = Vec::new();
    for _ in 0..50 {
        observed = openhuman_tinyhumans::voice::reply_speech::test_seam::observed();
        if !observed.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        !observed.is_empty(),
        "expected reply_speech::synthesize_reply to be invoked when speak_reply=true; observed={observed:?}"
    );
    assert!(
        observed.iter().any(|t| !t.trim().is_empty()),
        "expected at least one non-empty text passed to synthesize_reply; observed={observed:?}"
    );
    assert!(
        observed
            .iter()
            .any(|t| t.contains("Hello from e2e mock agent")),
        "expected the observed seam text to include the mock reply phrase; got {observed:?}"
    );

    mock_join.abort();
    rpc_join.abort();
}

// ── Model resolution and voice settings ─────────────────────────────────
