use super::*;

// ── secret_store_for_config ────────────────────────────────────

#[test]
fn secret_store_for_config_scopes_to_config_parent() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // Build the store — must not panic and must operate under tmp path.
    let _store = secret_store_for_config(&config);
}

// ── encrypt_secret / decrypt_secret ───────────────────────────

#[tokio::test]
async fn encrypt_then_decrypt_round_trips_locally() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let plaintext = "top-secret-value";
    let enc = encrypt_secret(&config, plaintext).await.unwrap();
    assert_ne!(enc.value, plaintext);
    let dec = decrypt_secret(&config, &enc.value).await.unwrap();
    assert_eq!(dec.value, plaintext);
}

#[tokio::test]
async fn decrypt_secret_round_trips_noise_through_migrate_path() {
    // `decrypt` accepts legacy plaintext values (migration path) rather
    // than erroring — validate that behaviour by round-tripping a
    // non-ciphertext input. The assertion only checks that we get a
    // deterministic `Ok`, not what the value is.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let res = decrypt_secret(&config, "not-a-real-ciphertext").await;
    assert!(
        res.is_ok(),
        "decrypt should accept non-ciphertext via migrate path, got {res:?}"
    );
}

// ── store_session (input validation) ──────────────────────────

#[tokio::test]
async fn store_session_rejects_empty_or_whitespace_token() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = store_session(&config, "", None, None).await.unwrap_err();
    assert!(err.contains("token is required"));
    let err = store_session(&config, "   ", None, None).await.unwrap_err();
    assert!(err.contains("token is required"));
}

#[test]
fn sanitize_stored_session_user_discards_empty_objects() {
    assert_eq!(sanitize_stored_session_user(Some(json!({}))), None);
    assert_eq!(
        sanitize_stored_session_user(Some(serde_json::Value::Null)),
        None
    );
    assert_eq!(
        sanitize_stored_session_user(Some(json!({ "firstName": "steven" }))),
        Some(json!({ "firstName": "steven" }))
    );
}

#[test]
fn auth_me_store_failure_classifier_only_accepts_transient_shapes() {
    assert!(auth_me_store_failure_is_transient(
        "GET /auth/me failed (503 Service Unavailable): overloaded"
    ));
    assert!(auth_me_store_failure_is_transient(
        "GET /auth/me failed (503 Service Unavailable): session timeout"
    ));
    assert!(auth_me_store_failure_is_transient(
        "GET /auth/me: error sending request for url"
    ));
    assert!(!auth_me_store_failure_is_transient(
        "GET /auth/me failed (401 Unauthorized): bad token"
    ));
    assert!(!auth_me_store_failure_is_transient(
        "GET /auth/me failed (401 Unauthorized): session timeout"
    ));
    assert!(!auth_me_store_failure_is_transient(
        "GET /auth/me failed (403 Forbidden): connection reset"
    ));
}

#[tokio::test]
async fn store_session_rejects_live_jwt_when_auth_me_transient() {
    let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("workspace")).unwrap();
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let mut config = test_config(&tmp);
    config.api_url = Some(spawn_auth_me_status(StatusCode::SERVICE_UNAVAILABLE).await);
    let token = jwt_with_payload(json!({
        "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp()
    }));

    let err = store_session(&config, &token, None, Some(json!({})))
        .await
        .unwrap_err();

    assert!(
        err.contains("Session validation failed (GET /auth/me)"),
        "expected auth/me validation error, got: {err}"
    );
    let state = auth_get_state(&config).await.unwrap().value;
    assert!(!state.is_authenticated);
    assert!(state.user.is_none());
}

#[tokio::test]
async fn store_session_rejects_supplied_user_when_auth_me_transient() {
    let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("workspace")).unwrap();
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let mut config = test_config(&tmp);
    config.api_url = Some(spawn_auth_me_status(StatusCode::SERVICE_UNAVAILABLE).await);
    let token = jwt_with_payload(json!({
        "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp()
    }));

    let err = store_session(
        &config,
        &token,
        None,
        Some(json!({
            "name": "Callback User",
            "email": "callback@example.test"
        })),
    )
    .await
    .unwrap_err();

    assert!(
        err.contains("Session validation failed (GET /auth/me)"),
        "expected auth/me validation error, got: {err}"
    );
    let state = auth_get_state(&config).await.unwrap().value;
    assert!(!state.is_authenticated);
    assert!(state.user.is_none());
}

#[tokio::test]
async fn store_session_rejects_non_object_user_when_auth_me_transient() {
    let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("workspace")).unwrap();
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let mut config = test_config(&tmp);
    config.api_url = Some(spawn_auth_me_status(StatusCode::SERVICE_UNAVAILABLE).await);
    let token = jwt_with_payload(json!({
        "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp()
    }));

    let err = store_session(&config, &token, None, Some(json!("callback-user")))
        .await
        .unwrap_err();

    assert!(
        err.contains("Session validation failed (GET /auth/me)"),
        "expected auth/me validation error, got: {err}"
    );
    let state = auth_get_state(&config).await.unwrap().value;
    assert!(!state.is_authenticated);
    assert!(state.user.is_none());
}

#[tokio::test]
async fn store_session_defers_minimal_live_jwt_when_auth_me_transient_and_allowed() {
    let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("workspace")).unwrap();
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let mut config = test_config(&tmp);
    config.api_url = Some(spawn_auth_me_status(StatusCode::SERVICE_UNAVAILABLE).await);
    let token = jwt_with_payload(json!({
        "sub": "unverified-jwt-user",
        "email": "jwt@example.test",
        "name": "Unverified JWT User",
        "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp()
    }));

    let result = store_session_with_deferred_validation(
        &config,
        &token,
        None,
        Some(json!({
            "id": "supplied-callback-user",
            "name": "Supplied Callback User"
        })),
    )
    .await
    .unwrap();

    assert!(result.value.has_token);
    let log_text = result.logs.join(" ");
    assert!(
        log_text.contains("session JWT accepted with deferred GET /auth/me validation"),
        "expected deferred validation log, got: {log_text}"
    );
    let state = auth_get_state(&config).await.unwrap().value;
    assert!(state.is_authenticated);
    assert_eq!(
        state.user,
        Some(json!({ "pendingBackendValidation": true })),
        "deferred fallback must not copy identity claims from an unverified JWT or callback payload"
    );
}

#[tokio::test]
async fn store_session_defers_live_jwt_when_auth_me_hangs_past_budget() {
    // Issue #5166: a reachable-but-slow backend must not hang store-time
    // validation until the desktop sign-in RPC times out. Capping the budget
    // makes the hang fail fast (as transient) into the deferred-validation path
    // so a live-`exp` JWT still persists instead of bouncing the user.
    let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _budget = EnvVarGuard::set(AUTH_ME_STORE_VALIDATION_BUDGET_ENV, "200");
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("workspace")).unwrap();
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let mut config = test_config(&tmp);
    config.api_url = Some(spawn_auth_me_hang().await);
    let token = jwt_with_payload(json!({
        "sub": "unverified-jwt-user",
        "email": "jwt@example.test",
        "name": "Unverified JWT User",
        "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp()
    }));

    let started = std::time::Instant::now();
    let result = store_session_with_deferred_validation(&config, &token, None, Some(json!({})))
        .await
        .unwrap();
    // The 200ms budget must cap the 60s hang — proves the timeout fired rather
    // than the store awaiting the backend. Bound safely above the 200ms budget
    // but below the 12s default so a multi-second regression still fails.
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "store-time validation should be capped by the budget, took {:?}",
        started.elapsed()
    );

    assert!(result.value.has_token);
    let log_text = result.logs.join(" ");
    assert!(
        log_text.contains("session JWT accepted with deferred GET /auth/me validation"),
        "expected deferred validation log after budget timeout, got: {log_text}"
    );
    let state = auth_get_state(&config).await.unwrap().value;
    assert!(state.is_authenticated);
    assert_eq!(
        state.user,
        Some(json!({ "pendingBackendValidation": true })),
        "budget-timeout fallback must not copy identity claims from an unverified JWT"
    );
}

#[test]
fn auth_me_store_validation_budget_reads_env_override() {
    let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    {
        let _guard = EnvVarGuard::set(AUTH_ME_STORE_VALIDATION_BUDGET_ENV, "750");
        assert_eq!(
            auth_me_store_validation_budget(),
            std::time::Duration::from_millis(750)
        );
    }
    // Invalid / non-positive overrides fall back to the compiled default.
    {
        let _guard = EnvVarGuard::set(AUTH_ME_STORE_VALIDATION_BUDGET_ENV, "0");
        assert_eq!(
            auth_me_store_validation_budget(),
            AUTH_ME_STORE_VALIDATION_BUDGET
        );
    }
    {
        let _guard = EnvVarGuard::set(AUTH_ME_STORE_VALIDATION_BUDGET_ENV, "not-a-number");
        assert_eq!(
            auth_me_store_validation_budget(),
            AUTH_ME_STORE_VALIDATION_BUDGET
        );
    }
}

#[tokio::test]
async fn deferred_session_without_user_id_does_not_replace_active_user_profile() {
    let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let root_dir = default_root_openhuman_dir().unwrap();
    let active_user_id = "existing-active-user";
    write_active_user_id(&root_dir, active_user_id).unwrap();
    let active_user_dir = user_openhuman_dir(&root_dir, active_user_id);
    std::fs::create_dir_all(active_user_dir.join("workspace")).unwrap();
    let mut config = Config {
        config_path: active_user_dir.join("config.toml"),
        workspace_dir: active_user_dir.join("workspace"),
        action_dir: active_user_dir.join("workspace"),
        ..Config::default()
    };
    config.api_url = Some(spawn_auth_me_status(StatusCode::SERVICE_UNAVAILABLE).await);
    let mut metadata = std::collections::HashMap::new();
    metadata.insert("user_id".to_string(), active_user_id.to_string());
    metadata.insert(
        "user_json".to_string(),
        json!({
            "id": active_user_id,
            "name": "Existing Active User"
        })
        .to_string(),
    );
    AuthService::from_config(&config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "existing.active.session",
            metadata,
            true,
        )
        .unwrap();
    let pending_token = jwt_with_payload(json!({
        "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp()
    }));

    let err =
        store_session_with_deferred_validation(&config, &pending_token, None, Some(json!({})))
            .await
            .unwrap_err();

    assert!(
        err.contains("backend user id required before replacing the active session"),
        "expected active-session protection error, got: {err}"
    );
    let state = auth_get_state(&config).await.unwrap().value;
    assert!(state.is_authenticated);
    let token = auth_get_session_token_json(&config).await.unwrap().value;
    assert_eq!(token.get("token"), Some(&json!("existing.active.session")));
    assert_eq!(state.user_id.as_deref(), Some(active_user_id));
    assert_eq!(
        state.user.as_ref().and_then(|value| value.get("id")),
        Some(&json!(active_user_id))
    );
}

#[tokio::test]
async fn store_session_rejects_live_jwt_when_auth_me_unauthorized() {
    let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("workspace")).unwrap();
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let mut config = test_config(&tmp);
    config.api_url = Some(spawn_auth_me_status(StatusCode::UNAUTHORIZED).await);
    let token = jwt_with_payload(json!({
        "exp": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp()
    }));

    let err = store_session(&config, &token, None, None)
        .await
        .unwrap_err();

    assert!(
        err.contains("Session validation failed (GET /auth/me)"),
        "expected auth/me validation error, got: {err}"
    );
    let state = auth_get_state(&config).await.unwrap().value;
    assert!(!state.is_authenticated);
}

/// #5307 — a lapsed session on `GET /auth/me` must stay classifiable as
/// session expiry all the way out of `auth_get_me`.
///
/// The dispatcher (`core::jsonrpc::invoke_method`) keys BOTH behaviours off the
/// error string this function returns: it skips the Sentry report and publishes
/// `DomainEvent::SessionExpired`, which is what makes `SessionExpiredSubscriber`
/// clear the dead JWT. Before #5232 routed `fetch_current_user` through
/// `authed_json`, a 401 surfaced as `"GET /auth/me failed (401 Unauthorized): …"`
/// and matched the dispatcher's HTTP-verb-prefixed 401 rule. The typed
/// `BackendApiError::Unauthorized` renders as
/// `"backend rejected session token on GET /auth/me"` and matches nothing, so on
/// 0.63.9 every lapsed session reported to Sentry as a code defect
/// (TAURI-RUST-RYD) and the stale token was never cleared — so the next
/// revalidation re-fired the same 401 and forced the user out again, forever.
///
/// Pinned through the real `auth_get_me` entry point rather than on the
/// classifier alone: `is_session_expired_error` already recognised the flattened
/// form (`jsonrpc_tests::is_session_expired_error_matches_flattened_backend_unauthorized`)
/// — the only thing wrong was that this call site never produced it.
#[tokio::test]
async fn auth_get_me_401_stays_classifiable_as_session_expiry() {
    let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let mut config = store_live_session("user-5307");
    config.api_url = Some(spawn_auth_me_status(StatusCode::UNAUTHORIZED).await);

    let err = auth_get_me(&config).await.unwrap_err();

    // Assert on the sentinel specifically, not merely on
    // `is_session_expired_message`: the local "session JWT required" guard also
    // satisfies that predicate, so a test that only checked classification
    // would pass even if the backend 401 were never reached.
    assert!(
        err.starts_with("SESSION_EXPIRED:"),
        "a 401 from GET /auth/me must carry the SESSION_EXPIRED sentinel that \
         `flatten_authed_error` produces, so the dispatcher suppresses the Sentry \
         report and publishes SessionExpired (which clears the dead JWT and breaks \
         the forced-logout loop), got: {err}"
    );
    assert!(
        crate::core::observability::is_session_expired_message(&err),
        "the flattened 401 must classify as session expiry, got: {err}"
    );
}

/// The sibling authed route in this module must not regress the same way.
#[tokio::test]
async fn auth_create_channel_link_token_401_stays_classifiable_as_session_expiry() {
    let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let mut config = store_live_session("user-5307");
    let app = Router::new().route(
        "/auth/channels/telegram/link-token",
        axum::routing::post(|| async { StatusCode::UNAUTHORIZED }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    config.api_url = Some(format!("http://{addr}"));

    let err = auth_create_channel_link_token(&config, "telegram")
        .await
        .unwrap_err();

    assert!(
        err.starts_with("SESSION_EXPIRED:"),
        "a 401 on the channel link-token route must carry the SESSION_EXPIRED \
         sentinel, got: {err}"
    );
}

// ── store_session (local session) ─────────────────────────────

/// A local session token requires a non-empty user payload — the backend
/// fetch path is bypassed entirely, so there is no fallback to derive the
/// user from an API response.
#[tokio::test]
async fn store_session_local_token_rejects_missing_user_payload() {
    let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("workspace")).unwrap();
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let config = test_config(&tmp);
    let local_token = "header.payload.local";
    let err = store_session(&config, local_token, None, None)
        .await
        .unwrap_err();
    assert!(
        err.contains("local session requires a user payload"),
        "expected 'local session requires a user payload', got: {err}"
    );
}

/// A local session token with a user payload must be accepted without any
/// network call, must force a deterministic `local-<device>` user id
/// regardless of what the caller passes, and must return a stored profile
/// summary.
#[tokio::test]
async fn store_session_local_token_succeeds_without_network_and_forces_local_user_id() {
    let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("workspace")).unwrap();
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let config = test_config(&tmp);
    let local_token = "header.payload.local";
    let user = serde_json::json!({
        "id": "local",
        "name": "Local User",
        "email": "local@openhuman.local"
    });
    // Pass a different user_id to verify it is overridden.
    let result = store_session(
        &config,
        local_token,
        Some("should-be-overridden".to_string()),
        Some(user),
    )
    .await
    .unwrap();
    // Profile must be stored (no network call was required).
    assert!(
        result.value.has_token,
        "local session should result in a stored token"
    );
    // Logs must mention that backend validation was skipped.
    let log_text = result.logs.join(" ");
    assert!(
        log_text.contains("local session accepted without backend validation"),
        "expected log confirming no backend call, got: {log_text}"
    );
    let expected_local_user_id = local_session_user_id();
    assert!(
        log_text.contains(&format!(
            "user directory activated for {expected_local_user_id}"
        )),
        "expected user-directory activation log for deterministic local uid, got: {log_text}"
    );
    assert!(
        log_text.contains("onboarding left incomplete for local session setup"),
        "expected local session to remain in onboarding, got: {log_text}"
    );
    // The profile_id or metadata must reflect the forced user_id.
    // Because store_session re-activates the user directory and reloads
    // config (so it picks up the user-scoped workspace path), we verify
    // via the returned profile summary rather than a secondary profile lookup.
    assert_eq!(
        result.value.provider, "app-session",
        "profile must be stored under the app-session provider"
    );
}
