//! In-process JSON-RPC E2E coverage for auth.

use crate::config_auth_harness::*;

#[tokio::test]
async fn credentials_session_expired_subscriber_ignores_unrelated_events() {
    let subscriber = SessionExpiredSubscriber::new();
    assert_eq!(subscriber.name(), "credentials::session_expired_handler");
    assert_eq!(subscriber.domains(), Some(&["auth"][..]));

    subscriber
        .handle(&DomainEvent::AgentTurnStarted {
            session_id: "worker-a-session".to_string(),
            channel: "e2e".to_string(),
        })
        .await;
}

/// Sentry 36649: an offline local session ("Continue locally") has no
/// TinyHumans account, so every hosted RPC — the ones that moved into
/// `openhuman-tinyhumans` included — must answer the `BACKEND_UNAVAILABLE:`
/// sentinel the JSON-RPC layer demotes, without a backend request.

#[tokio::test]
async fn hosted_rpcs_answer_backend_unavailable_for_a_local_session() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let local_session = rpc(
        &harness.rpc_base,
        18_201,
        "openhuman.auth_store_session",
        json!({
            "token": "header.payload.local",
            "user": { "id": "local-hosted-36649", "name": "Local Hosted Worker" }
        }),
    )
    .await;
    assert_eq!(
        payload(&local_session, "auth_store_session local")
            .get("credential")
            .and_then(Value::as_str),
        Some("local")
    );

    for (id, method, params) in [
        (18_202, "openhuman.team_get_usage", json!({})),
        (18_203, "openhuman.announcements_get_latest", json!({})),
        (18_204, "openhuman.billing_get_current_plan", json!({})),
        (18_205, "openhuman.webhooks_list_tunnels", json!({})),
        (18_206, "openhuman.channels_telegram_login_start", json!({})),
        (18_207, "openhuman.auth_oauth_list_integrations", json!({})),
        (
            18_208,
            "openhuman.auth_create_channel_link_token",
            json!({ "channel": "telegram" }),
        ),
    ] {
        assert_error_contains(
            &rpc(&harness.rpc_base, id, method, params).await,
            method,
            "BACKEND_UNAVAILABLE:",
        );
    }

    harness.join.abort();
}


#[tokio::test]
async fn credentials_session_expired_subscriber_clears_remote_session_but_keeps_local_session() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    // The host hands the core an already-obtained session; no backend is
    // consulted to store it.
    let remote_session = rpc(
        &harness.rpc_base,
        18_101,
        "openhuman.auth_store_session",
        json!({
            "token": "session-expired-remote-jwt",
            "user_id": "session-expired-user",
            "user": {
                "id": "session-expired-user",
                "name": "Session Expired Worker",
                "email": "session-expired@example.test"
            }
        }),
    )
    .await;
    assert_eq!(
        payload(&remote_session, "auth_store_session before SessionExpired")
            .get("credential")
            .and_then(Value::as_str),
        Some("session")
    );

    let subscriber = SessionExpiredSubscriber::new();
    subscriber
        .handle(&DomainEvent::SessionExpired {
            source: "coverage-test".to_string(),
            reason: "remote token rejected".to_string(),
            credential_fingerprint: Some(
                openhuman_core::credentials::backend_credential::credential_fingerprint(
                    "session-expired-remote-jwt",
                ),
            ),
        })
        .await;

    let cleared_state = rpc(
        &harness.rpc_base,
        18_102,
        "openhuman.auth_get_state",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&cleared_state, "auth_get_state after remote SessionExpired")
            .get("isAuthenticated")
            .and_then(Value::as_bool),
        Some(false),
        "remote SessionExpired events should clear the stored session"
    );

    let local_session = rpc(
        &harness.rpc_base,
        18_103,
        "openhuman.auth_store_session",
        json!({
            "token": "header.payload.local",
            "user": {
                "id": "renderer-local-session-expired",
                "name": "Local Session Expired Worker",
                "email": "local-session-expired@example.test"
            }
        }),
    )
    .await;
    assert_eq!(
        payload(
            &local_session,
            "auth_store_session local before SessionExpired"
        )
        .get("credential")
        .and_then(Value::as_str),
        Some("local")
    );

    subscriber
        .handle(&DomainEvent::SessionExpired {
            source: "coverage-test".to_string(),
            reason: "local token should survive".to_string(),
            credential_fingerprint: None,
        })
        .await;

    let local_state = rpc(
        &harness.rpc_base,
        18_104,
        "openhuman.auth_get_state",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&local_state, "auth_get_state after local SessionExpired")
            .get("isAuthenticated")
            .and_then(Value::as_bool),
        Some(true),
        "local offline sessions should not be cleared by SessionExpired events"
    );

    harness.join.abort();
}


#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_service_direct_paths_cover_profile_selection_and_validation() {
    let tmp = tempdir().expect("tempdir");
    let auth = AuthService::new(tmp.path(), false);
    let store = AuthProfilesStore::new(tmp.path(), false);

    assert_eq!(normalize_provider("  GitHub  ").unwrap(), "github");
    assert!(normalize_provider("   ").is_err());
    assert!(auth.set_active_profile("github", "missing").await.is_err());
    assert_eq!(
        auth.get_provider_bearer_token("github", None).await
            .expect("missing profile lookup"),
        None
    );

    let stored = auth
        .store_provider_token(
            "github",
            "personal",
            " ghp-token ",
            [("scope".to_string(), "repo".to_string())]
                .into_iter()
                .collect(),
            false,
        ).await
        .expect("store token profile");
    assert_eq!(stored.provider, "github");
    assert_eq!(
        auth.get_provider_bearer_token("GitHub", Some("personal")).await
            .expect("profile override lookup"),
        Some(" ghp-token ".to_string())
    );
    assert_eq!(
        auth.get_provider_bearer_token("GitHub", None).await
            .expect("no active/default lookup"),
        Some(" ghp-token ".to_string())
    );

    let active_id = auth
        .set_active_profile("GitHub", "personal").await
        .expect("set active by profile name");
    assert_eq!(active_id, stored.id);
    assert!(auth
        .remove_profile("GitHub", "personal").await
        .expect("remove stored profile"));
    assert!(!auth
        .remove_profile("GitHub", "personal").await
        .expect("remove missing profile"));

    let oauth_profile = AuthProfile::new_oauth(
        "gitlab",
        "main",
        TokenSet {
            access_token: "gitlab-access-token".to_string(),
            refresh_token: Some("gitlab-refresh-token".to_string()),
            id_token: None,
            expires_at: None,
            token_type: Some("Bearer".to_string()),
            scope: Some("read_user".to_string()),
        },
    );
    let oauth_id = oauth_profile.id.clone();
    store
        .upsert_profile(oauth_profile, true).await
        .expect("store oauth profile through shared store");
    assert_eq!(
        auth.get_provider_bearer_token("gitlab", None).await
            .expect("oauth bearer lookup"),
        Some("gitlab-access-token".to_string())
    );
    assert!(auth
        .get_profile("gitlab", Some("missing")).await
        .expect("missing override lookup")
        .is_none());
    let wrong_provider_err = auth
        .set_active_profile("github", &oauth_id).await
        .expect_err("full profile id from another provider should fail")
        .to_string();
    assert!(
        wrong_provider_err.contains("belongs to provider gitlab"),
        "full profile ids must still match the requested provider: {wrong_provider_err}"
    );
}


#[tokio::test]
async fn auth_provider_prefix_listing_sorts_filters_and_excludes_app_session() {
    let _lock = env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        ..Default::default()
    };
    config.secrets.encrypt = false;
    std::fs::create_dir_all(config.config_path.parent().expect("config parent"))
        .expect("create config parent");

    let auth = AuthService::from_config(&config);
    auth.store_provider_token(
        "channel:slack:bot",
        "default",
        "slack-token",
        [("team".to_string(), "T1".to_string())]
            .into_iter()
            .collect(),
        true,
    ).await
    .expect("store slack channel token");
    auth.store_provider_token(
        "channel:telegram:managed_dm",
        "default",
        "telegram-token",
        [("chat_id".to_string(), "42".to_string())]
            .into_iter()
            .collect(),
        false,
    ).await
    .expect("store telegram channel token");
    auth.store_provider_token(
        "github",
        "default",
        "github-token",
        Default::default(),
        true,
    ).await
    .expect("store non-channel token");
    auth.store_provider_token(
        APP_SESSION_PROVIDER,
        "default",
        "session-token",
        Default::default(),
        true,
    ).await
    .expect("store app session token");

    let channels = list_provider_credentials_by_prefix(&config, "channel:")
        .await
        .expect("list channel credentials by prefix");
    let providers = channels
        .iter()
        .map(|profile| profile.provider.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        providers,
        vec!["channel:slack:bot", "channel:telegram:managed_dm"]
    );
    assert!(channels.iter().all(|profile| profile.has_token));
    assert!(channels.iter().all(|profile| !profile.has_token_set));

    let missing = list_provider_credentials_by_prefix(&config, "calendar:")
        .await
        .expect("missing prefix listing should succeed");
    assert!(missing.is_empty());
}


#[tokio::test]
async fn composio_direct_credentials_helpers_trim_store_and_clear_key() {
    let _lock = env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        ..Default::default()
    };
    config.secrets.encrypt = false;
    std::fs::create_dir_all(config.config_path.parent().expect("config parent"))
        .expect("create config parent");

    assert_eq!(
        get_composio_api_key(&config).await.expect("empty composio key store"),
        None
    );
    assert!(
        store_composio_api_key(&config, "   ").await.is_err(),
        "blank composio keys should be rejected"
    );

    let stored = store_composio_api_key(&config, "  cmp_worker_a_secret  ")
        .await
        .expect("store direct composio key");
    assert_eq!(
        stored.value.get("provider").and_then(Value::as_str),
        Some(COMPOSIO_DIRECT_PROVIDER)
    );
    assert_eq!(
        get_composio_api_key(&config).await.expect("stored composio key"),
        Some("cmp_worker_a_secret".to_string())
    );

    let stored_via_rpc = rpc_store_composio_api_key(&config, "cmp_worker_a_second")
        .await
        .expect("store direct composio key via rpc helper");
    assert_eq!(
        stored_via_rpc.value.get("stored").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        get_composio_api_key(&config).await.expect("updated composio key"),
        Some("cmp_worker_a_second".to_string())
    );

    let cleared = clear_composio_api_key(&config)
        .await
        .expect("clear direct composio key");
    assert_eq!(
        cleared.value.get("removed").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        get_composio_api_key(&config).await.expect("cleared composio key"),
        None
    );
    let cleared_again = clear_composio_api_key(&config)
        .await
        .expect("clear missing direct composio key");
    assert_eq!(
        cleared_again.value.get("removed").and_then(Value::as_bool),
        Some(false)
    );
}


#[tokio::test]
async fn credentials_public_ops_cover_service_and_missing_session_error_paths() {
    let _lock = env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let mut config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        ..Default::default()
    };
    config.secrets.encrypt = false;
    config.local_ai.runtime_enabled = false;
    std::fs::create_dir_all(config.config_path.parent().expect("config parent"))
        .expect("create config parent");

    let ctx = openhuman_core::core::runtime::DispatchCtx::host_default();
    openhuman_core::credentials::start_credential_gated_services(&ctx, &config).await;
    openhuman_core::credentials::stop_credential_gated_services(&ctx, &config).await;

    // The account-bound link-token and OAuth ops live in `openhuman-tinyhumans`
    // (`hosted::{channel_link, oauth}`); validation still runs first, and a
    // missing credential fails before any request.
    use openhuman_tinyhumans::hosted::{channel_link, oauth};
    assert!(channel_link::auth_create_channel_link_token(&config, "   ")
        .await
        .expect_err("blank channel should fail")
        .contains("channel is required"));
    assert!(
        channel_link::auth_create_channel_link_token(&config, "matrix")
            .await
            .expect_err("unsupported channel should fail")
            .contains("unsupported channel")
    );
    assert!(
        channel_link::auth_create_channel_link_token(&config, "telegram")
            .await
            .expect_err("missing session should fail")
            .contains("no backend session token")
    );
    assert!(oauth::oauth_connect(
        &config,
        "github",
        Some("skill"),
        Some("code"),
        Some("handoff"),
    )
    .await
    .expect_err("oauth connect without session should fail")
    .contains("no backend session token"));
    assert!(oauth::oauth_list_integrations(&config)
        .await
        .expect_err("oauth list without session should fail")
        .contains("no backend session token"));
    assert!(oauth::oauth_fetch_integration_tokens(
        &config,
        "0123456789abcdef01234567",
        "0123456789abcdef0123456789abcdef",
    )
    .await
    .expect_err("oauth token fetch without session should fail")
    .contains("no backend session token"));
    assert!(
        oauth::oauth_fetch_client_key(&config, "0123456789abcdef01234567")
            .await
            .expect_err("client key fetch without session should fail")
            .contains("no backend session token")
    );
    assert!(
        oauth::oauth_revoke_integration(&config, "0123456789abcdef01234567")
            .await
            .expect_err("oauth revoke without session should fail")
            .contains("no backend session token")
    );
}


#[tokio::test]
async fn credentials_secret_helpers_round_trip_with_file_keyring_backend() {
    let _lock = env_lock_async().await;
    let _keyring_guard = EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file");
    let tmp = tempdir().expect("tempdir");
    let config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        ..Default::default()
    };
    std::fs::create_dir_all(config.config_path.parent().expect("config parent"))
        .expect("create config parent");

    let encrypted = encrypt_secret(&config, "worker-a-sensitive-value")
        .await
        .expect("encrypt secret")
        .value;
    assert_ne!(encrypted, "worker-a-sensitive-value");
    assert!(
        encrypted.starts_with("enc"),
        "encrypted secret should carry an encrypted payload marker: {encrypted}"
    );

    let decrypted = decrypt_secret(&config, &encrypted)
        .await
        .expect("decrypt secret")
        .value;
    assert_eq!(decrypted, "worker-a-sensitive-value");
}


#[tokio::test]
async fn auth_credentials_controller_paths_round_trip_and_validate_errors() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let state = rpc(
        &harness.rpc_base,
        20_001,
        "openhuman.auth_get_state",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&state, "auth_get_state")
            .get("isAuthenticated")
            .and_then(Value::as_bool),
        Some(false)
    );

    let token = rpc(
        &harness.rpc_base,
        20_002,
        "openhuman.auth_get_session_token",
        json!({}),
    )
    .await;
    assert!(
        payload(&token, "auth_get_session_token")
            .get("token")
            .is_some(),
        "session token read should return a token field even when empty: {token}"
    );

    // The core no longer talks to the backend's auth endpoints: a session JWT
    // arrives already obtained, and one that is not tied to a user is refused.
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            20_003,
            "openhuman.auth_set_credential",
            json!({ "token": "opaque-jwt" }),
        )
        .await,
        "auth_set_credential without a user id",
        "userId required",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            20_004,
            "openhuman.auth_set_credential",
            json!({ "token": "sk-1", "kind": "jwt" }),
        )
        .await,
        "auth_set_credential unknown kind",
        "unknown credential kind",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            20_005,
            "openhuman.auth_create_channel_link_token",
            json!({ "channel": "mastodon" }),
        )
        .await,
        "auth_create_channel_link_token unsupported",
        "unsupported channel",
    );

    // `auth_oauth_*` are served by
    // `openhuman-tinyhumans` now: validation first, then the core's
    // credential resolution, whose missing-session wording they report.
    for (id, method, params, needle) in [
        (
            20_006,
            "openhuman.auth_oauth_connect",
            json!({ "provider": "github" }),
            "no backend session token",
        ),
        (
            20_007,
            "openhuman.auth_oauth_list_integrations",
            json!({}),
            "no backend session token",
        ),
        (
            20_008,
            "openhuman.auth_oauth_fetch_integration_tokens",
            json!({ "integrationId": "abc", "key": "secret" }),
            "integrationId must be a 24-char hex id",
        ),
        (
            20_009,
            "openhuman.auth_oauth_fetch_client_key",
            json!({ "integrationId": "abc" }),
            "integrationId must be a 24-char hex id",
        ),
        (
            20_010,
            "openhuman.auth_oauth_revoke_integration",
            json!({ "integrationId": "abc" }),
            "no backend session token",
        ),
    ] {
        let response = rpc(&harness.rpc_base, id, method, params).await;
        assert_error_contains(&response, method, needle);
    }

    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            20_011,
            "openhuman.auth_store_provider_credentials",
            json!({ "provider": "   " }),
        )
        .await,
        "auth_store_provider_credentials empty provider",
        "provider is required",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            20_012,
            "openhuman.auth_store_provider_credentials",
            json!({ "provider": "worker-a", "fields": "not-an-object" }),
        )
        .await,
        "auth_store_provider_credentials invalid fields",
        "fields must be a JSON object",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            20_019,
            "openhuman.auth_store_provider_credentials",
            json!({ "provider": "worker-a-empty" }),
        )
        .await,
        "auth_store_provider_credentials missing credential material",
        "provide at least one credential",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            20_020,
            "openhuman.auth_store_session",
            json!({ "token": "header.payload.local" }),
        )
        .await,
        "auth_store_session local token without user",
        "local session requires a user payload",
    );

    let stored_provider = rpc(
        &harness.rpc_base,
        20_013,
        "openhuman.auth_store_provider_credentials",
        json!({
            "provider": "worker-a",
            "profile": "secondary",
            "token": "provider-secret",
            "fields": { "region": "test" },
            "setActive": false
        }),
    )
    .await;
    assert_eq!(
        payload(&stored_provider, "auth_store_provider_credentials")
            .get("provider")
            .and_then(Value::as_str),
        Some("worker-a")
    );

    let listed = rpc(
        &harness.rpc_base,
        20_014,
        "openhuman.auth_list_provider_credentials",
        json!({ "provider": "worker-a" }),
    )
    .await;
    let listed_payload = payload(&listed, "auth_list_provider_credentials");
    assert!(
        listed_payload
            .as_array()
            .expect("credentials list")
            .iter()
            .any(|profile| profile.get("profileName").and_then(Value::as_str) == Some("secondary")),
        "stored provider profile should be listed: {listed_payload}"
    );

    let listed_without_filter = rpc(
        &harness.rpc_base,
        20_021,
        "openhuman.auth_list_provider_credentials",
        json!({}),
    )
    .await;
    assert!(
        payload(
            &listed_without_filter,
            "auth_list_provider_credentials default params"
        )
        .as_array()
        .expect("unfiltered credentials list")
        .iter()
        .any(|profile| profile.get("provider").and_then(Value::as_str) == Some("worker-a")),
        "unfiltered credentials list should include stored provider: {listed_without_filter}"
    );

    let removed = rpc(
        &harness.rpc_base,
        20_015,
        "openhuman.auth_remove_provider_credentials",
        json!({ "provider": "worker-a", "profile": "secondary" }),
    )
    .await;
    assert_eq!(
        payload(&removed, "auth_remove_provider_credentials")
            .get("removed")
            .and_then(Value::as_bool),
        Some(true)
    );

    let session = rpc(
        &harness.rpc_base,
        20_016,
        "openhuman.auth_store_session",
        json!({
            "token": "header.payload.local",
            "user": {
                "id": "ignored",
                "name": "Worker A",
                "email": "worker-a@example.test"
            }
        }),
    )
    .await;
    assert_eq!(
        payload(&session, "auth_store_session")
            .get("credential")
            .and_then(Value::as_str),
        Some("local")
    );

    let authed = rpc(
        &harness.rpc_base,
        20_017,
        "openhuman.auth_get_state",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&authed, "auth_get_state")
            .get("isAuthenticated")
            .and_then(Value::as_bool),
        Some(true)
    );

    let cleared = rpc(
        &harness.rpc_base,
        20_018,
        "openhuman.auth_clear_session",
        json!({}),
    )
    .await;
    assert!(
        payload(&cleared, "auth_clear_session")
            .get("removed")
            .and_then(Value::as_bool)
            .is_some(),
        "clear session should return removal status: {cleared}"
    );

    harness.join.abort();
}


#[tokio::test]
async fn auth_local_session_normalizes_user_and_app_state_snapshot_uses_stored_identity() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let session = rpc(
        &harness.rpc_base,
        21_001,
        "openhuman.auth_store_session",
        json!({
            "token": "header.payload.local",
            "user": {
                "id": "renderer-supplied-id",
                "name": "Local Worker",
                "email": "local-worker@example.test"
            }
        }),
    )
    .await;
    assert_eq!(
        payload(&session, "auth_store_session local")
            .get("credential")
            .and_then(Value::as_str),
        Some("local")
    );

    let state = rpc(
        &harness.rpc_base,
        21_002,
        "openhuman.auth_get_state",
        json!({}),
    )
    .await;
    let state_payload = payload(&state, "auth_get_state after local session");
    assert_eq!(
        state_payload
            .get("isAuthenticated")
            .and_then(Value::as_bool),
        Some(true)
    );
    let user_id = state_payload
        .get("userId")
        .and_then(Value::as_str)
        .expect("local session should set userId");
    assert!(
        user_id.starts_with("local-"),
        "local session user id should be host-scoped, got {user_id:?}"
    );
    assert_eq!(
        state_payload.pointer("/user/id").and_then(Value::as_str),
        Some(user_id)
    );
    assert_eq!(
        state_payload.pointer("/user/_id").and_then(Value::as_str),
        Some(user_id)
    );

    let token = rpc(
        &harness.rpc_base,
        21_003,
        "openhuman.auth_get_session_token",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&token, "auth_get_session_token after local session")
            .get("token")
            .and_then(Value::as_str),
        Some("header.payload.local")
    );

    let snapshot = rpc(
        &harness.rpc_base,
        21_004,
        "openhuman.app_state_snapshot",
        json!({}),
    )
    .await;
    let snapshot_payload = payload(&snapshot, "app_state_snapshot local session");
    assert_eq!(
        snapshot_payload.get("sessionToken").and_then(Value::as_str),
        Some("header.payload.local")
    );
    assert_eq!(
        snapshot_payload
            .pointer("/currentUser/email")
            .and_then(Value::as_str),
        Some("local-worker@example.test")
    );

    let cleared = rpc(
        &harness.rpc_base,
        21_005,
        "openhuman.auth_clear_session",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&cleared, "auth_clear_session after local session")
            .get("removed")
            .and_then(Value::as_bool),
        Some(true)
    );

    harness.join.abort();
}


#[tokio::test]
async fn auth_remote_backend_bearer_only_paths_round_trip_with_a_handed_over_session() {
    let _lock = env_lock_async().await;
    let (backend_base, backend_state, backend_join) = serve_mock_backend().await;
    let harness = setup().await;
    let _backend_guard = EnvVarGuard::set("BACKEND_URL", &backend_base);

    // The host hands over an already-obtained session with the user it
    // resolved; the core stores it without consulting the backend.
    let session = rpc(
        &harness.rpc_base,
        22_001,
        "openhuman.auth_set_credential",
        json!({
            "token": "remote-jwt",
            "userId": "remote-user-1",
            "user": {
                "id": "remote-user-1",
                "name": "Remote Worker",
                "email": "remote-worker@example.test"
            }
        }),
    )
    .await;
    let state = payload(&session, "auth_set_credential remote");
    assert_eq!(
        state.get("credential").and_then(Value::as_str),
        Some("session")
    );
    assert_eq!(
        state.get("userId").and_then(Value::as_str),
        Some("remote-user-1")
    );
    assert_eq!(
        backend_state.auth_me_hits.load(Ordering::SeqCst),
        0,
        "installing a credential must not touch GET /auth/me"
    );

    // Every remaining auth.* backend call is bearer-only and keeps working
    // against the stored session.
    let link = rpc(
        &harness.rpc_base,
        22_004,
        "openhuman.auth_create_channel_link_token",
        json!({ "channel": " Telegram " }),
    )
    .await;
    assert_eq!(
        payload(&link, "auth_create_channel_link_token remote")
            .get("linkToken")
            .and_then(Value::as_str),
        Some("link-token-123")
    );

    let integrations = rpc(
        &harness.rpc_base,
        22_005,
        "openhuman.auth_oauth_list_integrations",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&integrations, "auth_oauth_list_integrations remote")
            .pointer("/0/provider")
            .and_then(Value::as_str),
        Some("github")
    );

    let oauth_connect = rpc(
        &harness.rpc_base,
        22_011,
        "openhuman.auth_oauth_connect",
        json!({
            "provider": "github",
            "skillId": "worker-a-skill",
            "responseType": "code",
            "encryptionMode": "handoff"
        }),
    )
    .await;
    assert_eq!(
        payload(&oauth_connect, "auth_oauth_connect remote")
            .get("oauthUrl")
            .and_then(Value::as_str),
        Some("https://github.example.test/oauth?state=worker-a-state")
    );

    let integration_tokens = rpc(
        &harness.rpc_base,
        22_012,
        "openhuman.auth_oauth_fetch_integration_tokens",
        json!({
            "integrationId": "0123456789abcdef01234567",
            "key": "0123456789abcdef0123456789abcdef"
        }),
    )
    .await;
    assert_eq!(
        payload(
            &integration_tokens,
            "auth_oauth_fetch_integration_tokens remote"
        )
        .get("accessToken")
        .and_then(Value::as_str),
        Some("gh-access-token")
    );

    let client_key = rpc(
        &harness.rpc_base,
        22_006,
        "openhuman.auth_oauth_fetch_client_key",
        json!({ "integrationId": "0123456789abcdef01234567" }),
    )
    .await;
    assert_eq!(
        payload(&client_key, "auth_oauth_fetch_client_key remote")
            .get("clientKey")
            .and_then(Value::as_str),
        Some("client-key-share")
    );

    let revoked = rpc(
        &harness.rpc_base,
        22_007,
        "openhuman.auth_oauth_revoke_integration",
        json!({ "integrationId": "0123456789abcdef01234567" }),
    )
    .await;
    assert_eq!(
        payload(&revoked, "auth_oauth_revoke_integration remote")
            .get("revoked")
            .and_then(Value::as_bool),
        Some(true)
    );

    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            22_008,
            "openhuman.auth_oauth_fetch_integration_tokens",
            json!({ "integrationId": "short", "key": "secret" }),
        )
        .await,
        "auth_oauth_fetch_integration_tokens invalid id with session",
        "integrationId must be a 24-char hex id",
    );

    // The snapshot reports the handed-over user and never refreshes it.
    let snapshot = rpc(
        &harness.rpc_base,
        22_009,
        "openhuman.app_state_snapshot",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&snapshot, "app_state_snapshot remote")
            .pointer("/currentUser/email")
            .and_then(Value::as_str),
        Some("remote-worker@example.test")
    );
    assert_eq!(
        backend_state.auth_me_hits.load(Ordering::SeqCst),
        0,
        "the snapshot must not call GET /auth/me; the host owns that"
    );

    // Re-handing the same token for the same user is a cheap refresh that
    // replaces the stored payload.
    let refreshed = rpc(
        &harness.rpc_base,
        22_013,
        "openhuman.auth_set_credential",
        json!({
            "token": "remote-jwt",
            "userId": "remote-user-1",
            "user": { "id": "remote-user-1", "name": "Renamed Worker" }
        }),
    )
    .await;
    assert_eq!(
        payload(&refreshed, "auth_set_credential refresh")
            .pointer("/user/name")
            .and_then(Value::as_str),
        Some("Renamed Worker")
    );
    let identity = openhuman_core::credentials::identity::peek_credential_user_identity()
        .expect("set_credential should seed the identity slot");
    assert_eq!(identity.id.as_deref(), Some("remote-user-1"));
    assert_eq!(identity.name.as_deref(), Some("Renamed Worker"));

    let cleared = rpc(
        &harness.rpc_base,
        22_014,
        "openhuman.auth_clear_credential",
        json!({ "kind": "session" }),
    )
    .await;
    assert_eq!(
        payload(&cleared, "auth_clear_credential")
            .get("removedSession")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert!(
        openhuman_core::credentials::identity::peek_credential_user_identity().is_none()
    );

    harness.join.abort();
    backend_join.abort();
}


#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn credentials_profile_store_public_api_persists_updates_and_recovers_bad_files() {
    let _lock = env_lock_async().await;
    let _keyring_guard = EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file");
    let tmp = tempdir().expect("tempdir");
    let state_dir = tmp.path().join("profiles");
    let store = AuthProfilesStore::new(&state_dir, true);

    assert_eq!(store.path(), state_dir.join("auth-profiles/secrets.enc"));
    let empty = store.load().await.expect("empty store load");
    assert!(empty.profiles.is_empty());

    assert!(TokenSet {
        access_token: "soon".to_string(),
        refresh_token: None,
        id_token: None,
        expires_at: Some(chrono::Utc::now() + chrono::Duration::seconds(1)),
        token_type: None,
        scope: None,
    }
    .is_expiring_within(Duration::from_secs(5)));
    assert!(!TokenSet {
        access_token: "no-expiry".to_string(),
        refresh_token: None,
        id_token: None,
        expires_at: None,
        token_type: None,
        scope: None,
    }
    .is_expiring_within(Duration::from_secs(5)));

    let mut token_profile = AuthProfile::new_token("openai", "work", "sk-worker-a".to_string());
    token_profile
        .metadata
        .insert("region".to_string(), "test".to_string());
    let token_id = token_profile.id.clone();
    store
        .upsert_profile(token_profile, true).await
        .expect("upsert token profile");

    let mut oauth_profile = AuthProfile::new_oauth(
        "github",
        "default",
        TokenSet {
            access_token: "gh-access".to_string(),
            refresh_token: Some("gh-refresh".to_string()),
            id_token: Some("gh-id".to_string()),
            expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            token_type: Some("Bearer".to_string()),
            scope: Some("repo user".to_string()),
        },
    );
    oauth_profile.account_id = Some("acct-1".to_string());
    oauth_profile.workspace_id = Some("workspace-1".to_string());
    let oauth_id = oauth_profile.id.clone();
    store
        .upsert_profile(oauth_profile, false).await
        .expect("upsert oauth profile");

    let loaded = store.load().await.expect("load stored profiles");
    assert_eq!(
        loaded.active_profiles.get("openai").map(String::as_str),
        Some(token_id.as_str())
    );
    assert_eq!(
        loaded
            .profiles
            .get(&token_id)
            .and_then(|profile| profile.token.as_deref()),
        Some("sk-worker-a")
    );
    assert_eq!(
        loaded
            .profiles
            .get(&oauth_id)
            .and_then(|profile| profile.token_set.as_ref())
            .map(|tokens| tokens.access_token.as_str()),
        Some("gh-access")
    );

    store
        .set_active_profile("github", &oauth_id).await
        .expect("set active oauth profile");
    store
        .clear_active_profile("openai").await
        .expect("clear active token profile");
    let updated = store
        .update_profile(&oauth_id, |profile| {
            profile
                .metadata
                .insert("updated".to_string(), "true".to_string());
            Ok(())
        }).await
        .expect("update oauth metadata");
    assert_eq!(
        updated.metadata.get("updated").map(String::as_str),
        Some("true")
    );

    assert!(
        store
            .remove_profile(&token_id).await
            .expect("remove existing token profile"),
        "existing token profile should be removed"
    );
    assert!(
        !store
            .remove_profile(&token_id).await
            .expect("remove missing token profile"),
        "missing token profile removal should be idempotent"
    );

    let after_remove = store.load().await.expect("load after remove");
    assert!(!after_remove.profiles.contains_key(&token_id));
    assert_eq!(
        after_remove
            .active_profiles
            .get("github")
            .map(String::as_str),
        Some(oauth_id.as_str())
    );

    let corrupt_dir = tmp.path().join("corrupt");
    std::fs::create_dir_all(&corrupt_dir).expect("create corrupt profile dir");
    let corrupt_path = corrupt_dir.join("auth-profiles.json");
    std::fs::write(&corrupt_path, "{ not valid json").expect("write corrupt profile store");
    let corrupt_store = AuthProfilesStore::new(&corrupt_dir, true);
    let recovered = corrupt_store
        .load().await
        .expect("corrupt profile store recovers");
    assert!(recovered.profiles.is_empty());
    assert!(
        !corrupt_path.exists(),
        "corrupted auth profile store should be moved away"
    );
    assert!(
        std::fs::read_dir(&corrupt_dir)
            .expect("read corrupt dir")
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-")),
        "corrupt auth profile store should leave a quarantine file"
    );

    let legacy_dir = tmp.path().join("legacy");
    std::fs::create_dir_all(&legacy_dir).expect("create legacy profile dir");
    std::fs::write(
        legacy_dir.join("auth-profiles.json"),
        json!({
            "schema_version": 0,
            "updated_at": "2026-01-01T00:00:00Z",
            "active_profiles": {},
            "profiles": {}
        })
        .to_string(),
    )
    .expect("write legacy profile store");
    let legacy = AuthProfilesStore::new(&legacy_dir, true)
        .load().await
        .expect("legacy schema 0 should normalize");
    assert_eq!(legacy.schema_version, 1);

    let future_dir = tmp.path().join("future");
    std::fs::create_dir_all(&future_dir).expect("create future profile dir");
    std::fs::write(
        future_dir.join("auth-profiles.json"),
        json!({
            "schema_version": 99,
            "updated_at": "2026-01-01T00:00:00Z",
            "active_profiles": {},
            "profiles": {}
        })
        .to_string(),
    )
    .expect("write future profile store");
    let future_err = AuthProfilesStore::new(&future_dir, true)
        .load().await
        .expect_err("future schema should fail");
    assert!(
        future_err
            .to_string()
            .contains("Unsupported auth profile schema version"),
        "unexpected future schema error: {future_err:#}"
    );
}


#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn credentials_auth_service_selects_active_default_and_requested_profiles() {
    let _lock = env_lock_async().await;
    let _keyring_guard = EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "encrypted_file");
    let tmp = tempdir().expect("tempdir");
    let service = AuthService::new(&tmp.path().join("auth-service"), false);

    service
        .store_provider_token(
            "github",
            "default",
            "default-token",
            Default::default(),
            false,
        ).await
        .expect("store default github profile");
    assert_eq!(
        service
            .get_provider_bearer_token("github", None).await
            .expect("get default provider token")
            .as_deref(),
        Some("default-token"),
        "without an active profile, provider token lookup should fall back to default"
    );

    let oauth_profile = AuthProfile::new_oauth(
        "github",
        "work",
        TokenSet {
            access_token: "oauth-access".to_string(),
            refresh_token: None,
            id_token: None,
            expires_at: None,
            token_type: Some("Bearer".to_string()),
            scope: Some("repo".to_string()),
        },
    );
    let oauth_id = oauth_profile.id.clone();
    service.load_profiles().await.expect("load before oauth upsert");
    AuthProfilesStore::new(&tmp.path().join("auth-service"), false)
        .upsert_profile(oauth_profile, false).await
        .expect("upsert oauth work profile");
    assert_eq!(
        service
            .set_active_profile(" GITHUB ", "github:work").await
            .expect("set active by full profile id"),
        oauth_id
    );
    assert_eq!(
        service
            .get_provider_bearer_token("github", None).await
            .expect("get active oauth bearer")
            .as_deref(),
        Some("oauth-access"),
        "active OAuth profiles should expose their access token as bearer material"
    );
    assert_eq!(
        service
            .get_provider_bearer_token("github", Some("default")).await
            .expect("explicit default token")
            .as_deref(),
        Some("default-token"),
        "explicit profile overrides should win over active profiles"
    );
    assert_eq!(
        service
            .get_provider_bearer_token("github", Some("missing")).await
            .expect("missing explicit profile")
            .as_deref(),
        None
    );

    let slack_profile = AuthProfile::new_token("slack", "main", "slack-token".to_string());
    let slack_id = slack_profile.id.clone();
    AuthProfilesStore::new(&tmp.path().join("auth-service"), false)
        .upsert_profile(slack_profile, false).await
        .expect("upsert slack profile");
    let mismatch = service
        .set_active_profile("github", &slack_id).await
        .expect_err("provider/profile mismatch should fail");
    assert!(
        mismatch.to_string().contains("belongs to provider slack"),
        "unexpected provider mismatch error: {mismatch:#}"
    );

    assert!(service
        .remove_profile("github", "github:work").await
        .expect("remove active oauth profile"));
}


#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn credentials_profile_store_recovers_dropped_entries_empty_files_and_datetime_errors() {
    let _lock = env_lock_async().await;
    let _keyring_guard = EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file");
    let tmp = tempdir().expect("tempdir");

    let default_profiles =
        openhuman_core::credentials::profiles::AuthProfilesData::default();
    assert_eq!(default_profiles.schema_version, 1);
    assert!(default_profiles.profiles.is_empty());

    let empty_path_store = AuthProfilesStore::new(Path::new(""), false);
    assert_eq!(empty_path_store.path(), Path::new("auth-profiles.json"));

    let empty_dir = tmp.path().join("empty-file");
    std::fs::create_dir_all(&empty_dir).expect("create empty profile dir");
    std::fs::write(empty_dir.join("auth-profiles.json"), "").expect("write empty profile file");
    let empty = AuthProfilesStore::new(&empty_dir, false)
        .load().await
        .expect("empty persisted profile file should load as default");
    assert!(empty.profiles.is_empty());
    assert!(empty.active_profiles.is_empty());

    let mixed_dir = tmp.path().join("mixed");
    std::fs::create_dir_all(&mixed_dir).expect("create mixed profile dir");
    std::fs::write(
        mixed_dir.join("auth-profiles.json"),
        json!({
            "schema_version": 1,
            "updated_at": "2026-01-01T00:00:00Z",
            "active_profiles": {
                "gitlab": "gitlab:main",
                "legacy": "legacy:bad-kind"
            },
            "profiles": {
                "gitlab:main": {
                    "provider": "gitlab",
                    "profile_name": "main",
                    "kind": "token",
                    "token": "plain-gitlab-token",
                    "created_at": "not-a-date",
                    "updated_at": "also-not-a-date",
                    "metadata": {
                        "origin": "fixture"
                    }
                },
                "legacy:bad-kind": {
                    "provider": "legacy",
                    "profile_name": "bad-kind",
                    "kind": "api_key",
                    "token": "drop-me",
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z"
                }
            }
        })
        .to_string(),
    )
    .expect("write mixed profile fixture");
    let mixed_store = AuthProfilesStore::new(&mixed_dir, false);
    let mixed = mixed_store
        .load().await
        .expect("mixed profile store should drop only bad entries");
    assert_eq!(
        mixed
            .profiles
            .get("gitlab:main")
            .and_then(|profile| profile.token.as_deref()),
        Some("plain-gitlab-token")
    );
    assert!(!mixed.profiles.contains_key("legacy:bad-kind"));
    assert!(!mixed.active_profiles.contains_key("legacy"));
    assert_eq!(
        mixed.active_profiles.get("gitlab").map(String::as_str),
        Some("gitlab:main")
    );
    let rewritten: Value = serde_json::from_str(
        &std::fs::read_to_string(mixed_dir.join("auth-profiles.json"))
            .expect("read rewritten mixed profile store"),
    )
    .expect("rewritten mixed store should be json");
    assert!(
        rewritten.pointer("/profiles/legacy:bad-kind").is_none(),
        "dropped profile should be purged from persisted store: {rewritten}"
    );

    let invalid_datetime_dir = tmp.path().join("invalid-datetime");
    std::fs::create_dir_all(&invalid_datetime_dir).expect("create invalid datetime dir");
    std::fs::write(
        invalid_datetime_dir.join("auth-profiles.json"),
        json!({
            "schema_version": 1,
            "updated_at": "2026-01-01T00:00:00Z",
            "active_profiles": {
                "github": "github:oauth"
            },
            "profiles": {
                "github:oauth": {
                    "provider": "github",
                    "profile_name": "oauth",
                    "kind": "oauth",
                    "access_token": "plain-access",
                    "expires_at": "not-rfc3339",
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z"
                }
            }
        })
        .to_string(),
    )
    .expect("write invalid datetime profile fixture");
    let invalid_datetime_err = AuthProfilesStore::new(&invalid_datetime_dir, false)
        .load().await
        .expect_err("invalid oauth expiry should fail profile load");
    assert!(
        invalid_datetime_err
            .to_string()
            .contains("Invalid RFC3339 timestamp"),
        "unexpected invalid datetime error: {invalid_datetime_err:#}"
    );

    let missing_oauth_secret_dir = tmp.path().join("missing-oauth-secret");
    std::fs::create_dir_all(&missing_oauth_secret_dir).expect("create missing oauth secret dir");
    std::fs::write(
        missing_oauth_secret_dir.join("auth-profiles.json"),
        json!({
            "schema_version": 1,
            "updated_at": "2026-01-01T00:00:00Z",
            "active_profiles": {
                "github": "github:missing-access"
            },
            "profiles": {
                "github:missing-access": {
                    "provider": "github",
                    "profile_name": "missing-access",
                    "kind": "oauth",
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z"
                }
            }
        })
        .to_string(),
    )
    .expect("write missing oauth secret fixture");
    let missing_secret = AuthProfilesStore::new(&missing_oauth_secret_dir, false)
        .load().await
        .expect("oauth profile missing access token should be dropped");
    assert!(missing_secret.profiles.is_empty());
    assert!(missing_secret.active_profiles.is_empty());
    let rewritten_missing_secret: Value = serde_json::from_str(
        &std::fs::read_to_string(missing_oauth_secret_dir.join("auth-profiles.json"))
            .expect("read rewritten missing oauth secret profile store"),
    )
    .expect("rewritten missing oauth secret store should be json");
    assert!(
        rewritten_missing_secret
            .pointer("/profiles/github:missing-access")
            .is_none(),
        "missing oauth secret profile should be purged from persisted store: {rewritten_missing_secret}"
    );
    assert!(
        rewritten_missing_secret
            .pointer("/active_profiles/github")
            .is_none(),
        "active pointer to missing oauth secret profile should be purged: {rewritten_missing_secret}"
    );

    let public_api_dir = tmp.path().join("public-api-errors");
    let public_store = AuthProfilesStore::new(&public_api_dir, false);
    assert!(public_store
        .set_active_profile("github", "github:missing").await
        .expect_err("missing active profile should fail")
        .to_string()
        .contains("Auth profile not found"));
    assert!(public_store
        .update_profile("github:missing", |_| Ok(())).await
        .expect_err("missing update profile should fail")
        .to_string()
        .contains("Auth profile not found"));
}


#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn credentials_profile_store_round_trips_oauth_secret_fields_after_backend_selection() {
    let _lock = env_lock_async().await;
    let _keyring_guard = EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "encrypted_file");
    let tmp = tempdir().expect("tempdir");
    let state_dir = tmp.path().join("json-fallback");
    let store = AuthProfilesStore::new(&state_dir, false);
    let profile = AuthProfile::new_oauth(
        "github",
        "json-fallback",
        TokenSet {
            access_token: "json-access".to_string(),
            refresh_token: Some("json-refresh".to_string()),
            id_token: Some("json-id".to_string()),
            expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(2)),
            token_type: Some("Bearer".to_string()),
            scope: Some("repo user".to_string()),
        },
    );
    let profile_id = profile.id.clone();

    store
        .upsert_profile(profile, true).await
        .expect("upsert oauth profile after backend selection");
    let raw: Value = serde_json::from_str(
        &std::fs::read_to_string(state_dir.join("auth-profiles.json"))
            .expect("read persisted auth profiles"),
    )
    .expect("persisted auth profiles json");
    let persisted_access_token = raw.pointer(&format!("/profiles/{profile_id}/access_token"));
    assert!(
        persisted_access_token.is_some_and(|value| {
            value.is_null()
                || value
                    .as_str()
                    .is_some_and(|secret| secret.starts_with("enc2:"))
        }),
        "persisted profile should either keychain-strip or encrypt access token: {raw}"
    );

    let loaded = store.load().await.expect("reload json fallback profile");
    let tokens = loaded
        .profiles
        .get(&profile_id)
        .and_then(|profile| profile.token_set.as_ref())
        .expect("oauth token set should round-trip");
    assert_eq!(tokens.access_token, "json-access");
    assert_eq!(tokens.refresh_token.as_deref(), Some("json-refresh"));
    assert_eq!(tokens.id_token.as_deref(), Some("json-id"));

    let root_path_store = AuthProfilesStore::new(Path::new("/"), false);
    assert_eq!(
        root_path_store.path(),
        Path::new("/").join("auth-profiles.json")
    );
}


#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn credentials_profile_store_keychain_migration_and_fallback_paths_are_deterministic() {
    let _lock = env_lock_async().await;
    let _keyring_guard = EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file");
    let tmp = tempdir().expect("tempdir");

    let hit_dir = tmp.path().join("keychain-hit");
    std::fs::create_dir_all(&hit_dir).expect("create keychain hit dir");
    let hit_profile_id = "github:main";
    openhuman_core::security::secret_storage::set(
        "keychain-hit",
        &format!("auth:{hit_profile_id}"),
        &json!({
            "access_token": "kc-access",
            "refresh_token": "kc-refresh",
            "id_token": "kc-id",
            "token": null
        })
        .to_string(),
    )
    .expect("seed file-backed keyring secret");
    std::fs::write(
        hit_dir.join("auth-profiles.json"),
        json!({
            "schema_version": 1,
            "updated_at": "2026-01-01T00:00:00Z",
            "active_profiles": {
                "github": hit_profile_id
            },
            "profiles": {
                "github:main": {
                    "provider": "github",
                    "profile_name": "main",
                    "kind": "oauth",
                    "access_token": "legacy-access",
                    "refresh_token": "legacy-refresh",
                    "id_token": "legacy-id",
                    "expires_at": "2026-01-01T00:00:00Z",
                    "token_type": "Bearer",
                    "scope": "repo",
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z"
                }
            }
        })
        .to_string(),
    )
    .expect("write keychain hit fixture");
    let hit_store = AuthProfilesStore::new(&hit_dir, true);
    let hit_loaded = hit_store.load().await.expect("keychain hit should load");
    let hit_tokens = hit_loaded
        .profiles
        .get(hit_profile_id)
        .and_then(|profile| profile.token_set.as_ref())
        .expect("keychain hit token set");
    assert_eq!(hit_tokens.access_token, "kc-access");
    assert_eq!(hit_tokens.refresh_token.as_deref(), Some("kc-refresh"));
    let hit_rewritten: Value = serde_json::from_str(
        &std::fs::read_to_string(hit_dir.join("auth-profiles.json"))
            .expect("read rewritten keychain hit profile store"),
    )
    .expect("rewritten keychain hit json");
    assert!(
        hit_rewritten
            .get("profiles")
            .and_then(Value::as_object)
            .and_then(|profiles| profiles.get(hit_profile_id))
            .and_then(|profile| profile.get("access_token"))
            .is_none_or(Value::is_null),
        "legacy JSON secret fields should be cleared after keychain hit: {hit_rewritten}"
    );

    let migrate_dir = tmp.path().join("keychain-migrate");
    std::fs::create_dir_all(&migrate_dir).expect("create keychain migrate dir");
    let migrate_profile_id = "gitlab:work";
    std::fs::write(
        migrate_dir.join("auth-profiles.json"),
        json!({
            "schema_version": 1,
            "updated_at": "2026-01-01T00:00:00Z",
            "active_profiles": {
                "gitlab": migrate_profile_id
            },
            "profiles": {
                "gitlab:work": {
                    "provider": "gitlab",
                    "profile_name": "work",
                    "kind": "token",
                    "token": "plain-token-for-migration",
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z"
                }
            }
        })
        .to_string(),
    )
    .expect("write keychain migration fixture");
    let migrate_store = AuthProfilesStore::new(&migrate_dir, true);
    let migrated = migrate_store
        .load().await
        .expect("plaintext JSON token should migrate to keychain");
    assert_eq!(
        migrated
            .profiles
            .get(migrate_profile_id)
            .and_then(|profile| profile.token.as_deref()),
        Some("plain-token-for-migration")
    );
    let migrated_keychain = openhuman_core::security::secret_storage::get(
        "keychain-migrate",
        &format!("auth:{migrate_profile_id}"),
    )
    .expect("read migrated keychain token")
    .expect("migrated keychain token should exist");
    assert!(
        migrated_keychain.contains("plain-token-for-migration"),
        "migrated keychain payload should contain redacted fixture token"
    );

    let fallback_dir = tmp.path().join("keychain-fallback");
    std::fs::create_dir_all(&fallback_dir).expect("create keychain fallback dir");
    let fallback_profile_id = "slack:bot";
    openhuman_core::security::secret_storage::set(
        "keychain-fallback",
        &format!("auth:{fallback_profile_id}"),
        "not-json",
    )
    .expect("seed malformed keychain payload");
    std::fs::write(
        fallback_dir.join("auth-profiles.json"),
        json!({
            "schema_version": 1,
            "updated_at": "2026-01-01T00:00:00Z",
            "active_profiles": {
                "slack": fallback_profile_id
            },
            "profiles": {
                "slack:bot": {
                    "provider": "slack",
                    "profile_name": "bot",
                    "kind": "token",
                    "token": "json-fallback-token",
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z"
                }
            }
        })
        .to_string(),
    )
    .expect("write keychain fallback fixture");
    let fallback = AuthProfilesStore::new(&fallback_dir, true)
        .load().await
        .expect("malformed keychain payload should fall back to JSON");
    assert_eq!(
        fallback
            .profiles
            .get(fallback_profile_id)
            .and_then(|profile| profile.token.as_deref()),
        Some("json-fallback-token")
    );

    assert!(
        AuthProfilesStore::new(&migrate_dir, true)
            .remove_profile(migrate_profile_id).await
            .expect("remove migrated profile"),
        "migrated profile should be removable"
    );
    assert!(
        openhuman_core::security::secret_storage::get(
            "keychain-migrate",
            &format!("auth:{migrate_profile_id}"),
        )
        .expect("read deleted migrated keychain token")
        .is_none(),
        "removing a profile should delete its keychain payload"
    );
}
