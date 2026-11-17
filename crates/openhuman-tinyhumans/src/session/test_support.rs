//! Shared fixtures: an in-process backend stub (axum) that answers
//! `POST /auth/login-token/consume` and `GET /auth/me`, and a fake
//! [`CoreLink`] with the credential store the core would keep.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, LazyLock, Mutex};

use async_trait::async_trait;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::sync::Mutex as AsyncMutex;

use crate::session::link::{self, CoreLink};

/// Serialises tests that touch process-global state (environment variables,
/// the `identity` slot).
pub static ENV_LOCK: AsyncMutex<()> = AsyncMutex::const_new(());

/// Unpadded base64url, enough to assemble unsigned JWT fixtures at runtime
/// (kept out of source as literals so secret scanners do not trip on them).
fn b64url(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[n as usize & 63] as char);
        }
    }
    out
}

fn unsigned_jwt(claims: Value, signature: &str) -> String {
    let header = b64url(br#"{"alg":"none","typ":"JWT"}"#);
    let payload = b64url(claims.to_string().as_bytes());
    format!("{header}.{payload}.{signature}")
}

/// A JWT (alg none) with `sub`/`userId` = `user-123` and `exp` in 2100.
pub static LIVE_JWT: LazyLock<String> = LazyLock::new(|| {
    unsigned_jwt(
        json!({ "sub": "user-123", "userId": "user-123", "exp": 4102444800u64 }),
        "sig",
    )
});
/// Same subject, `exp` in 2001.
pub static EXPIRED_JWT: LazyLock<String> =
    LazyLock::new(|| unsigned_jwt(json!({ "sub": "user-123", "exp": 978307200u64 }), "sig"));
/// A JWT with an `exp` but no subject claim.
pub static LIVE_JWT_NO_SUB: LazyLock<String> =
    LazyLock::new(|| unsigned_jwt(json!({ "exp": 4102444800u64 }), "sig"));
/// Opaque, not a JWT.
pub const OPAQUE_TOKEN: &str = "mock-jwt-token";
/// The offline local session shape: signature segment literally `local`.
pub static LOCAL_TOKEN: LazyLock<String> =
    LazyLock::new(|| unsigned_jwt(json!({ "sub": "local" }), "local"));

/// One scripted `/auth/me` answer.
#[derive(Debug, Clone)]
pub enum MeAnswer {
    Ok(Value),
    Status(u16),
    /// Sleep this long before answering OK (for timeout tests).
    Slow(u64),
    /// Sleep this long before answering with a status (for race tests).
    SlowStatus(u64, u16),
}

#[derive(Default)]
pub struct StubState {
    pub me: Mutex<VecDeque<MeAnswer>>,
    pub me_calls: Mutex<Vec<HeaderMap>>,
    pub consume_calls: Mutex<Vec<Value>>,
    pub consume_jwt: Mutex<Option<String>>,
}

pub struct Backend {
    pub url: String,
    pub state: Arc<StubState>,
}

impl Backend {
    /// The `/auth/me` answers in order; the last one repeats forever.
    pub async fn start(answers: Vec<MeAnswer>) -> Self {
        let state = Arc::new(StubState::default());
        *state.me.lock().unwrap() = answers.into();
        *state.consume_jwt.lock().unwrap() = Some(LIVE_JWT.clone());
        let app = Router::new()
            .route("/auth/me", get(handle_me))
            .route("/auth/login-token/consume", post(handle_consume))
            .with_state(Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url: format!("http://{addr}"),
            state,
        }
    }

    pub fn me_calls(&self) -> usize {
        self.state.me_calls.lock().unwrap().len()
    }

    pub fn consume_calls(&self) -> Vec<Value> {
        self.state.consume_calls.lock().unwrap().clone()
    }
}

pub fn me_user() -> Value {
    json!({ "_id": "user-123", "email": "u@example.com", "name": "User" })
}

async fn handle_me(State(state): State<Arc<StubState>>, headers: HeaderMap) -> impl IntoResponse {
    state.me_calls.lock().unwrap().push(headers);
    let answer = {
        let mut queue = state.me.lock().unwrap();
        if queue.len() > 1 {
            queue.pop_front()
        } else {
            queue.front().cloned()
        }
    };
    match answer.unwrap_or(MeAnswer::Ok(me_user())) {
        MeAnswer::Ok(user) => (
            StatusCode::OK,
            Json(json!({ "success": true, "data": user })),
        )
            .into_response(),
        MeAnswer::Status(code) => (
            StatusCode::from_u16(code).unwrap(),
            Json(json!({ "success": false, "message": format!("status {code}") })),
        )
            .into_response(),
        MeAnswer::Slow(ms) => {
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            (
                StatusCode::OK,
                Json(json!({ "success": true, "data": me_user() })),
            )
                .into_response()
        }
        MeAnswer::SlowStatus(ms, code) => {
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            (
                StatusCode::from_u16(code).unwrap(),
                Json(json!({ "success": false, "message": format!("status {code}") })),
            )
                .into_response()
        }
    }
}

async fn handle_consume(
    State(state): State<Arc<StubState>>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    state.consume_calls.lock().unwrap().push(body.clone());
    let token = body.get("token").and_then(Value::as_str).unwrap_or("");
    if token == "expired" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "success": false, "message": "expired" })),
        )
            .into_response();
    }
    let jwt = state
        .consume_jwt
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_default();
    (
        StatusCode::OK,
        Json(json!({ "success": true, "data": { "jwt": jwt } })),
    )
        .into_response()
}

/// What the fake core holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredCredential {
    pub kind: String,
    pub token: String,
    pub user_id: Option<String>,
    pub user: Option<Value>,
    pub issuing_backend: Option<String>,
}

/// A [`CoreLink`] that behaves like the core's `auth.*` credential RPCs,
/// records every call, and reports `api_url`.
#[derive(Default)]
pub struct FakeCore {
    pub api_url: Mutex<String>,
    pub session: Mutex<Option<StoredCredential>>,
    pub api_key: Mutex<Option<String>>,
    pub calls: Mutex<Vec<(String, Value)>>,
    /// When set, every invoke fails with this message.
    pub fail_with: Mutex<Option<String>>,
    /// When set, only an invoke of this method name fails (with
    /// `fail_with`'s message, or a default when that is unset) — every other
    /// method still behaves normally. Lets a test fail just the core handoff
    /// (`AUTH_SET_CREDENTIAL`) without also breaking the API-URL resolution
    /// `client()` performs on every call.
    pub fail_method: Mutex<Option<String>>,
}

impl FakeCore {
    pub fn new(api_url: &str) -> Arc<Self> {
        let core = Self::default();
        *core.api_url.lock().unwrap() = api_url.to_string();
        Arc::new(core)
    }

    pub fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().unwrap().clone()
    }

    pub fn methods(&self) -> Vec<String> {
        self.calls().into_iter().map(|(m, _)| m).collect()
    }

    pub fn session(&self) -> Option<StoredCredential> {
        self.session.lock().unwrap().clone()
    }

    fn state(&self) -> Value {
        if let Some(key) = self.api_key.lock().unwrap().as_ref() {
            let _ = key;
            return json!({ "isAuthenticated": true, "credential": "api-key", "userId": null, "user": null, "profileId": null });
        }
        match self.session.lock().unwrap().as_ref() {
            Some(s) => json!({
                "isAuthenticated": true,
                "credential": s.kind,
                "userId": s.user_id,
                "user": s.user,
                "profileId": "app-session:default",
                "issuingBackend": s.issuing_backend,
            }),
            None => {
                json!({ "isAuthenticated": false, "userId": null, "user": null, "profileId": null })
            }
        }
    }
}

#[async_trait]
impl CoreLink for FakeCore {
    async fn invoke(&self, method: &str, params: Value) -> Result<Value, String> {
        self.calls
            .lock()
            .unwrap()
            .push((method.to_string(), params.clone()));
        if let Some(message) = self.fail_with.lock().unwrap().clone() {
            return Err(message);
        }
        if self.fail_method.lock().unwrap().as_deref() == Some(method) {
            return Err(format!("{method} failed (test-injected)"));
        }
        match method {
            link::CONFIG_RESOLVE_API_URL => {
                Ok(json!({ "api_url": self.api_url.lock().unwrap().clone() }))
            }
            link::AUTH_GET_STATE => Ok(self.state()),
            link::AUTH_GET_SESSION_TOKEN => Ok(json!({
                "result": { "token": self.session.lock().unwrap().as_ref().map(|s| s.token.clone()) },
                "logs": ["session token fetched"],
            })),
            link::AUTH_SET_CREDENTIAL => {
                let token = params
                    .get("token")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let kind = params
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("session")
                    .to_string();
                if token.is_empty() {
                    return Err("token is required".to_string());
                }
                if kind == "api-key" {
                    *self.api_key.lock().unwrap() = Some(token);
                    *self.session.lock().unwrap() = None;
                } else {
                    *self.api_key.lock().unwrap() = None;
                    *self.session.lock().unwrap() = Some(StoredCredential {
                        kind,
                        token,
                        user_id: params
                            .get("userId")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        user: params.get("user").cloned(),
                        issuing_backend: params
                            .get("issuingBackend")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    });
                }
                Ok(self.state())
            }
            link::AUTH_CLEAR_CREDENTIAL => {
                match params.get("kind").and_then(Value::as_str) {
                    Some("api-key") => *self.api_key.lock().unwrap() = None,
                    Some(_) => *self.session.lock().unwrap() = None,
                    None => {
                        *self.api_key.lock().unwrap() = None;
                        *self.session.lock().unwrap() = None;
                    }
                }
                Ok(json!({ "cleared": true }))
            }
            other => Err(format!("unknown method {other}")),
        }
    }
}
