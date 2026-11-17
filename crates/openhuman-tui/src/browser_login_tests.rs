use super::*;
use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use openhuman_rpc::tinyhumans::{link::CoreAuthState, ClientHeaders};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

fn jwt() -> String {
    format!(
        "{}.{}.sig",
        URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#),
        URL_SAFE_NO_PAD.encode(json!({"sub":"test-user","exp":4102444800_u64}).to_string())
    )
}

struct FakeCore {
    base: Mutex<String>,
    token: Mutex<Option<String>>,
    state: Mutex<CoreAuthState>,
}

#[async_trait]
impl CoreLink for FakeCore {
    async fn invoke(&self, method: &str, params: Value) -> Result<Value, String> {
        let mut state = self.state.lock().unwrap();
        match method {
            "openhuman.config_resolve_api_url" => {
                Ok(json!({"api_url":self.base.lock().unwrap().clone()}))
            }
            "openhuman.auth_get_state" => Ok(serde_json::to_value(&*state).unwrap()),
            "openhuman.auth_get_session_token" => {
                Ok(json!({"token":self.token.lock().unwrap().clone()}))
            }
            "openhuman.auth_set_credential" => {
                *self.token.lock().unwrap() = params["token"].as_str().map(str::to_owned);
                *state = CoreAuthState {
                    is_authenticated: true,
                    credential: Some("session".into()),
                    user_id: params["userId"].as_str().map(str::to_owned),
                    user: Some(params["user"].clone()),
                    issuing_backend: params["issuingBackend"].as_str().map(str::to_owned),
                    ..Default::default()
                };
                Ok(serde_json::to_value(&*state).unwrap())
            }
            "openhuman.auth_clear_credential" => {
                *state = CoreAuthState::default();
                *self.token.lock().unwrap() = None;
                Ok(json!({}))
            }
            _ => Err("Unsupported test method".into()),
        }
    }
}

struct Backend {
    task: tokio::task::JoinHandle<()>,
    base: String,
    exchanges: Arc<AtomicUsize>,
    profiles: Arc<AtomicUsize>,
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Backend {
    async fn start(reject: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let exchanges = Arc::new(AtomicUsize::new(0));
        let profiles = Arc::new(AtomicUsize::new(0));
        let calls = exchanges.clone();
        let me_calls = profiles.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let head = read_head(&mut stream).await.unwrap();
                let consume = head.starts_with("POST /auth/login-token/consume ");
                if consume {
                    calls.fetch_add(1, Ordering::SeqCst);
                } else {
                    assert!(head.starts_with("GET /auth/me "));
                    me_calls.fetch_add(1, Ordering::SeqCst);
                }
                let body = if consume {
                    json!({"success":true,"data":{"jwt":jwt()}})
                } else if reject {
                    json!({"success":false,"message":"rejected"})
                } else {
                    json!({"success":true,"data":{"_id":"test-user","name":"Test account"}})
                }
                .to_string();
                let status = if reject && !consume {
                    "401 Unauthorized"
                } else {
                    "200 OK"
                };
                let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        Self {
            task,
            base,
            exchanges,
            profiles,
        }
    }

    fn manager(&self) -> Arc<SessionManager<FakeCore>> {
        SessionManager::new(
            Arc::new(FakeCore {
                base: Mutex::new(self.base.clone()),
                token: Mutex::new(None),
                state: Mutex::new(CoreAuthState::default()),
            }),
            ClientHeaders::new("openhuman-test"),
        )
    }
}

fn request(port: u16, target: &str) -> String {
    format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")
}

async fn callback(port: u16, target: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream
        .write_all(request(port, target).as_bytes())
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    response
}

#[test]
fn callback_rejects_wrong_nonce_method_path_origin_host_and_duplicate_params() {
    let valid = request(1234, "/auth?state=expected&token=a%2Eb%2Bc");
    assert!(
        matches!(classify(&valid, "expected", 1234), Callback::Token(token) if token.as_str()=="a.b+c")
    );
    for invalid in [
        valid.replace("expected", "wrong"),
        valid.replace("GET", "POST"),
        valid.replace("/auth?", "/else?"),
        valid.replace("Host:", "Origin: https://example.invalid\r\nHost:"),
        valid.replace("127.0.0.1", "example.invalid"),
        request(1234, "/auth?state=expected&state=wrong&token=test"),
        request(1234, "/auth?state=expected&token="),
        request(1234, "/auth?state=expected&token=%ZZ"),
    ] {
        assert!(matches!(
            classify(&invalid, "expected", 1234),
            Callback::Ignore(_)
        ));
    }
    assert!(matches!(
        classify(
            &request(1234, "/auth?state=expected&error=secret"),
            "expected",
            1234
        ),
        Callback::Rejected
    ));
}

#[test]
fn token_classification_and_url_encoding_match_backend_contract() {
    assert!(is_login_token(&"a".repeat(64)));
    for token in [jwt(), "A".repeat(64), "a".repeat(63), "g".repeat(64)] {
        assert!(!is_login_token(&token));
    }
    assert_eq!(
        percent_encode("http://127.0.0.1:1234/auth?state=abc"),
        "http%3A%2F%2F127.0.0.1%3A1234%2Fauth%3Fstate%3Dabc"
    );
}

#[tokio::test]
async fn browser_login_exchanges_one_time_token_and_stores_verified_account() {
    let backend = Backend::start(false).await;
    let manager = backend.manager();
    let flow = BrowserLogin::start(manager.clone(), LoginProvider::Github)
        .await
        .unwrap();
    let port = flow.listener.local_addr().unwrap().port();
    assert_eq!(flow.callback_port(), port);
    assert_eq!(
        flow.login_url(),
        format!(
            "{}/auth/github/login?redirect=app&redirectUri={}",
            backend.base,
            percent_encode(&format!(
                "http://127.0.0.1:{port}/auth?state={}",
                flow.state
            ))
        )
    );
    let target = format!("/auth?state={}&token={}", flow.state, "a".repeat(64));
    let wait = tokio::spawn(flow.wait(Duration::from_secs(5)));
    assert!(callback(port, "/auth?state=wrong&token=untrusted")
        .await
        .starts_with("HTTP/1.1 400"));
    assert!(callback(port, &target).await.starts_with("HTTP/1.1 200"));
    let state = wait.await.unwrap().unwrap();
    assert!(state.core.is_authenticated);
    assert_eq!(state.core.user_id.as_deref(), Some("test-user"));
    assert_eq!(backend.exchanges.load(Ordering::SeqCst), 1);
    manager.logout().await.unwrap();
    assert!(manager.link().token.lock().unwrap().is_none());
}

#[tokio::test]
async fn jwt_callback_validates_without_exchange_and_restores_in_new_manager() {
    let backend = Backend::start(false).await;
    let manager = backend.manager();
    let flow = BrowserLogin::start(manager.clone(), LoginProvider::Google)
        .await
        .unwrap();
    let port = flow.listener.local_addr().unwrap().port();
    let target = format!(
        "/auth?state={}&token={}",
        flow.state,
        percent_encode(&jwt())
    );
    let wait = tokio::spawn(flow.wait(Duration::from_secs(5)));
    callback(port, &target).await;
    wait.await.unwrap().unwrap();
    assert_eq!(backend.exchanges.load(Ordering::SeqCst), 0);
    let restored =
        SessionManager::new(manager.link().clone(), ClientHeaders::new("openhuman-test"));
    let calls = backend.profiles.load(Ordering::SeqCst);
    let state = crate::session::refresh_session(&restored).await.unwrap();
    assert_eq!(state.current_user.unwrap()["name"], "Test account");
    assert!(backend.profiles.load(Ordering::SeqCst) > calls);
}

#[tokio::test]
async fn rejected_callback_and_rejected_backend_never_store_credentials() {
    let backend = Backend::start(true).await;
    let manager = backend.manager();
    for token in [Some(jwt()), None] {
        let flow = BrowserLogin::start(manager.clone(), LoginProvider::Google)
            .await
            .unwrap();
        let port = flow.listener.local_addr().unwrap().port();
        let target = match token {
            Some(token) => format!("/auth?state={}&token={token}", flow.state),
            None => format!(
                "/auth?state={}&error=sensitive-provider-message",
                flow.state
            ),
        };
        let wait = tokio::spawn(flow.wait(Duration::from_secs(5)));
        callback(port, &target).await;
        let error = wait.await.unwrap().unwrap_err();
        assert!(!error.contains("sensitive-provider-message"));
        assert!(!error.contains(&jwt()));
        assert!(manager.link().token.lock().unwrap().is_none());
    }
}

#[tokio::test]
async fn rejected_persisted_session_is_cleared_on_restore() {
    let backend = Backend::start(true).await;
    let manager = backend.manager();
    *manager.link().token.lock().unwrap() = Some(jwt());
    *manager.link().state.lock().unwrap() = CoreAuthState {
        is_authenticated: true,
        credential: Some("session".into()),
        user_id: Some("test-user".into()),
        ..Default::default()
    };
    let state = crate::session::refresh_session(&manager).await.unwrap();
    assert!(!state.core.is_authenticated);
    assert!(manager.link().token.lock().unwrap().is_none());
}

#[tokio::test]
async fn cancellation_and_timeout_close_listener_without_storing() {
    let backend = Backend::start(false).await;
    let manager = backend.manager();
    let flow = BrowserLogin::start(manager.clone(), LoginProvider::Google)
        .await
        .unwrap();
    let port = flow.listener.local_addr().unwrap().port();
    let cancel = flow.cancellation();
    let wait = tokio::spawn(flow.wait(Duration::from_secs(30)));
    cancel.cancel();
    assert_eq!(wait.await.unwrap().unwrap_err(), "Login cancelled.");
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    let flow = BrowserLogin::start(manager.clone(), LoginProvider::Google)
        .await
        .unwrap();
    assert_eq!(
        flow.wait(Duration::ZERO).await.unwrap_err(),
        "Timed out waiting for browser login."
    );
    assert!(manager.link().token.lock().unwrap().is_none());
}

#[tokio::test]
async fn browser_callback_cannot_send_credentials_to_a_changed_backend() {
    let issuer = Backend::start(false).await;
    let changed = Backend::start(false).await;
    let manager = issuer.manager();
    for token in [jwt(), "a".repeat(64)] {
        *manager.link().base.lock().unwrap() = issuer.base.clone();
        let flow = BrowserLogin::start(manager.clone(), LoginProvider::Google)
            .await
            .unwrap();
        let target = format!(
            "/auth?state={}&token={}",
            flow.state,
            percent_encode(&token)
        );
        let port = flow.callback_port();
        *manager.link().base.lock().unwrap() = changed.base.clone();
        let wait = tokio::spawn(flow.wait(Duration::from_secs(5)));
        callback(port, &target).await;
        let error = wait.await.unwrap().unwrap_err();
        assert!(error.starts_with("SESSION_BACKEND_MISMATCH"));
        assert!(!error.contains(&token));
        assert!(manager.link().token.lock().unwrap().is_none());
    }
    for backend in [&issuer, &changed] {
        assert_eq!(backend.exchanges.load(Ordering::SeqCst), 0);
        assert_eq!(backend.profiles.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn persisted_restore_preserves_safe_actionable_backend_mismatch() {
    let issuer = Backend::start(false).await;
    let changed = Backend::start(false).await;
    let manager = issuer.manager();
    *manager.link().token.lock().unwrap() = Some(jwt());
    *manager.link().state.lock().unwrap() = CoreAuthState {
        is_authenticated: true,
        credential: Some("session".into()),
        issuing_backend: Some(issuer.base.clone()),
        ..Default::default()
    };
    *manager.link().base.lock().unwrap() = changed.base.clone();
    let error = crate::session::refresh_session(&manager).await.unwrap_err();
    assert!(error.starts_with("SESSION_BACKEND_MISMATCH"));
    assert!(error.contains("Restore that backend or start sign-in again"));
    assert!(!error.contains(&jwt()));
    assert_eq!(changed.profiles.load(Ordering::SeqCst), 0);
}
