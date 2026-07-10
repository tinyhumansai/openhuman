use super::flow::{build_authorize_url, exchange_authorization_code, parse_callback_input};
use super::store::persist_openai_oauth_token;
use super::{
    complete_openai_oauth, disconnect_openai_oauth, openai_oauth_status, start_openai_oauth,
};
use crate::openhuman::config::Config;
use crate::openhuman::credentials::profiles::{
    AuthProfile, AuthProfileKind, AuthProfilesStore, TokenSet,
};
use crate::openhuman::inference::openai_oauth::store::{
    import_codex_cli_auth_from_path, OPENAI_OAUTH_PROFILE_NAME, OPENAI_PROVIDER_KEY,
};
use crate::openhuman::inference::openai_oauth::{
    lookup_openai_bearer_token, lookup_openai_oauth_credentials,
};
use crate::openhuman::inference::provider::factory::lookup_key_for_slug;
use chrono::{Duration, Utc};
use motosan_ai_oauth::{OAuthConfig, StateStrategy, TokenBodyFormat};
use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            match self.previous.take() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

fn test_config(tmp: &tempfile::TempDir) -> Config {
    Config {
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().unwrap()
}

fn unsigned_jwt(payload: serde_json::Value) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
    let payload = URL_SAFE_NO_PAD.encode(payload.to_string());
    format!("{header}.{payload}.")
}

fn test_oauth_config(token_url: &'static str) -> OAuthConfig {
    OAuthConfig {
        client_id: "client-id",
        client_secret: Some("client-secret"),
        auth_url: "https://auth.example.test/oauth/authorize",
        token_url,
        scopes: &["scope-a", "scope-b"],
        redirect_port: Some(1455),
        callback_path: "/auth/callback",
        redirect_uri_host: "127.0.0.1",
        token_body: TokenBodyFormat::Form,
        extra_auth_params: &[("prompt", "consent")],
        state_strategy: StateStrategy::Random,
    }
}

#[test]
fn start_openai_oauth_returns_authorize_url() {
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);

    let start = start_openai_oauth(&config).unwrap();
    assert!(start.auth_url.contains("auth.openai.com"));
    assert!(start.auth_url.contains("code_challenge="));
    assert_eq!(start.redirect_uri, "http://127.0.0.1:1455/auth/callback");
    assert!(!start.state.is_empty());
    assert!(!openai_oauth_status(&config).unwrap().connected);
}

#[test]
fn build_authorize_url_includes_codex_pkce_and_extra_params() {
    let url = build_authorize_url(
        &test_oauth_config("https://token.example.test/oauth/token"),
        "challenge-123",
        "state-123",
        "http://127.0.0.1:1455/auth/callback",
    );
    let parsed = reqwest::Url::parse(&url).unwrap();
    let pairs = parsed
        .query_pairs()
        .into_owned()
        .collect::<std::collections::BTreeMap<_, _>>();

    assert_eq!(
        pairs.get("client_id").map(String::as_str),
        Some("client-id")
    );
    assert_eq!(pairs.get("response_type").map(String::as_str), Some("code"));
    assert_eq!(
        pairs.get("scope").map(String::as_str),
        Some("scope-a scope-b")
    );
    assert_eq!(pairs.get("state").map(String::as_str), Some("state-123"));
    assert_eq!(
        pairs.get("code_challenge").map(String::as_str),
        Some("challenge-123")
    );
    assert_eq!(
        pairs.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert_eq!(pairs.get("prompt").map(String::as_str), Some("consent"));
}

#[test]
fn parse_callback_input_accepts_full_redirect_url() {
    let url = "http://127.0.0.1:1455/auth/callback?code=abc&state=xyz";
    let (code, state) = parse_callback_input(url).unwrap();
    assert_eq!(code, "abc");
    assert_eq!(state, "xyz");
}

#[test]
fn parse_callback_input_accepts_raw_query_string() {
    let (code, state) = parse_callback_input("code=abc%20123&state=xyz").unwrap();
    assert_eq!(code, "abc 123");
    assert_eq!(state, "xyz");
}

#[test]
fn parse_callback_input_rejects_missing_code() {
    let err = parse_callback_input("http://127.0.0.1:1455/auth/callback?state=xyz").unwrap_err();
    assert!(err.contains("code"));
}

#[test]
fn parse_callback_input_rejects_blank_invalid_and_missing_state() {
    let blank = parse_callback_input("   ").unwrap_err();
    assert!(blank.contains("required"));

    let invalid = parse_callback_input("not-a-callback").unwrap_err();
    assert!(invalid.contains("invalid"));

    let missing_state =
        parse_callback_input("http://127.0.0.1:1455/auth/callback?code=abc").unwrap_err();
    assert!(missing_state.contains("state"));
}

#[test]
fn complete_openai_oauth_rejects_missing_pending_session() {
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    let err = runtime()
        .block_on(complete_openai_oauth(
            &config,
            "http://127.0.0.1:1455/auth/callback?code=fake&state=state",
        ))
        .unwrap_err();
    assert!(err.contains("no pending OAuth session"));
}

#[test]
fn complete_openai_oauth_rejects_expired_pending_session() {
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    std::fs::write(
        tmp.path().join("openai-oauth-pending.json"),
        serde_json::json!({
            "state": "state",
            "verifier": "verifier",
            "redirect_uri": "http://127.0.0.1:1455/auth/callback",
            "created_at": 1_u64,
        })
        .to_string(),
    )
    .unwrap();

    let err = runtime()
        .block_on(complete_openai_oauth(
            &config,
            "http://127.0.0.1:1455/auth/callback?code=fake&state=state",
        ))
        .unwrap_err();
    assert!(err.contains("no pending OAuth session"));
    assert!(!tmp.path().join("openai-oauth-pending.json").exists());
}

#[test]
fn complete_openai_oauth_rejects_state_mismatch() {
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    let start = start_openai_oauth(&config).unwrap();
    let callback = format!(
        "http://127.0.0.1:1455/auth/callback?code=fake&state=not-{}",
        start.state
    );
    let err = runtime()
        .block_on(complete_openai_oauth(&config, &callback))
        .unwrap_err();
    assert!(err.contains("state mismatch"));
}

#[tokio::test]
async fn exchange_authorization_code_parses_successful_token_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "access-token",
            "refresh_token": "refresh-token",
            "id_token": "id-token",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;
    let token_url: &'static str = Box::leak(format!("{}/token", server.uri()).into_boxed_str());

    let token = exchange_authorization_code(
        &test_oauth_config(token_url),
        "code-123",
        "verifier-123",
        "http://127.0.0.1:1455/auth/callback",
    )
    .await
    .unwrap();

    assert_eq!(token.access_token, "access-token");
    assert_eq!(token.refresh_token, "refresh-token");
    assert_eq!(token.id_token.as_deref(), Some("id-token"));
    assert_eq!(token.expires_in, 3600);
    assert!(token.issued_at > 0);
}

#[tokio::test]
async fn exchange_authorization_code_reports_http_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad auth code"))
        .mount(&server)
        .await;
    let token_url: &'static str = Box::leak(format!("{}/token", server.uri()).into_boxed_str());

    let err = exchange_authorization_code(
        &test_oauth_config(token_url),
        "code-123",
        "verifier-123",
        "http://127.0.0.1:1455/auth/callback",
    )
    .await
    .unwrap_err();

    assert!(err.contains("HTTP 400"));
    assert!(err.contains("bad auth code"));
}

#[test]
fn persist_openai_oauth_token_stores_oauth_profile_with_metadata() {
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    let access_token = unsigned_jwt(serde_json::json!({ "sub": "acct_123" }));
    let token = motosan_ai_oauth::Token {
        access_token: access_token.clone(),
        refresh_token: "refresh-token".into(),
        id_token: Some("id-token".into()),
        expires_in: 3600,
        issued_at: 123,
    };

    let profile = persist_openai_oauth_token(&config, &token).unwrap();
    assert_eq!(profile.kind, AuthProfileKind::OAuth);
    assert_eq!(
        profile.metadata.get("account_id").map(String::as_str),
        Some("acct_123")
    );
    assert_eq!(
        profile
            .token_set
            .as_ref()
            .map(|set| set.access_token.as_str()),
        Some(access_token.as_str())
    );
    assert_eq!(
        profile
            .token_set
            .as_ref()
            .and_then(|set| set.refresh_token.as_deref()),
        Some("refresh-token")
    );
    assert!(profile
        .token_set
        .as_ref()
        .and_then(|set| set.expires_at)
        .is_some());

    let data = AuthProfilesStore::new(tmp.path(), false).load().unwrap();
    let stored = data.profiles.get(&profile.id).unwrap();
    assert_eq!(stored.id, profile.id);
}

#[test]
fn persist_openai_oauth_token_rejects_blank_access_token() {
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    let token = motosan_ai_oauth::Token {
        access_token: "   ".into(),
        refresh_token: "refresh-token".into(),
        id_token: Some("id-token".into()),
        expires_in: 3600,
        issued_at: 123,
    };

    let err = persist_openai_oauth_token(&config, &token).unwrap_err();
    assert!(
        err.contains("access_token"),
        "expected missing access_token error, got: {err}"
    );

    let data = AuthProfilesStore::new(tmp.path(), false).load().unwrap();
    assert!(
        data.profiles.is_empty(),
        "blank-access OAuth token should not be persisted"
    );
}

#[test]
fn import_codex_cli_auth_file_stores_oauth_profile_with_account_metadata() {
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    let access_token = unsigned_jwt(serde_json::json!({
        "sub": "acct_from_jwt",
        "exp": (Utc::now() + Duration::hours(1)).timestamp(),
    }));
    let auth_path = tmp.path().join("codex-auth.json");
    std::fs::write(
        &auth_path,
        serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": access_token.clone(),
                "refresh_token": "codex-refresh",
                "id_token": "codex-id",
                "account_id": "acct_from_file",
            }
        })
        .to_string(),
    )
    .unwrap();

    let profile = import_codex_cli_auth_from_path(&config, &auth_path).unwrap();

    assert_eq!(profile.kind, AuthProfileKind::OAuth);
    assert_eq!(
        profile.metadata.get("account_id").map(String::as_str),
        Some("acct_from_file")
    );
    let token_set = profile.token_set.as_ref().unwrap();
    assert_eq!(token_set.access_token, access_token);
    assert_eq!(token_set.refresh_token.as_deref(), Some("codex-refresh"));
    assert_eq!(token_set.id_token.as_deref(), Some("codex-id"));
    assert!(token_set.expires_at.is_some());

    let credentials = lookup_openai_oauth_credentials(&config)
        .unwrap()
        .expect("imported oauth credentials");
    assert_eq!(credentials.access_token, access_token);
    assert_eq!(credentials.account_id.as_deref(), Some("acct_from_file"));
    assert_eq!(
        lookup_key_for_slug("openai", &config).unwrap(),
        access_token
    );
}

#[test]
fn import_codex_cli_auth_extracts_nested_chatgpt_account_id() {
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    let access_token = unsigned_jwt(serde_json::json!({
        "https://api.openai.com/auth": {
            "chatgpt_account_id": "acct_nested"
        },
        "sub": "acct_subject",
        "exp": (Utc::now() + Duration::hours(1)).timestamp(),
    }));
    let auth_path = tmp.path().join("codex-auth.json");
    std::fs::write(
        &auth_path,
        serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": access_token,
                "refresh_token": "codex-refresh"
            }
        })
        .to_string(),
    )
    .unwrap();

    let profile = import_codex_cli_auth_from_path(&config, &auth_path).unwrap();

    assert_eq!(
        profile.metadata.get("account_id").map(String::as_str),
        Some("acct_nested")
    );
    let credentials = lookup_openai_oauth_credentials(&config)
        .unwrap()
        .expect("imported oauth credentials");
    assert_eq!(credentials.account_id.as_deref(), Some("acct_nested"));
}

#[test]
fn import_codex_cli_auth_decodes_padded_base64url_access_token() {
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    let payload = "eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjdF9wYWRkZWQifSwibm9uY2UiOiI-In0=";
    assert!(
        (payload.contains('-') || payload.contains('_')) && payload.ends_with('='),
        "test fixture must exercise padded base64url input"
    );
    let access_token = format!("e30.{payload}.");
    let auth_path = tmp.path().join("codex-auth.json");
    std::fs::write(
        &auth_path,
        serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": access_token,
                "refresh_token": "codex-refresh"
            }
        })
        .to_string(),
    )
    .unwrap();

    let profile = import_codex_cli_auth_from_path(&config, &auth_path).unwrap();

    assert_eq!(
        profile.metadata.get("account_id").map(String::as_str),
        Some("acct_padded")
    );
}

#[test]
fn import_codex_cli_auth_file_reports_missing_file_with_login_hint() {
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    let err = import_codex_cli_auth_from_path(&config, &tmp.path().join("missing-auth.json"))
        .unwrap_err();

    assert!(err.contains("Could not read Codex CLI auth"));
    assert!(err.contains("codex login"));
}

/// Drift-proof coupling: every user-state error the real Codex-CLI import
/// producer emits MUST classify as `CodexCliAuthUnavailable`, so the Sentry
/// demotion at `ops.rs` (TAURI-RUST-83A) keeps working even if the wording
/// changes. If a future edit to `store.rs` drops the `codex cli auth` /
/// `.codex/auth.json` anchor from a message, this test fails in CI.
#[test]
fn codex_import_user_state_errors_classify_as_expected() {
    use crate::core::observability::{expected_error_kind, ExpectedErrorKind};

    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);

    // Missing file (no `codex login`).
    let missing = import_codex_cli_auth_from_path(&config, &tmp.path().join("missing-auth.json"))
        .unwrap_err();

    // Unparseable file.
    let garbage_path = tmp.path().join("garbage-auth.json");
    std::fs::write(&garbage_path, b"not json").unwrap();
    let garbage = import_codex_cli_auth_from_path(&config, &garbage_path).unwrap_err();

    // Parses but carries no tokens.
    let no_tokens_path = tmp.path().join("no-tokens-auth.json");
    std::fs::write(&no_tokens_path, b"{}").unwrap();
    let no_tokens = import_codex_cli_auth_from_path(&config, &no_tokens_path).unwrap_err();

    // Parses with a tokens object but no access token.
    let no_access_path = tmp.path().join("no-access-auth.json");
    std::fs::write(&no_access_path, br#"{"tokens":{"refresh_token":"r"}}"#).unwrap();
    let no_access = import_codex_cli_auth_from_path(&config, &no_access_path).unwrap_err();

    for err in [&missing, &garbage, &no_tokens, &no_access] {
        assert_eq!(
            expected_error_kind(err),
            Some(ExpectedErrorKind::CodexCliAuthUnavailable),
            "codex import user-state error must classify as CodexCliAuthUnavailable: {err}"
        );
    }
}

/// Exercise the ops entry point (`inference_openai_oauth_import_codex_cli`) on
/// the failure path so the `report_error_or_expected` call at the match arm is
/// covered: point `CODEX_HOME` at an empty dir (no `auth.json`) and assert the
/// RPC surfaces the actionable error.
#[tokio::test]
async fn inference_import_codex_cli_surfaces_error_when_auth_missing() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    let _env_guard = EnvVarGuard::set("CODEX_HOME", tmp.path());

    let err = crate::openhuman::inference::ops::inference_openai_oauth_import_codex_cli(&config)
        .await
        .unwrap_err();

    assert!(err.contains("Could not read Codex CLI auth"));
    assert!(err.contains("codex login"));
}

#[test]
fn openai_oauth_status_reports_token_profile_as_disconnected() {
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    let store = AuthProfilesStore::new(tmp.path(), false);
    store
        .upsert_profile(
            AuthProfile::new_token(
                OPENAI_PROVIDER_KEY,
                OPENAI_OAUTH_PROFILE_NAME,
                "sk-token-profile".to_string(),
            ),
            true,
        )
        .unwrap();

    let status = openai_oauth_status(&config).unwrap();
    assert!(!status.connected);
    assert_eq!(status.auth_method.as_deref(), Some("token"));
    assert!(status.profile_id.is_some());
}

#[test]
fn lookup_key_for_slug_prefers_api_key_over_oauth_for_openai() {
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    let store = AuthProfilesStore::new(tmp.path(), false);

    let oauth_profile = AuthProfile::new_oauth(
        OPENAI_PROVIDER_KEY,
        OPENAI_OAUTH_PROFILE_NAME,
        TokenSet {
            access_token: "oauth-access".into(),
            refresh_token: Some("refresh".into()),
            id_token: None,
            expires_at: Some(Utc::now() + Duration::hours(1)),
            token_type: Some("Bearer".into()),
            scope: None,
        },
    );
    store.upsert_profile(oauth_profile, true).unwrap();

    let api_profile =
        AuthProfile::new_token("provider:openai", "default", "sk-api-key".to_string());
    store.upsert_profile(api_profile, true).unwrap();

    // The standard `lookup_key_for_slug` path resolves the API key first; the
    // OAuth fallback only fires when no API key is present.
    let token = lookup_key_for_slug("openai", &config).unwrap();
    assert_eq!(token, "sk-api-key");
}

#[test]
fn lookup_openai_bearer_token_uses_oauth_when_api_key_missing() {
    let tmp = tempdir().unwrap();
    let config = test_config(&tmp);
    let store = AuthProfilesStore::new(tmp.path(), false);
    let oauth_profile = AuthProfile::new_oauth(
        OPENAI_PROVIDER_KEY,
        OPENAI_OAUTH_PROFILE_NAME,
        TokenSet {
            access_token: "oauth-access".into(),
            refresh_token: Some("refresh".into()),
            id_token: None,
            expires_at: Some(Utc::now() + Duration::hours(1)),
            token_type: Some("Bearer".into()),
            scope: None,
        },
    );
    store.upsert_profile(oauth_profile, true).unwrap();

    let token = lookup_openai_bearer_token(&config).unwrap();
    assert_eq!(token.as_deref(), Some("oauth-access"));
}

#[test]
fn credits_gate_bypassed_with_oauth_only_credentials() {
    // #3767 regression: the per-tier credits-gate bypass chains through
    // route_has_usable_credentials → lookup_key_for_slug, which falls back to
    // the OpenAI OAuth token for the `openai` slug. Pin that OAuth-only
    // credentials (no new-style provider key) bypass the gate when the chat tier
    // is routed to a concrete OpenAI model.
    use crate::openhuman::inference::provider::factory::role_bypasses_managed_credits;

    let tmp = tempdir().unwrap();
    let mut config = test_config(&tmp);
    config.chat_provider = Some("openai:gpt-4o".into());

    // No credential yet → gate stays on.
    assert!(!role_bypasses_managed_credits("chat", &config));

    let store = AuthProfilesStore::new(tmp.path(), false);
    let oauth_profile = AuthProfile::new_oauth(
        OPENAI_PROVIDER_KEY,
        OPENAI_OAUTH_PROFILE_NAME,
        TokenSet {
            access_token: "oauth-access".into(),
            refresh_token: Some("refresh".into()),
            id_token: None,
            expires_at: Some(Utc::now() + Duration::hours(1)),
            token_type: Some("Bearer".into()),
            scope: None,
        },
    );
    store.upsert_profile(oauth_profile, true).unwrap();

    // OAuth-only credential now backs the route → gate bypassed.
    assert!(role_bypasses_managed_credits("chat", &config));
}

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
fn lookup_openai_bearer_token_keeps_expired_token_when_refresh_fails_without_runtime() {
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

    let token = lookup_openai_bearer_token(&config).unwrap();
    assert_eq!(token.as_deref(), Some("expired-access"));
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
            expires_at: Some(Utc::now() - Duration::minutes(5)),
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
        "invalid refresh payload should not replace the last known good access token"
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
