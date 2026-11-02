use super::*;
use crate::session::test_support::{me_user, Backend, MeAnswer, ENV_LOCK, LIVE_JWT};
use serde_json::json;

fn headers() -> ClientHeaders {
    ClientHeaders::new("OpenCompany")
        .with_core_version("1.2.3+abc")
        .with_tauri_version("0.9.0\r\nX: y")
}

#[test]
fn client_headers_are_sanitised() {
    let h = headers();
    assert_eq!(h.sdk_name, "opencompany");
    assert_eq!(h.core_version.as_deref(), Some("1.2.3+abc"));
    assert_eq!(h.tauri_version.as_deref(), Some("0.9.0Xy"));
    assert_eq!(ClientHeaders::new("!!!").sdk_name, "openhuman");
}

#[test]
fn base_url_is_normalised_and_validated() {
    let c = SessionClient::new("https://api.example.com/v1/chat?x=1#f", &headers()).unwrap();
    assert_eq!(c.base_url(), "https://api.example.com");
    assert!(matches!(
        SessionClient::new("not a url", &headers()),
        Err(SessionClientError::InvalidBaseUrl(_))
    ));
    assert!(matches!(
        SessionClient::new("ftp://x", &headers()),
        Err(SessionClientError::InvalidBaseUrl(_))
    ));
    assert!(matches!(
        SessionClient::new("https://operator:secret@api.example.com", &headers()),
        Err(SessionClientError::InvalidBaseUrl(_))
    ));
}

#[test]
fn fetch_me_error_from_sdk_rejects_only_a_401() {
    let status = |status| tinyhumans_sdk::Error::Status {
        status,
        body: Value::Null,
    };
    assert!(matches!(
        FetchMeError::from_sdk(status(401)),
        FetchMeError::Rejected(_)
    ));
    for code in [400, 402, 403, 404, 429, 500, 503] {
        assert!(
            FetchMeError::from_sdk(status(code)).is_availability_failure(),
            "{code} must keep the session"
        );
    }
}

#[test]
fn fetch_me_error_classification() {
    assert!(FetchMeError::Transient("503".into()).is_availability_failure());
    assert!(FetchMeError::Transport("timeout".into()).is_availability_failure());
    assert!(!FetchMeError::Rejected("401".into()).is_availability_failure());
    assert!(!FetchMeError::Suppressed {
        message: "x".into(),
        consecutive: 2,
        retry_in: std::time::Duration::from_secs(1)
    }
    .is_availability_failure());
    assert!(contains_transient_transport_phrase("Operation timed out"));
    assert!(!contains_transient_transport_phrase("bad credentials"));
}

#[tokio::test]
async fn consume_login_token_returns_jwt_and_sends_attribution_headers() {
    let backend = Backend::start(vec![MeAnswer::Ok(me_user())]).await;
    let client = SessionClient::new(&backend.url, &headers()).unwrap();
    let jwt = client.consume_login_token(" tok ").await.unwrap();
    assert_eq!(jwt, *LIVE_JWT);
    assert_eq!(backend.consume_calls(), vec![json!({ "token": "tok" })]);

    let user = client
        .fetch_me(&Credential::session(LIVE_JWT.as_str()))
        .await
        .unwrap();
    assert_eq!(user["_id"], "user-123");
    let calls = backend.state.me_calls.lock().unwrap();
    let h = &calls[0];
    assert_eq!(h.get("x-sdk-name").unwrap(), "opencompany");
    assert_eq!(h.get("x-core-version").unwrap(), "1.2.3+abc");
    assert_eq!(
        h.get("authorization").unwrap().to_str().unwrap(),
        format!("Bearer {}", LIVE_JWT.as_str())
    );
    assert!(h.get("x-api-key").is_none());
}

#[tokio::test]
async fn consume_login_token_rejects_empty_and_reports_backend_failure() {
    let backend = Backend::start(vec![]).await;
    let client = SessionClient::new(&backend.url, &headers()).unwrap();
    assert!(matches!(
        client.consume_login_token("  ").await,
        Err(SessionClientError::EmptyLoginToken)
    ));
    assert!(matches!(
        client.consume_login_token("expired").await,
        Err(SessionClientError::ConsumeFailed(_))
    ));
    *backend.state.consume_jwt.lock().unwrap() = Some(String::new());
    assert!(matches!(
        client.consume_login_token("tok").await,
        Err(SessionClientError::MissingJwt)
    ));
}

#[tokio::test]
async fn fetch_me_sends_api_key_header_for_api_keys() {
    let backend = Backend::start(vec![]).await;
    let client = SessionClient::new(&backend.url, &headers()).unwrap();
    client.fetch_me(&Credential::api_key("sk-1")).await.unwrap();
    let calls = backend.state.me_calls.lock().unwrap();
    assert_eq!(calls[0].get("x-api-key").unwrap(), "sk-1");
    assert!(calls[0].get("authorization").is_none());
}

#[tokio::test]
async fn fetch_me_classifies_statuses() {
    let backend = Backend::start(vec![MeAnswer::Status(401), MeAnswer::Status(503)]).await;
    let client = SessionClient::new(&backend.url, &headers()).unwrap();
    let cred = Credential::session(LIVE_JWT.as_str());
    assert!(matches!(
        client.fetch_me(&cred).await,
        Err(FetchMeError::Rejected(_))
    ));
    assert!(matches!(
        client.fetch_me(&cred).await,
        Err(FetchMeError::Transient(_))
    ));
}

#[tokio::test]
async fn fetch_me_reports_unreachable_backend_as_transport() {
    let client = SessionClient::new("http://127.0.0.1:9", &headers()).unwrap();
    assert!(matches!(
        client
            .fetch_me(&Credential::session(LIVE_JWT.as_str()))
            .await,
        Err(FetchMeError::Transport(_))
    ));
}

#[tokio::test]
async fn validate_for_store_retries_once_after_transient() {
    let _env = ENV_LOCK.lock().await;
    let backend = Backend::start(vec![MeAnswer::Status(502), MeAnswer::Ok(me_user())]).await;
    let client = SessionClient::new(&backend.url, &headers()).unwrap();
    let user = client
        .validate_for_store(&Credential::session(LIVE_JWT.as_str()))
        .await
        .unwrap();
    assert_eq!(user["_id"], "user-123");
    assert_eq!(backend.me_calls(), 2);
}

#[tokio::test]
async fn validate_for_store_does_not_retry_a_rejection() {
    let _env = ENV_LOCK.lock().await;
    let backend = Backend::start(vec![MeAnswer::Status(401), MeAnswer::Ok(me_user())]).await;
    let client = SessionClient::new(&backend.url, &headers()).unwrap();
    assert!(matches!(
        client
            .validate_for_store(&Credential::session(LIVE_JWT.as_str()))
            .await,
        Err(FetchMeError::Rejected(_))
    ));
    assert_eq!(backend.me_calls(), 1);
}

#[tokio::test]
async fn validate_for_store_times_out_into_transport() {
    let _env = ENV_LOCK.lock().await;
    std::env::set_var(VALIDATION_BUDGET_ENV, "100");
    let backend = Backend::start(vec![MeAnswer::Slow(2_000)]).await;
    let client = SessionClient::new(&backend.url, &headers()).unwrap();
    let result = client
        .validate_for_store(&Credential::session(LIVE_JWT.as_str()))
        .await;
    std::env::remove_var(VALIDATION_BUDGET_ENV);
    match result {
        Err(FetchMeError::Transport(message)) => assert!(message.contains("timeout"), "{message}"),
        other => panic!("expected transport timeout, got {other:?}"),
    }
}

#[tokio::test]
async fn validation_budget_honours_new_then_legacy_env() {
    let _env = ENV_LOCK.lock().await;
    std::env::remove_var(VALIDATION_BUDGET_ENV);
    std::env::remove_var(LEGACY_VALIDATION_BUDGET_ENV);
    assert_eq!(validation_budget(), VALIDATION_BUDGET);
    std::env::set_var(LEGACY_VALIDATION_BUDGET_ENV, "250");
    assert_eq!(validation_budget().as_millis(), 250);
    std::env::set_var(VALIDATION_BUDGET_ENV, "300");
    assert_eq!(validation_budget().as_millis(), 300);
    std::env::set_var(VALIDATION_BUDGET_ENV, "0");
    assert_eq!(validation_budget().as_millis(), 250);
    std::env::remove_var(VALIDATION_BUDGET_ENV);
    std::env::remove_var(LEGACY_VALIDATION_BUDGET_ENV);
}

#[tokio::test]
async fn fetch_me_does_not_follow_a_cross_origin_redirect_with_the_api_key() {
    use wiremock::matchers::any;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let elsewhere = MockServer::start().await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(me_user()))
        .mount(&elsewhere)
        .await;
    let backend = MockServer::start().await;
    Mock::given(any())
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", format!("{}/auth/me", elsewhere.uri())),
        )
        .mount(&backend)
        .await;

    let client = SessionClient::new(&backend.uri(), &headers()).unwrap();
    let _ = client.fetch_me(&Credential::api_key("sk-1")).await;

    assert_eq!(backend.received_requests().await.unwrap().len(), 1);
    let leaked = elsewhere.received_requests().await.unwrap();
    assert!(
        leaked.iter().all(|r| !r.headers.contains_key("x-api-key")),
        "the redirect target received the API key"
    );
}
