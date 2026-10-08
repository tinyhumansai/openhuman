//! Drive the real chat, span collector and authenticated OTLP proxy through
//! independent sharing/content choices against a recording loopback backend.
//! Disclosure rendering is exercised by privacy-trace-disclosure.spec.ts
//! (desktop) and privacy-what-leaves-sheet.spec.ts (browser).

use super::*;

const TELEMETRY_PATH: &str = "/telemetry/langfuse/otel/v1/traces";

#[test]
fn trace_exports_follow_independent_consent_choices() {
    run_json_rpc_e2e_on_agent_stack("trace_consent", trace_exports_follow_consent_inner);
}

fn exported_spans(requests: &[Value]) -> Vec<&Value> {
    requests
        .iter()
        .flat_map(|request| request["body"]["resourceSpans"].as_array().unwrap())
        .flat_map(|resource| resource["scopeSpans"].as_array().unwrap())
        .flat_map(|scope| scope["spans"].as_array().unwrap())
        .collect()
}

fn attribute<'a>(span: &'a Value, key: &str) -> Option<&'a Value> {
    span["attributes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|attr| attr["key"] == key)
        .map(|attr| &attr["value"])
}

async fn assert_consent_snapshot(rpc_base: &str, sharing: bool, content: bool) {
    let response = post_json_rpc(rpc_base, 7016, "openhuman.config_get", json!({})).await;
    let snapshot = peel_logs_envelope(assert_no_jsonrpc_error(&response, "read consent"));
    assert_eq!(
        snapshot["config"]["observability"]["share_usage_data"],
        sharing
    );
    assert_eq!(
        snapshot["config"]["observability"]["agent_tracing"]["capture_content"],
        content
    );
}

/// The local sink runs after remote export. Waiting for this turn's NDJSON
/// provides a completion barrier before inspecting the recording backend.
async fn wait_for_local_trace(path: &Path, request_id: &str) {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let payload = std::fs::read_to_string(path).unwrap_or_default();
            if payload
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .any(|span| {
                    span["trace_id"]
                        .as_str()
                        .is_some_and(|id| id.ends_with(request_id))
                })
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("this turn should finish exporting to its local sink");
}

async fn trace_exports_follow_consent_inner() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("privacy workspace");
    let root = tmp.path().join(".openhuman");
    let user_dir = root.join("users").join("trace-consent-user");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let _workspace = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &user_dir);
    let _backend = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _sharing = EnvVarGuard::unset("OPENHUMAN_SHARE_USAGE_DATA");
    let _content = EnvVarGuard::unset("OPENHUMAN_AGENT_TRACING_CAPTURE_CONTENT");
    let _action_dir = EnvVarGuard::unset("OPENHUMAN_ACTION_DIR");
    let _script_guard = ScriptedFifoGuard;
    clear_forced_chat_completions();

    let recordings = Arc::new(Mutex::new(Vec::<Value>::new()));
    let capture = recordings.clone();
    let backend = mock_upstream_router().route(
        TELEMETRY_PATH,
        post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let capture = capture.clone();
            async move {
                capture.lock().unwrap().push(json!({
                    "authorization": headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
                    "body": body
                }));
                Json(json!({}))
            }
        }),
    );
    let (mock_addr, mock_join) = serve_on_ephemeral(backend).await;
    let mock_origin = format!("http://{mock_addr}");
    write_min_config_with_local_ai_disabled(&root, &mock_origin);
    write_min_config_with_local_ai_disabled(&user_dir, &mock_origin);

    let mut config = openhuman_core::config::load_config_with_timeout()
        .await
        .unwrap();
    // Seed an already-authenticated profile in this fixture's workspace.
    // This keeps login's global user activation and background services out
    // of the test while using the real credential resolver for every export.
    use openhuman_core::security::credentials::{
        AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
    };
    openhuman_core::security::keyring::init_workspace(&config.workspace_dir);
    AuthService::from_config(&config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "e2e-test-jwt",
            std::collections::HashMap::from([(
                "user_id".to_string(),
                "trace-consent-user".to_string(),
            )]),
            true,
        )
        .unwrap();

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");
    assert_consent_snapshot(&rpc_base, false, false).await;
    let defaults = openhuman_core::config::Config::default();
    assert!(!defaults.observability.share_usage_data);
    assert!(!defaults.observability.agent_tracing.capture_content);
    assert!(!config.observability.share_usage_data);
    assert!(!config.observability.agent_tracing.capture_content);
    assert!(!config.observability.agent_tracing.enabled);

    let local_path = tmp.path().join("local-traces.ndjson");
    // Enable the independent local sink solely as the export completion barrier.
    config.observability.agent_tracing.enabled = true;
    config.observability.agent_tracing.export_path =
        Some(local_path.to_string_lossy().into_owned());
    config.web_chat.suggestions_enabled = false;
    let action_dir = tmp.path().join("files");
    std::fs::create_dir_all(&action_dir).unwrap();
    config.action_dir_override = Some(action_dir.clone());
    config.save().await.unwrap();

    // Exercise a real deferred file_read tool with synthetic content.
    let tool_path = action_dir.join("TRACE_TOOL_ARGUMENT_CANARY.txt");
    std::fs::write(&tool_path, "TRACE_TOOL_RESULT_CANARY").unwrap();

    for (index, sharing, content) in [
        (0, false, false),
        (1, false, true),
        (2, true, false),
        (3, true, true),
        (4, false, true),
    ] {
        println!("trace consent case {index}: sharing={sharing}, content={content}");
        config.observability.share_usage_data = sharing;
        config.observability.agent_tracing.capture_content = content;
        config.save().await.unwrap();
        assert_consent_snapshot(&rpc_base, sharing, content).await;
        let before = recordings.lock().unwrap().len();
        let prompt = format!("TRACE_PROMPT_CANARY_{index}: read the synthetic file");
        let reply = format!("TRACE_REPLY_CANARY_{index}");
        push_forced_chat_completion_when(
            &prompt,
            forced_tool_call_completion("file_read", json!({"path": tool_path})),
        );
        push_forced_chat_completion_when(&prompt, forced_text_completion(&reply));

        let client_id = format!("trace-consent-client-{index}");
        let thread_id = format!("trace-consent-thread-{index}");
        let events_url = format!("{rpc_base}/events?client_id={client_id}");
        let (terminal_task, request_tx) =
            spawn_ready_terminal_web_chat_event_for_request(&events_url).await;
        let accepted = post_json_rpc(
            &rpc_base,
            7020 + index,
            "openhuman.channel_web_chat",
            json!({
                "client_id": client_id,
                "thread_id": thread_id,
                "message": prompt,
                "model_override": "e2e-mock-model"
            }),
        )
        .await;
        let accepted = assert_no_jsonrpc_error(&accepted, "run consent test turn");
        let request_id = accepted["result"]["request_id"]
            .as_str()
            .unwrap()
            .to_owned();
        signal_accepted_web_chat_request_id(request_tx, accepted);
        let terminal = tokio::time::timeout(Duration::from_secs(60), terminal_task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(terminal["event"], "chat_done", "{terminal}");
        assert!(terminal["full_response"].as_str().unwrap().contains(&reply));
        // Closing the conversation drops its warm session's progress sender,
        // allowing the real bridge to seal and export the completed trace.
        let closed = post_json_rpc(
            &rpc_base,
            7030 + index,
            "openhuman.threads_delete",
            json!({"thread_id": thread_id, "deleted_at": chrono::Utc::now().to_rfc3339()}),
        )
        .await;
        assert_no_jsonrpc_error(&closed, "close the completed conversation");
        wait_for_local_trace(&local_path, &request_id).await;

        let requests = recordings.lock().unwrap()[before..].to_vec();
        if !sharing {
            assert!(
                requests.is_empty(),
                "sharing off should keep traces local: {requests:?}"
            );
            continue;
        }
        assert!(
            !requests.is_empty(),
            "sharing on should reach the backend OTLP proxy"
        );
        assert!(requests
            .iter()
            .all(|request| request["authorization"] == "Bearer e2e-test-jwt"));
        let spans = exported_spans(&requests);
        assert!(spans
            .iter()
            .any(|span| span["name"] == "agent.turn:orchestrator"));
        assert!(spans.iter().all(
            |span| span["startTimeUnixNano"].is_string() && span["endTimeUnixNano"].is_string()
        ));
        assert!(spans
            .iter()
            .any(|span| attribute(span, "langfuse.observation.usage_details").is_some()));
        let serialized = serde_json::to_string(&requests).unwrap();
        if content {
            for canary in [
                prompt.as_str(),
                reply.as_str(),
                "TRACE_TOOL_ARGUMENT_CANARY",
                "TRACE_TOOL_RESULT_CANARY",
            ] {
                assert!(
                    serialized.contains(canary),
                    "content opt-in should include {canary}: {serialized}"
                );
            }
            let generation = spans
                .iter()
                .find(|span| {
                    attribute(span, "langfuse.observation.type")
                        == Some(&json!({"stringValue": "generation"}))
                })
                .expect("the exported turn should include a generation");
            let input = attribute(generation, "langfuse.observation.input").unwrap()["stringValue"]
                .as_str()
                .unwrap();
            let messages: Value = serde_json::from_str(input).unwrap();
            assert!(
                messages.as_array().unwrap().iter().any(|message| {
                    message["role"] == "system"
                        && message["content"]
                            .as_str()
                            .is_some_and(|text| !text.is_empty())
                }),
                "content opt-in includes the structured system prompt"
            );
            let tool = spans
                .iter()
                .find(|span| span["name"] == "tool.file_read")
                .expect("the real file_read call should have an exported span");
            assert!(attribute(tool, "langfuse.observation.input")
                .unwrap()
                .to_string()
                .contains("TRACE_TOOL_ARGUMENT_CANARY"));
            assert!(attribute(tool, "langfuse.observation.output")
                .unwrap()
                .to_string()
                .contains("TRACE_TOOL_RESULT_CANARY"));
        } else {
            for span in &spans {
                assert!(attribute(span, "langfuse.observation.input").is_none());
                assert!(attribute(span, "langfuse.observation.output").is_none());
            }
            for canary in [
                "TRACE_PROMPT_CANARY",
                "TRACE_REPLY_CANARY",
                "TRACE_TOOL_ARGUMENT_CANARY",
                "TRACE_TOOL_RESULT_CANARY",
            ] {
                assert!(
                    !serialized.contains(canary),
                    "metadata sharing should withhold {canary}"
                );
            }
        }
    }
    assert!(with_forced_chat_completions(|queue| queue.is_empty()));
    rpc_join.abort();
    mock_join.abort();
}
