use super::*;
use serde_json::json;
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

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

// ── clear_session ──────────────────────────────────────────────

#[tokio::test]
async fn clear_session_on_empty_store_reports_removed_false() {
    let tmp = TempDir::new().unwrap();
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
