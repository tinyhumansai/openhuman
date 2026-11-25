use super::source::OpenAiProviderCredentialSource;
use chrono::{Duration, Utc};
use openhuman_embed::__host::credentials::ProviderCredentialSource;
use tinyinference_providers::oauth::OAuthTokenSet;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test(flavor = "current_thread")]
async fn refresh_runs_from_a_current_thread_runtime() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "fresh-access",
            "refresh_token": "fresh-refresh",
            "expires_in": 3600,
        })))
        .expect(1)
        .mount(&server)
        .await;
    let source = OpenAiProviderCredentialSource {
        token_url_override: Some(format!("{}/token", server.uri())),
    };

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        source.resolve_openai_oauth(
            "provider:openai",
            "old-refresh",
            Some(Utc::now() + Duration::seconds(30)),
        )
    }));

    let tokens: OAuthTokenSet = result
        .expect("current-thread refresh must not panic")
        .expect("mock token endpoint succeeds")
        .expect("an expiring credential is refreshed");
    assert_eq!(tokens.access_token, "fresh-access");
    assert_eq!(tokens.refresh_token.as_deref(), Some("fresh-refresh"));
}

#[tokio::test(flavor = "current_thread")]
async fn expired_codex_token_keeps_reconnect_error_when_refresh_fails() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_string("refresh rejected"))
        .expect(1)
        .mount(&server)
        .await;
    let source = OpenAiProviderCredentialSource {
        token_url_override: Some(format!("{}/token", server.uri())),
    };

    let error = source
        .resolve_openai_oauth(
            "provider:openai",
            "expired-refresh",
            Some(Utc::now() - Duration::seconds(1)),
        )
        .expect_err("expired token cannot be used after a failed refresh");

    assert!(error.contains("authentication token is expired"));
}

#[test]
fn refresh_test_override_rejects_non_loopback_urls() {
    assert!(!super::source::is_loopback_http_endpoint(
        "https://example.test/token"
    ));
    assert!(!super::source::is_loopback_http_endpoint(
        "http://192.0.2.1/token"
    ));
}

#[test]
fn cli_import_preserves_account_metadata_and_refresh_material() {
    let imported = super::source::parse_codex_cli_auth(
        br#"{"tokens":{"access_token":"access","refresh_token":"refresh","id_token":"identity","account_id":"acct-42"}}"#,
    )
    .unwrap();

    assert_eq!(imported.token.access_token, "access");
    assert_eq!(imported.token.refresh_token.as_deref(), Some("refresh"));
    assert_eq!(imported.token.id_token.as_deref(), Some("identity"));
    assert_eq!(imported.account_id.as_deref(), Some("acct-42"));
}
