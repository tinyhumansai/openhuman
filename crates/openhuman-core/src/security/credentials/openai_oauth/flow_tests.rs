use super::pending;
use super::store::persist_openai_oauth_token;
use super::{
    complete_openai_oauth, disconnect_openai_oauth, openai_oauth_status, start_openai_oauth,
};
use crate::config::test_env::EnvVarGuard;
use crate::config::Config;
use crate::inference::provider::factory::lookup_key_for_slug;
use crate::security::credentials::openai_oauth::store::{
    import_codex_cli_auth_from_path, OPENAI_OAUTH_PROFILE_NAME, OPENAI_PROVIDER_KEY,
};
use crate::security::credentials::openai_oauth::{
    lookup_openai_bearer_token, lookup_openai_oauth_credentials,
};
use crate::security::credentials::profiles::{
    AuthProfile, AuthProfileKind, AuthProfilesStore, TokenSet,
};
use chrono::{Duration, Utc};
use tempfile::tempdir;
use tinyinference_providers::oauth::{
    authorization_url as build_authorize_url, exchange_authorization_code, parse_callback_input,
    OAuthConfig, OAuthTokenSet,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn tiny_oauth_config(config: &OAuthConfig, redirect_uri: &str) -> OAuthConfig {
    let mut config = config.clone();
    config.redirect_uri = redirect_uri.to_string();
    config
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

fn test_oauth_config(token_url: &str) -> OAuthConfig {
    OAuthConfig {
        client_id: "client-id".to_string(),
        client_secret: Some("client-secret".to_string()),
        authorize_url: "https://auth.example.test/oauth/authorize".to_string(),
        token_url: token_url.to_string(),
        redirect_uri: "http://127.0.0.1:1455/auth/callback".to_string(),
        scopes: vec!["scope-a".to_string(), "scope-b".to_string()],
        extra_authorize_params: vec![("prompt".to_string(), "consent".to_string())],
        state_equals_verifier: false,
        pending_secret: "openai-oauth-pending".to_string(),
    }
}

#[path = "flow_tests_bearer_token_tests.rs"]
mod bearer_token_tests;
#[path = "flow_tests_lifecycle_tests.rs"]
mod lifecycle_tests;
