use super::*;

#[test]
fn lookup_key_for_slug_uses_legacy_openai_api_key_when_new_style_is_empty() {
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    let store = AuthProfilesStore::new(tmp.path(), false);
    let oauth_profile = AuthProfile::new_oauth(
        OPENAI_PROVIDER_KEY,
        OPENAI_OAUTH_PROFILE_NAME,
        TokenSet {
            access_token: "   ".into(),
            refresh_token: None,
            id_token: None,
            expires_at: Some(Utc::now() + Duration::hours(1)),
            token_type: Some("Bearer".into()),
            scope: None,
        },
    );
    store.upsert_profile(oauth_profile, true).unwrap();
    store
        .upsert_profile(
            AuthProfile::new_token("openai", "default", "sk-legacy-key".to_string()),
            true,
        )
        .unwrap();

    // Legacy bare-slug key resolves through the standard path's legacy
    // fallback, ahead of the OAuth fallback.
    let token = lookup_key_for_slug("openai", &config).unwrap();
    assert_eq!(token, "sk-legacy-key");
}

#[test]
fn lookup_openai_bearer_token_returns_session_expired_error_when_refresh_fails_on_expired_token() {
    // A token that is 5 minutes past expiry with a refresh token present but no
    // tokio runtime available (so try_refresh_oauth_token returns Err).
    // After the fix in store.rs: is_expiring_within(ZERO) returns true AND
    // refresh returned Err, so the function must return Err with a
    // "session expired" message rather than Ok(Some("expired-access")).
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    let store = AuthProfilesStore::new(tmp.path(), false);
    let oauth_profile = AuthProfile::new_oauth(
        OPENAI_PROVIDER_KEY,
        OPENAI_OAUTH_PROFILE_NAME,
        TokenSet {
            access_token: "expired-access".into(),
            refresh_token: Some("refresh".into()),
            id_token: None,
            expires_at: Some(Utc::now() - Duration::minutes(5)),
            token_type: Some("Bearer".into()),
            scope: None,
        },
    );
    store.upsert_profile(oauth_profile, true).unwrap();

    let result = lookup_openai_bearer_token(&config);
    let err = result.expect_err("expected Err for expired token with failed refresh");
    assert!(
        err.to_lowercase()
            .contains("authentication token is expired"),
        "error message should contain 'authentication token is expired' to trigger \
         is_openai_oauth_session_expired_message (not the app-session classifier), got: {err:?}"
    );
}

#[test]
fn lookup_openai_bearer_token_returns_ok_when_nearly_expiring_and_refresh_fails() {
    // A token expiring 30 seconds in the future — within the 2-minute warning
    // window but NOT yet expired.  The refresh fails (no tokio runtime), but
    // since is_expiring_within(ZERO) is false the function must still return
    // Ok(Some(...)) so the caller can make one last inference call before the
    // token truly expires.
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    let store = AuthProfilesStore::new(tmp.path(), false);
    let oauth_profile = AuthProfile::new_oauth(
        OPENAI_PROVIDER_KEY,
        OPENAI_OAUTH_PROFILE_NAME,
        TokenSet {
            access_token: "nearly-expiring-access".into(),
            refresh_token: Some("refresh".into()),
            id_token: None,
            expires_at: Some(Utc::now() + Duration::seconds(30)),
            token_type: Some("Bearer".into()),
            scope: None,
        },
    );
    store.upsert_profile(oauth_profile, true).unwrap();

    let token = lookup_openai_bearer_token(&config)
        .expect("nearly-expiring token should be returned even when refresh fails");
    assert_eq!(
        token.as_deref(),
        Some("nearly-expiring-access"),
        "should return the existing token for one last inference call"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn lookup_openai_bearer_token_does_not_persist_blank_refreshed_access_token() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    let store = AuthProfilesStore::new(tmp.path(), false);
    let original_profile = AuthProfile::new_oauth(
        OPENAI_PROVIDER_KEY,
        OPENAI_OAUTH_PROFILE_NAME,
        TokenSet {
            access_token: "oauth-access".into(),
            refresh_token: Some("refresh-token".into()),
            id_token: None,
            // Token is nearly-expiring (within 2-min skew) but NOT past expiry.
            // This ensures refresh is attempted while the session-expired path
            // (which fires only when expires_at <= now()) stays silent on failure.
            expires_at: Some(Utc::now() + Duration::seconds(90)),
            token_type: Some("Bearer".into()),
            scope: None,
        },
    );
    store.upsert_profile(original_profile, true).unwrap();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "   ",
            "refresh_token": "refresh-updated",
            "id_token": "id-updated",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;

    let _env_guard = EnvVarGuard::set(
        "OPENAI_CODEX_OAUTH_TOKEN_URL",
        format!("{}/token", server.uri()),
    );

    let token = lookup_openai_bearer_token(&config).unwrap();
    assert_eq!(
        token.as_deref(),
        Some("oauth-access"),
        "a failed refresh on a not-yet-expired token should return the cached token, not error"
    );

    let reloaded = AuthProfilesStore::new(tmp.path(), false).load().unwrap();
    let stored = reloaded
        .profiles
        .get(&format!(
            "{OPENAI_PROVIDER_KEY}:{OPENAI_OAUTH_PROFILE_NAME}"
        ))
        .expect("oauth profile should still exist after invalid refresh response");
    let token_set = stored.token_set.as_ref().expect("oauth token_set");
    assert_eq!(
        token_set.access_token, "oauth-access",
        "invalid refresh payload should not be persisted"
    );
}

#[test]
fn lookup_openai_bearer_token_returns_none_without_profiles_or_access_token() {
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    assert_eq!(lookup_openai_bearer_token(&config).unwrap(), None);

    let store = AuthProfilesStore::new(tmp.path(), false);
    let empty_oauth_profile = AuthProfile::new_oauth(
        OPENAI_PROVIDER_KEY,
        OPENAI_OAUTH_PROFILE_NAME,
        TokenSet {
            access_token: "   ".into(),
            refresh_token: None,
            id_token: None,
            expires_at: Some(Utc::now() - Duration::hours(1)),
            token_type: Some("Bearer".into()),
            scope: None,
        },
    );
    store.upsert_profile(empty_oauth_profile, true).unwrap();

    assert_eq!(lookup_openai_bearer_token(&config).unwrap(), None);
}

#[test]
fn disconnect_openai_oauth_clears_profile() {
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    let store = AuthProfilesStore::new(tmp.path(), false);
    let profile = AuthProfile::new_oauth(
        OPENAI_PROVIDER_KEY,
        OPENAI_OAUTH_PROFILE_NAME,
        TokenSet {
            access_token: "oauth-access".into(),
            refresh_token: None,
            id_token: None,
            expires_at: None,
            token_type: Some("Bearer".into()),
            scope: None,
        },
    );
    store.upsert_profile(profile, true).unwrap();
    assert!(openai_oauth_status(&config).unwrap().connected);

    disconnect_openai_oauth(&config).unwrap();
    assert!(!openai_oauth_status(&config).unwrap().connected);
}
