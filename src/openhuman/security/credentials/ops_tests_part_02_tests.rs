use super::*;

#[test]
fn normalize_local_session_user_overwrites_id_fields() {
    let out = normalize_local_session_user(
        json!({
            "id": "old",
            "_id": "old",
            "name": "Local User"
        }),
        "local-device-123",
    );

    assert_eq!(out["id"], "local-device-123");
    assert_eq!(out["_id"], "local-device-123");
    assert_eq!(out["name"], "Local User");
}

// ── clear_session ──────────────────────────────────────────────

#[tokio::test]
async fn clear_session_on_empty_store_reports_removed_false() {
    // `clear_session` clears the active-user marker under
    // `default_root_openhuman_dir()`, which is derived from the *process-global*
    // HOME. Without pinning HOME to this test's tempdir (under the shared env
    // lock) it deletes whichever concurrently-running test currently owns HOME —
    // e.g. `deferred_session_without_user_id_does_not_replace_active_user_profile`,
    // whose active-session guard then silently stops firing.
    let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let config = test_config(&tmp);
    let result = clear_session(&config).await.unwrap();
    assert_eq!(result.value["removed"], false);
}

// ── auth_get_state / auth_get_session_token_json ──────────────

#[tokio::test]
async fn auth_get_state_reflects_empty_store() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let state = auth_get_state(&config).await.unwrap();
    assert!(!state.value.is_authenticated);
    assert!(state.value.profile_id.is_none());
}

#[tokio::test]
async fn auth_get_session_token_json_returns_null_when_empty() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let out = auth_get_session_token_json(&config).await.unwrap();
    assert!(out.value["token"].is_null());
}

// ── consume_login_token (input validation) ────────────────────

#[tokio::test]
async fn consume_login_token_rejects_empty() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = consume_login_token(&config, "  ").await.unwrap_err();
    assert!(err.contains("loginToken is required"));
}

// ── auth_create_channel_link_token (validation) ───────────────

#[tokio::test]
async fn auth_create_channel_link_token_rejects_empty_channel() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = auth_create_channel_link_token(&config, "   ")
        .await
        .unwrap_err();
    assert!(err.contains("channel is required"));
}

#[tokio::test]
async fn auth_create_channel_link_token_rejects_unsupported_channel() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = auth_create_channel_link_token(&config, "Slack")
        .await
        .unwrap_err();
    assert!(err.contains("unsupported channel"));
}

// ── store_provider_credentials (validation + store path) ──────

#[tokio::test]
async fn store_provider_credentials_rejects_empty_provider() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = store_provider_credentials(&config, "  ", None, None, None, None)
        .await
        .unwrap_err();
    assert!(err.contains("provider is required"));
}

#[tokio::test]
async fn store_provider_credentials_rejects_when_no_credentials_supplied() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = store_provider_credentials(&config, "openai", None, None, None, None)
        .await
        .unwrap_err();
    assert!(err.contains("at least one credential"));
}

#[tokio::test]
async fn store_provider_credentials_rejects_blank_token_without_fields() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = store_provider_credentials(&config, "openai", None, Some("   ".into()), None, None)
        .await
        .unwrap_err();
    assert!(err.contains("at least one credential"));
}

#[tokio::test]
async fn store_provider_credentials_stores_token_and_persists_to_disk() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let result = store_provider_credentials(
        &config,
        "openai",
        Some("default"),
        Some("sk-test".into()),
        None,
        Some(true),
    )
    .await
    .unwrap();
    assert_eq!(result.value.provider, "openai");
    assert_eq!(result.value.profile_name, "default");
    assert!(result.value.has_token);

    let listed = list_provider_credentials(&config, None).await.unwrap();
    assert_eq!(listed.value.len(), 1);
    assert_eq!(listed.value[0].provider, "openai");
}

#[tokio::test]
async fn store_provider_credentials_extracts_token_from_fields() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let result = store_provider_credentials(
        &config,
        "openai",
        None,
        None,
        Some(json!({ "token": "from-fields", "extra": "value" })),
        None,
    )
    .await
    .unwrap();
    assert!(result.value.has_token);
}

#[tokio::test]
async fn store_provider_credentials_extracts_api_key_from_fields() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let result = store_provider_credentials(
        &config,
        "openai",
        None,
        None,
        Some(json!({ "api_key": "from-api-key-field" })),
        None,
    )
    .await
    .unwrap();
    assert!(result.value.has_token);
}

#[tokio::test]
async fn store_provider_credentials_accepts_fields_only_without_token() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // Non-empty fields but no token — should succeed as "credential via fields".
    let result = store_provider_credentials(
        &config,
        "custom",
        None,
        None,
        Some(json!({ "api_url": "https://custom.example" })),
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.value.provider, "custom");
}

// ── remove_provider_credentials ────────────────────────────────

#[tokio::test]
async fn remove_provider_credentials_reports_false_when_missing() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let result = remove_provider_credentials(&config, "nope", None)
        .await
        .unwrap();
    assert_eq!(result.value["removed"], false);
}

#[tokio::test]
async fn remove_provider_credentials_reports_true_after_store() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    store_provider_credentials(&config, "openai", None, Some("sk".into()), None, Some(true))
        .await
        .unwrap();
    let result = remove_provider_credentials(&config, "openai", None)
        .await
        .unwrap();
    assert_eq!(result.value["removed"], true);
}

// ── list_provider_credentials ─────────────────────────────────

#[tokio::test]
async fn list_provider_credentials_is_empty_for_fresh_store() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let result = list_provider_credentials(&config, None).await.unwrap();
    assert!(result.value.is_empty());
}

#[tokio::test]
async fn list_provider_credentials_filters_by_provider_and_excludes_app_session() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // Seed openai + anthropic + an app-session entry.
    store_provider_credentials(&config, "openai", None, Some("sk".into()), None, Some(true))
        .await
        .unwrap();
    store_provider_credentials(
        &config,
        "anthropic",
        None,
        Some("sk-ant".into()),
        None,
        Some(true),
    )
    .await
    .unwrap();
    let auth = AuthService::from_config(&config);
    auth.store_provider_token(
        APP_SESSION_PROVIDER,
        DEFAULT_AUTH_PROFILE_NAME,
        "jwt-token",
        std::collections::HashMap::new(),
        true,
    )
    .unwrap();

    let all = list_provider_credentials(&config, None).await.unwrap();
    let providers: Vec<&str> = all.value.iter().map(|p| p.provider.as_str()).collect();
    assert!(providers.contains(&"openai"));
    assert!(providers.contains(&"anthropic"));
    // app-session profile must be excluded from the listing.
    assert!(!providers.contains(&APP_SESSION_PROVIDER));

    let filtered = list_provider_credentials(&config, Some("openai".into()))
        .await
        .unwrap();
    assert_eq!(filtered.value.len(), 1);
    assert_eq!(filtered.value[0].provider, "openai");
}

#[tokio::test]
async fn list_provider_credentials_sorts_by_provider_then_profile_name() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    store_provider_credentials(
        &config,
        "zeta",
        Some("one"),
        Some("t".into()),
        None,
        Some(true),
    )
    .await
    .unwrap();
    store_provider_credentials(
        &config,
        "alpha",
        Some("b"),
        Some("t".into()),
        None,
        Some(true),
    )
    .await
    .unwrap();
    store_provider_credentials(
        &config,
        "alpha",
        Some("a"),
        Some("t".into()),
        None,
        Some(true),
    )
    .await
    .unwrap();

    let all = list_provider_credentials(&config, None).await.unwrap();
    assert_eq!(all.value.len(), 3);
    assert_eq!(all.value[0].provider, "alpha");
    assert_eq!(all.value[0].profile_name, "a");
    assert_eq!(all.value[1].provider, "alpha");
    assert_eq!(all.value[1].profile_name, "b");
    assert_eq!(all.value[2].provider, "zeta");
}

// ── oauth_* (validation paths that don't require network) ─────

#[tokio::test]
async fn oauth_connect_errors_without_session_token() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = oauth_connect(&config, "notion", None, None, None)
        .await
        .unwrap_err();
    assert!(err.contains("session JWT required"));
}

#[tokio::test]
async fn oauth_list_integrations_errors_without_session() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = oauth_list_integrations(&config).await.unwrap_err();
    assert!(err.contains("session JWT required"));
}

#[tokio::test]
async fn oauth_fetch_integration_tokens_errors_without_session() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = oauth_fetch_integration_tokens(&config, "int-1", "enc-key")
        .await
        .unwrap_err();
    assert!(err.contains("session JWT required"));
}

#[tokio::test]
async fn oauth_fetch_client_key_errors_without_session() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = oauth_fetch_client_key(&config, "int-1").await.unwrap_err();
    assert!(err.contains("session JWT required"));
}

#[tokio::test]
async fn oauth_revoke_integration_errors_without_session() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = oauth_revoke_integration(&config, "int-1")
        .await
        .unwrap_err();
    assert!(err.contains("session JWT required"));
}

#[tokio::test]
async fn auth_get_me_errors_without_session() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = auth_get_me(&config).await.unwrap_err();
    assert!(err.contains("session JWT required"));
}

// ── list_provider_credentials_by_prefix ───────────────────────

/// Issue #1149 root-cause regression: the exact-match filter on
/// `list_provider_credentials` cannot enumerate provider keys grouped
/// under a common stem (e.g. `channel:telegram:managed_dm`,
/// `channel:slack:bot_token`). The prefix variant fixes that — without
/// it, `channel_status` always returned `connected: false`.
#[tokio::test]
async fn list_provider_credentials_by_prefix_matches_namespaced_keys() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    for provider in [
        "channel:telegram:managed_dm",
        "channel:slack:bot_token",
        "skill:notion",
    ] {
        store_provider_credentials(
            &config,
            provider,
            None,
            Some("token-x".to_string()),
            None,
            Some(true),
        )
        .await
        .expect("seed credential");
    }

    let channels = list_provider_credentials_by_prefix(&config, "channel:")
        .await
        .expect("prefix list should succeed");
    let providers: Vec<&str> = channels.iter().map(|p| p.provider.as_str()).collect();

    assert_eq!(channels.len(), 2, "got {providers:?}");
    assert!(providers.contains(&"channel:slack:bot_token"));
    assert!(providers.contains(&"channel:telegram:managed_dm"));
}

#[tokio::test]
async fn list_provider_credentials_by_prefix_returns_empty_when_no_match() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    store_provider_credentials(
        &config,
        "skill:notion",
        None,
        Some("token-x".to_string()),
        None,
        Some(true),
    )
    .await
    .expect("seed credential");

    let result = list_provider_credentials_by_prefix(&config, "channel:")
        .await
        .expect("prefix list should succeed");
    assert!(result.is_empty(), "got {result:?}");
}

// ── Account-scoped storage isolation ──────────────────────────────────────
//
// The credential store is scoped to `config.workspace_dir` / `config.config_path`.
// Two configs pointing at different directories must not share credential data.
// This models the multi-account scenario: each user account activates a
// different `workspace_dir`, so credentials stored under one account must be
// completely invisible to a different account's config.

#[tokio::test]
async fn credentials_stored_under_one_workspace_dir_invisible_to_another() {
    let tmp_a = TempDir::new().unwrap();
    let tmp_b = TempDir::new().unwrap();
    let config_a = test_config(&tmp_a);
    let config_b = test_config(&tmp_b);

    // Store an OpenAI credential under account A.
    store_provider_credentials(
        &config_a,
        "openai",
        Some("default"),
        Some("sk-account-a".to_string()),
        None,
        Some(true),
    )
    .await
    .expect("store under config_a");

    // Account B's store must be empty — it has its own workspace_dir.
    let listed_b = list_provider_credentials(&config_b, None)
        .await
        .expect("list from config_b");
    assert!(
        listed_b.value.is_empty(),
        "credentials from account A must not be visible to account B, got: {:?}",
        listed_b.value
    );
}

#[tokio::test]
async fn clear_session_on_one_account_does_not_affect_another() {
    // See `clear_session_on_empty_store_reports_removed_false`: `clear_session`
    // reaches the HOME-derived root, so this test must own HOME while it runs.
    let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp_a = TempDir::new().unwrap();
    let tmp_b = TempDir::new().unwrap();
    let _home = EnvVarGuard::set_to_path("HOME", tmp_a.path());
    let config_a = test_config(&tmp_a);
    let config_b = test_config(&tmp_b);

    // Store an OpenAI credential under each account.
    store_provider_credentials(
        &config_a,
        "openai",
        None,
        Some("sk-a".to_string()),
        None,
        Some(true),
    )
    .await
    .unwrap();
    store_provider_credentials(
        &config_b,
        "openai",
        None,
        Some("sk-b".to_string()),
        None,
        Some(true),
    )
    .await
    .unwrap();

    // Clearing the session for account A must not wipe account B's credentials.
    clear_session(&config_a).await.unwrap();

    let listed_b = list_provider_credentials(&config_b, None)
        .await
        .expect("list from config_b after clear_session on config_a");
    assert_eq!(
        listed_b.value.len(),
        1,
        "account B credential must survive clear_session on account A"
    );
    assert_eq!(listed_b.value[0].provider, "openai");
}

#[tokio::test]
async fn each_account_workspace_holds_its_own_credential_data() {
    // Two accounts store credentials under distinct workspace dirs.
    // Listing with each config must see only its own data, never the other's.
    let tmp_a = TempDir::new().unwrap();
    let tmp_b = TempDir::new().unwrap();
    let config_a = test_config(&tmp_a);
    let config_b = test_config(&tmp_b);

    store_provider_credentials(
        &config_a,
        "anthropic",
        None,
        Some("sk-ant-a".to_string()),
        None,
        Some(true),
    )
    .await
    .unwrap();
    store_provider_credentials(
        &config_b,
        "anthropic",
        None,
        Some("sk-ant-b".to_string()),
        None,
        Some(true),
    )
    .await
    .unwrap();

    let result_a = list_provider_credentials(&config_a, Some("anthropic".into()))
        .await
        .unwrap();
    let result_b = list_provider_credentials(&config_b, Some("anthropic".into()))
        .await
        .unwrap();

    assert_eq!(
        result_a.value.len(),
        1,
        "config_a must see exactly its own anthropic credential"
    );
    assert_eq!(
        result_b.value.len(),
        1,
        "config_b must see exactly its own anthropic credential"
    );
    // Sanity: both found their own entry, neither crossed over.
    assert_eq!(result_a.value[0].provider, "anthropic");
    assert_eq!(result_b.value[0].provider, "anthropic");
}

/// #3490 regression: `start_login_gated_services` must launch its services
/// concurrently (independent `tokio::spawn` tasks) and return, rather than
/// awaiting them serially. With an all-disabled config every `start_if_enabled`
/// is a no-op, so this drives the spawn/await/join orchestration (the changed
/// code path) deterministically — no microphone, model, or network is touched —
/// and asserts the function completes promptly instead of blocking.
///
/// A serial regression here would still pass functionally, but the concurrent
/// structure is what collapses the readiness latency from the sum of the
/// per-service cold-starts to the slowest single one; this test guards that the
/// orchestration keeps returning (and never deadlocks/panics on join).
#[tokio::test]
async fn start_login_gated_services_completes_with_all_services_disabled() {
    // Serialize with the other env-mutating tests: this test sets a process-wide
    // opt-in env var below, and the lock keeps it from leaking into a
    // concurrently-running `store_session` test that would then start real
    // background services. (These are the same semantics `TEST_ENV_LOCK` gives
    // the HOME-mutating tests.)
    let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    // Under `#[cfg(test)]` `start_login_gated_services` skips the real services
    // by default (they leak across the parallel test run); opt this one test
    // back in so it actually drives the concurrent spawn/await path it guards.
    // Only presence is checked, so the value (a temp path) is irrelevant.
    let _run_services =
        EnvVarGuard::set_to_path("OPENHUMAN_RUN_LOGIN_GATED_SERVICES_IN_TEST", tmp.path());

    let mut config = Config::default();
    // Every service is disabled so each `start_if_enabled` is a no-op: the test
    // exercises the concurrent spawn/await machinery (the changed code) without
    // touching the mic, a model, the screen, or the network.
    config.local_ai.runtime_enabled = false;
    config.voice_server.auto_start = false;
    config.voice_server.always_on_enabled = false;

    // Bound the wait so a serial-blocking regression (or a hung join) fails the
    // test instead of hanging CI. Every service no-ops, so this resolves almost
    // immediately; the generous ceiling only guards against a deadlock.
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        start_login_gated_services(&config),
    )
    .await
    .expect("start_login_gated_services must return, not block");
}
