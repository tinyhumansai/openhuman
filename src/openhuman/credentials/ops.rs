//! JSON-RPC / CLI controller surface for credentials and app session auth.

use serde_json::json;

use crate::api::config::effective_api_url;
use crate::api::jwt::get_session_token;
use crate::api::rest::{user_id_from_profile_payload, BackendOAuthClient};
use crate::openhuman::config::Config;
use crate::openhuman::credentials::session_support::{
    build_session_state, parse_fields_value, profile_name_or_default, summarize_auth_profile,
};
use crate::openhuman::security::SecretStore;
use crate::rpc::RpcOutcome;

use super::{AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};
use crate::openhuman::config::{
    default_root_openhuman_dir, user_openhuman_dir, write_active_user_id,
};

fn secret_store_for_config(config: &Config) -> SecretStore {
    let data_dir = config
        .config_path
        .parent()
        .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
    SecretStore::new(&data_dir, true)
}

pub async fn encrypt_secret(
    config: &Config,
    plaintext: &str,
) -> Result<RpcOutcome<String>, String> {
    let store = secret_store_for_config(config);
    let ciphertext = store.encrypt(plaintext).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(ciphertext, "secret encrypted"))
}

pub async fn decrypt_secret(
    config: &Config,
    ciphertext: &str,
) -> Result<RpcOutcome<String>, String> {
    let store = secret_store_for_config(config);
    let plaintext = store.decrypt(ciphertext).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(plaintext, "secret decrypted"))
}

pub async fn store_session(
    config: &Config,
    token: &str,
    user_id: Option<String>,
    user: Option<serde_json::Value>,
) -> Result<RpcOutcome<super::responses::AuthProfileSummary>, String> {
    let trimmed_token = token.trim();
    if trimmed_token.is_empty() {
        return Err("token is required".to_string());
    }

    let api_url = effective_api_url(&config.api_url);

    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let settings = client
        .fetch_current_user(trimmed_token)
        .await
        .map_err(|e| format!("Session validation failed (GET /auth/me): {e:#}"))?;

    let mut metadata = std::collections::HashMap::new();
    if let Some(uid) = user_id
        .and_then(|v| {
            let t = v.trim().to_string();
            (!t.is_empty()).then_some(t)
        })
        .or_else(|| user_id_from_profile_payload(&settings))
    {
        metadata.insert("user_id".to_string(), uid);
    }
    let user_for_store = user.unwrap_or(settings);
    metadata.insert("user_json".to_string(), user_for_store.to_string());

    // Determine user_id so we can scope the openhuman directory to this user.
    let resolved_user_id = metadata.get("user_id").cloned();

    // If we know the user_id, activate the user-scoped directory BEFORE storing
    // the auth profile so that credentials land in the correct place.
    let mut logs = vec![format!(
        "session JWT verified via GET /auth/me on {}",
        api_url.trim_end_matches('/')
    )];

    if let Some(ref uid) = resolved_user_id {
        if let Ok(root_dir) = default_root_openhuman_dir() {
            let user_dir = user_openhuman_dir(&root_dir, uid);
            if let Err(e) = std::fs::create_dir_all(&user_dir) {
                tracing::warn!(
                    user_id = %uid,
                    error = %e,
                    "failed to create user directory"
                );
            } else if let Err(e) = write_active_user_id(&root_dir, uid) {
                tracing::warn!(
                    user_id = %uid,
                    error = %e,
                    "failed to write active_user.toml"
                );
            } else {
                logs.push(format!("user directory activated for {uid}"));
                tracing::info!(
                    user_id = %uid,
                    user_dir = %user_dir.display(),
                    "User-scoped directory activated"
                );
            }
        }
    }

    // Reload config so it picks up the newly activated user directory.
    // This ensures auth-profiles.json, encryption key, etc. are written
    // to the user-scoped location.
    let effective_config = if resolved_user_id.is_some() {
        match crate::openhuman::config::load_config_with_timeout().await {
            Ok(c) => c,
            Err(_) => config.clone(),
        }
    } else {
        config.clone()
    };

    let auth = AuthService::from_config(&effective_config);
    let profile = auth
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            trimmed_token,
            metadata,
            true,
        )
        .map_err(|e| e.to_string())?;

    logs.push("session stored".to_string());

    // Now that active_user.toml exists and config.workspace_dir resolves to
    // the per-user path, seed the subconscious defaults and spawn the
    // heartbeat loop. Idempotent — no-op on subsequent logins of the same
    // process. Bootstrap failures are non-fatal: the session itself is
    // already stored above, so we only warn.
    if let Err(e) = crate::openhuman::subconscious::global::bootstrap_after_login().await {
        tracing::warn!(error = %e, "[subconscious] post-login bootstrap failed");
        logs.push(format!("subconscious bootstrap warning: {e}"));
    } else {
        logs.push("subconscious engine bootstrapped".to_string());
    }

    Ok(RpcOutcome::new(summarize_auth_profile(&profile), logs))
}

pub async fn clear_session(config: &Config) -> Result<RpcOutcome<serde_json::Value>, String> {
    let auth = AuthService::from_config(config);
    let removed = auth
        .remove_profile(APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME)
        .map_err(|e| e.to_string())?;

    // Clear the active user marker so subsequent config loads fall back to the
    // default (unauthenticated) openhuman directory.
    if let Ok(root_dir) = default_root_openhuman_dir() {
        if let Err(e) = crate::openhuman::config::clear_active_user(&root_dir) {
            tracing::warn!(error = %e, "failed to clear active_user.toml on logout");
        }
    }

    // Tear down the subconscious engine + heartbeat loop. Without this the
    // cached engine would keep pointing at the previous user's workspace_dir
    // and the heartbeat task would leak, ticking against the wrong DB when a
    // different user signs in to the same sidecar process.
    crate::openhuman::subconscious::global::reset_engine_for_user_switch().await;

    Ok(RpcOutcome::single_log(
        json!({ "removed": removed }),
        "session cleared",
    ))
}

pub async fn auth_get_state(
    config: &Config,
) -> Result<RpcOutcome<super::responses::AuthStateResponse>, String> {
    let state = build_session_state(config)?;
    Ok(RpcOutcome::single_log(state, "session state fetched"))
}

pub async fn auth_get_session_token_json(
    config: &Config,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let token = get_session_token(config)?;
    Ok(RpcOutcome::single_log(
        json!({ "token": token }),
        "session token fetched",
    ))
}

pub async fn auth_get_me(config: &Config) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let user = client
        .fetch_current_user(&token)
        .await
        .map_err(|e| e.to_string())?;

    Ok(RpcOutcome::single_log(user, "current user fetched"))
}

pub async fn consume_login_token(
    config: &Config,
    login_token: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let token = login_token.trim();
    if token.is_empty() {
        return Err("loginToken is required".to_string());
    }

    let api_url = effective_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let jwt_token = client
        .consume_login_token(token)
        .await
        .map_err(|e| e.to_string())?;

    Ok(RpcOutcome::new(
        serde_json::json!({ "jwtToken": jwt_token }),
        vec![
            format!(
                "login token consumed via POST /telegram/login-tokens/:token/consume on {}",
                api_url.trim_end_matches('/')
            ),
            "session JWT received".to_string(),
        ],
    ))
}

pub async fn auth_create_channel_link_token(
    config: &Config,
    channel: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let channel = channel.trim();
    if channel.is_empty() {
        return Err("channel is required".to_string());
    }
    let channel = channel.to_lowercase();
    if !matches!(channel.as_str(), "telegram" | "discord") {
        return Err(format!("unsupported channel: {channel}"));
    }

    let api_url = effective_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let payload = client
        .create_channel_link_token(&channel, &token)
        .await
        .map_err(|e| e.to_string())?;

    Ok(RpcOutcome::single_log(
        payload,
        "channel link token created",
    ))
}

pub async fn store_provider_credentials(
    config: &Config,
    provider: &str,
    profile: Option<&str>,
    token: Option<String>,
    fields: Option<serde_json::Value>,
    set_active: Option<bool>,
) -> Result<RpcOutcome<super::responses::AuthProfileSummary>, String> {
    let provider = provider.trim().to_string();
    if provider.is_empty() {
        return Err("provider is required".to_string());
    }

    let profile_name = profile_name_or_default(profile);
    let mut metadata = parse_fields_value(fields)?;
    let token = token
        .as_ref()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| metadata.get("token").cloned())
        .or_else(|| metadata.get("api_key").cloned())
        .unwrap_or_default();
    if token.is_empty() && metadata.is_empty() {
        return Err("provide at least one credential via token or fields".to_string());
    }
    metadata.remove("token");

    let auth = AuthService::from_config(config);
    let stored = auth
        .store_provider_token(
            &provider,
            profile_name,
            &token,
            metadata,
            set_active.unwrap_or(true),
        )
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        summarize_auth_profile(&stored),
        "provider credentials stored",
    ))
}

pub async fn remove_provider_credentials(
    config: &Config,
    provider: &str,
    profile: Option<&str>,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let profile_name = profile_name_or_default(profile);
    let auth = AuthService::from_config(config);
    let removed = auth
        .remove_profile(provider, profile_name)
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        json!({
            "removed": removed,
            "provider": provider,
            "profile": profile_name,
        }),
        "provider credentials removed",
    ))
}

pub async fn list_provider_credentials(
    config: &Config,
    provider_filter: Option<String>,
) -> Result<RpcOutcome<Vec<super::responses::AuthProfileSummary>>, String> {
    let auth = AuthService::from_config(config);
    let profiles = auth.load_profiles().map_err(|e| e.to_string())?;
    let mut items = profiles
        .profiles
        .values()
        .filter(|profile| profile.provider != APP_SESSION_PROVIDER)
        .filter(|profile| {
            provider_filter
                .as_ref()
                .is_none_or(|provider| profile.provider == *provider)
        })
        .map(summarize_auth_profile)
        .collect::<Vec<_>>();
    items.sort_by(|a, b| {
        a.provider
            .cmp(&b.provider)
            .then_with(|| a.profile_name.cmp(&b.profile_name))
    });

    Ok(RpcOutcome::single_log(items, "provider credentials listed"))
}

pub async fn oauth_connect(
    config: &Config,
    provider: &str,
    skill_id: Option<&str>,
    response_type: Option<&str>,
    encryption_mode: Option<&str>,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| {
        "session JWT required; complete login and store_session first".to_string()
    })?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let r = client
        .connect(provider, &token, skill_id, response_type, encryption_mode)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        serde_json::json!({ "oauthUrl": r.oauth_url, "state": r.state }),
        "oauth connect URL ready",
    ))
}

pub async fn oauth_list_integrations(
    config: &Config,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let list = client
        .list_integrations(&token)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        serde_json::to_value(&list).map_err(|e| e.to_string())?,
        "integrations listed",
    ))
}

pub async fn oauth_fetch_integration_tokens(
    config: &Config,
    integration_id: &str,
    encryption_key: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let tokens = client
        .fetch_integration_tokens_handoff(integration_id, &token, encryption_key)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        serde_json::to_value(&tokens).map_err(|e| e.to_string())?,
        "integration tokens retrieved",
    ))
}

pub async fn oauth_fetch_client_key(
    config: &Config,
    integration_id: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let client_key = client
        .fetch_client_key(integration_id, &token)
        .await
        .map_err(|e| e.to_string())?;
    log::debug!(
        "[credentials] client key retrieved for integration {}",
        integration_id
    );
    Ok(RpcOutcome::single_log(
        json!({ "clientKey": client_key, "integrationId": integration_id }),
        "client key retrieved (one-time handoff)",
    ))
}

pub async fn oauth_revoke_integration(
    config: &Config,
    integration_id: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let api_url = effective_api_url(&config.api_url);
    let token = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    client
        .revoke_integration(integration_id, &token)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        serde_json::json!({ "revoked": true, "integrationId": integration_id }),
        "integration revoked",
    ))
}

#[cfg(test)]
mod tests {
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
}
