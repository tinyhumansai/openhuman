use super::*;

#[tokio::test]
async fn composio_get_user_profile_via_mock_returns_provider_profile() {
    let _serialised = module_guard().await;
    // The embedding seam fails loudly when unwired. Installed here rather
    // than relied upon from another test: `install_for_tests` is
    // `Once`-guarded, so a test that omits it passes only while some
    // earlier test in the same binary happened to run first.
    use crate::openhuman::config::TEST_ENV_LOCK;
    let _cache_guard = cache_guard();
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // This test mutates BACKEND_URL below via EnvVarGuard, which races with
    // api::config / core::cli_tests / medulla::ops / medulla::resolve tests
    // that mutate the same process-global var under the crate-wide lock —
    // TEST_ENV_LOCK alone does not serialize against those. Hold both.
    let _backend_env_guard = crate::api::config::backend_env_test_lock();

    let app = Router::new()
        .route(
            "/agent-integrations/composio/connections",
            get(|| async {
                Json(json!({
                    "success": true,
                    "data": {"connections": [
                        {"id":"c1","toolkit":"gmail","status":"ACTIVE"}
                    ]}
                }))
            }),
        )
        .route(
            "/agent-integrations/composio/execute",
            post(|Json(body): Json<Value>| async move {
                let action = body
                    .get("tool")
                    .and_then(Value::as_str)
                    .or_else(|| body.get("action").and_then(Value::as_str))
                    .unwrap_or("");
                let data = match action {
                    "GMAIL_GET_PROFILE" => json!({
                        "emailAddress": "pilot@example.com",
                        "displayName": "Phoenix Pilot",
                        "profileUrl": "https://mail.google.com/mail/u/0/#inbox"
                    }),
                    other => panic!("unexpected action: {other}"),
                };
                Json(json!({
                    "success": true,
                    "data": {
                        "successful": true,
                        "data": data,
                        "error": null
                    }
                }))
            }),
        );
    let base = start_mock_backend(app).await;
    // ProviderContext reloads the saved config and applies runtime env
    // overlays. Pin the backend override to the mock so CI's BACKEND_URL
    // cannot redirect this request to the hosted API.
    let _backend_url_guard = EnvVarGuard::set("BACKEND_URL", &base);
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let _workspace_env_guard = WorkspaceEnvGuard::set(tmp.path());
    config.save().await.unwrap();

    let outcome = composio_get_user_profile(&config, "c1").await.unwrap();

    assert_eq!(outcome.value.toolkit, "gmail");
    assert_eq!(outcome.value.connection_id.as_deref(), Some("c1"));
    assert_eq!(outcome.value.email.as_deref(), Some("pilot@example.com"));
    assert_eq!(outcome.value.display_name.as_deref(), Some("Phoenix Pilot"));
    assert!(outcome.logs.iter().any(|l| l.contains("gmail")));
}

#[tokio::test]
async fn composio_list_tools_via_mock_with_filter() {
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/tools",
        get(|Query(_q): Query<HashMap<String, String>>| async move {
            Json(json!({
                "success": true,
                "data": {"tools": [
                    {"type":"function","function":{"name":"GMAIL_SEND_EMAIL"}},
                    {"type":"function","function":{"name":"GMAIL_SEARCH"}}
                ]}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_list_tools(&config, Some(vec!["gmail".into()]), None)
        .await
        .unwrap();
    assert_eq!(outcome.value.tools.len(), 2);
}

#[tokio::test]
async fn composio_execute_via_mock_succeeds_and_logs_elapsed() {
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(|Json(b): Json<Value>| async move {
            Json(json!({
                "success": true,
                "data": {
                    "data": {"echo": b["tool"]},
                    "successful": true,
                    "error": null,
                    "costUsd": 0.001
                }
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let outcome = composio_execute(&config, "GMAIL_SEND", Some(json!({"to": "a"})), None)
        .await
        .unwrap();
    assert!(outcome.value.successful);
    assert!(outcome
        .logs
        .iter()
        .any(|l| l.contains("executed GMAIL_SEND")));
}

#[tokio::test]
async fn composio_execute_via_mock_propagates_backend_error() {
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/execute",
        post(|| async { Json(json!({"success": false, "error": "rate limited"})) }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base);
    let err = composio_execute(&config, "ANY_TOOL", None, None)
        .await
        .unwrap_err();
    // The dispatcher (`execute_composio_action`) classifies transport
    // failures and prefixes them with `[composio:error:<class>] …`; ops.rs
    // preserves that prefix so the frontend formatter can parse the class.
    // For an unrecognised tool slug and a 502-shaped envelope the only
    // signal we get is the backend error text, so assert on its contents.
    assert!(err.starts_with("[composio:error:"), "got: {err}");
    assert!(err.contains("rate limited"), "got: {err}");
}
