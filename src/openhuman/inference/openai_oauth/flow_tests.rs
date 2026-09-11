use super::flow::{build_authorize_url, exchange_authorization_code, parse_callback_input};
use super::store::persist_openai_oauth_token;
use super::{
    complete_openai_oauth, disconnect_openai_oauth, openai_oauth_status, start_openai_oauth,
};
use crate::openhuman::config::Config;
use crate::openhuman::inference::openai_oauth::store::{
    import_codex_cli_auth_from_path, OPENAI_OAUTH_PROFILE_NAME, OPENAI_PROVIDER_KEY,
};
use crate::openhuman::inference::openai_oauth::{
    lookup_openai_bearer_token, lookup_openai_oauth_credentials,
};
use crate::openhuman::inference::provider::factory::lookup_key_for_slug;
use crate::openhuman::security::credentials::profiles::{
    AuthProfile, AuthProfileKind, AuthProfilesStore, TokenSet,
};
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

#[path = "flow_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "flow_tests_part_02_tests.rs"]
mod part_02_tests;
