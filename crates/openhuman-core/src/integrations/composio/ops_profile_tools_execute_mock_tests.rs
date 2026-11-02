use super::*;

#[tokio::test]
async fn composio_get_user_profile_via_mock_returns_provider_profile() {
    let _serialised = module_guard().await;
    // The embedding seam fails loudly when unwired. Installed here rather
    // than relied upon from another test: `install_for_tests` is
    // `Once`-guarded, so a test that omits it passes only while some
    // earlier test in the same binary happened to run first.
    use crate::config::TEST_ENV_LOCK;
    let _cache_guard = cache_guard_async().await;
    let _env_guard = TEST_ENV_LOCK.lock().await;
    // This test mutates BACKEND_URL below via EnvVarGuard, which races with
    // openhuman_tinyhumans::backend::url / core::cli_tests tests
    // that mutate the same process-global var under the crate-wide lock —
    // TEST_ENV_LOCK alone does not serialize against those. Hold both.
    let _backend_env_guard = crate::config::app_env::env_test_lock_async().await;

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
    let config = config_with_backend(&tmp, base).await;
    let _workspace_env_guard = EnvVarGuard::workspace_unlocked(tmp.path());
    config.save().await.unwrap();

    let outcome =
        composio_get_user_profile(&crate::core::runtime::DispatchCtx::ambient(), &config, "c1")
            .await
            .unwrap();

    assert_eq!(outcome.value.toolkit, "gmail");
    assert_eq!(outcome.value.connection_id.as_deref(), Some("c1"));
    assert_eq!(outcome.value.email.as_deref(), Some("pilot@example.com"));
    assert_eq!(outcome.value.display_name.as_deref(), Some("Phoenix Pilot"));
    assert!(outcome.logs.iter().any(|l| l.contains("gmail")));
}

#[tokio::test]
async fn composio_list_tools_via_mock_filters_tags_by_toolkit() {
    let _serialised = module_guard().await;
    let observed_queries = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let handler_queries = std::sync::Arc::clone(&observed_queries);
    let app = Router::new().route(
        "/agent-integrations/composio/tools",
        get(move |Query(query): Query<HashMap<String, String>>| {
            let observed = std::sync::Arc::clone(&handler_queries);
            async move {
                observed.lock().unwrap().push(query);
                Json(json!({
                    "success": true,
                    "data": {"tools": [
                        {"type":"function","function":{"name":"GMAIL_SEND_EMAIL"}},
                        {"type":"function","function":{"name":"GMAIL_FETCH_EMAILS"}}
                    ]}
                }))
            }
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base).await;

    let gmail = composio_list_tools(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        Some(vec!["gmail".into()]),
        Some(vec!["stars".into()]),
    )
    .await
    .unwrap();
    assert_eq!(gmail.value.tools.len(), 2);

    composio_list_tools(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        Some(vec!["github".into()]),
        Some(vec!["stars".into()]),
    )
    .await
    .unwrap();

    let queries = observed_queries.lock().unwrap();
    assert_eq!(queries.len(), 2);
    assert!(
        !queries[0].contains_key("tags"),
        "non-GitHub toolkit must not forward tags: {:?}",
        queries[0]
    );
    assert_eq!(queries[1].get("tags").map(String::as_str), Some("stars"));
}

#[tokio::test]
async fn composio_list_tools_lets_the_module_apply_the_scope_preference_and_catalog() {
    // The host no longer opts out of the module's scope filter
    // (`apply_user_scopes` defaults on): an admin-scoped action, and one the
    // gmail catalog does not name, never reach the caller.
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/tools",
        get(|| async {
            Json(json!({
                "success": true,
                "data": {"tools": [
                    {"type":"function","function":{"name":"GMAIL_FETCH_EMAILS"}},
                    {"type":"function","function":{"name":"GMAIL_DELETE_MESSAGE"}},
                    {"type":"function","function":{"name":"GMAIL_NOT_IN_THE_CATALOG"}}
                ]}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base).await;
    let outcome = composio_list_tools(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        Some(vec!["gmail".into()]),
        None,
    )
    .await
    .unwrap();
    let names: Vec<&str> = outcome
        .value
        .tools
        .iter()
        .map(|tool| tool.function.name.as_str())
        .collect();
    assert_eq!(names, ["GMAIL_FETCH_EMAILS"]);
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
    let config = config_with_backend(&tmp, base).await;
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
    let config = config_with_backend(&tmp, base).await;
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

#[tokio::test]
async fn composio_list_tools_honours_a_preference_still_in_the_legacy_file() {
    // A workspace upgraded from before the preference moved into the module
    // still has `composio_user_scopes.json`. The module filters the listing by
    // the preference it holds, so that file must be migrated before the call;
    // otherwise its default (`write` on) outranks the user's stored `write=false`.
    let _serialised = module_guard().await;
    let app = Router::new().route(
        "/agent-integrations/composio/tools",
        get(|| async {
            Json(json!({
                "success": true,
                "data": {"tools": [
                    {"type":"function","function":{"name":"GMAIL_FETCH_EMAILS"}},
                    {"type":"function","function":{"name":"GMAIL_SEND_EMAIL"}}
                ]}
            }))
        }),
    );
    let base = start_mock_backend(app).await;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_with_backend(&tmp, base).await;
    crate::integrations::composio::file_store::save(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        crate::integrations::composio::file_store::USER_SCOPES_FILE,
        &json!({"gmail": {"read": true, "write": false, "admin": false}}),
    )
    .await
    .unwrap();

    let outcome = composio_list_tools(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        Some(vec!["gmail".into()]),
        None,
    )
    .await;
    // The module's preference store is process-wide: put the default back
    // before asserting so a failure cannot leak `write=false` into other tests.
    let restored = save_user_scope_pref(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "gmail",
        Default::default(),
    )
    .await;
    let outcome = outcome.unwrap();
    restored.unwrap();

    let names: Vec<&str> = outcome
        .value
        .tools
        .iter()
        .map(|tool| tool.function.name.as_str())
        .collect();
    assert_eq!(names, ["GMAIL_FETCH_EMAILS"]);
}
