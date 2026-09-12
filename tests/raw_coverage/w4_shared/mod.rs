//! Shared harness for the W4 e2e targets (`billing_cost_e2e`, `team_referral_e2e`,
//! `secrets_devices_e2e`).
//!
//! Included with `#[path]` rather than living in `tests/*.rs`, so cargo does not
//! auto-discover it as a fourth test binary. Every helper mirrors the equivalent
//! in `tests/json_rpc_e2e.rs`; the mock backend below only serves the routes the
//! billing / team / referral adapters actually call.
//!
//! All three suites are modules of the single `raw_coverage_all` target, so
//! every case here runs in one process alongside ~76 other suites. Two
//! consequences shape this file:
//!
//! 1. **`env_lock` binds to `crate::SHARED_ENV_LOCK`**, not a private static. A
//!    per-file lock compiles and passes in isolation while racing another
//!    suite's `HOME` / `OPENHUMAN_WORKSPACE` mutation under load.
//! 2. **The RPC bearer is read, never asserted.** `core::auth::RPC_TOKEN` is a
//!    process-global `OnceLock` and `init_rpc_token` is first-writer-wins, so a
//!    suite that hard-codes its own token gets a 401 whenever another suite
//!    initialised first. `rpc_token()` below asks for whatever is actually
//!    active. See `~/tinyhuman/bugs/e2e-wave-raw-coverage-shared-rpc-token.md`.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::extract::{Path as AxumPath, State};
use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use openhuman_core::core::auth::{get_rpc_token, init_rpc_token};

/// The one JWT the mock backend below accepts.
pub const SESSION_JWT: &str = "w4-e2e-session-jwt";
pub const SESSION_USER_ID: &str = "w4-user";

static AUTH_INIT: OnceLock<String> = OnceLock::new();
/// The crate-wide env mutex, not a private one — see the module note above.
static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;
static KEYRING_INIT: OnceLock<()> = OnceLock::new();

/// Serializes every case that touches process-global env (`HOME`,
/// `OPENHUMAN_WORKSPACE`, the backend-URL overrides) against every other
/// aggregated suite. Poison is recovered so one panicking case cannot wedge the
/// binary.
pub fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    let guard = ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // Under the lock, so this `set_var` cannot race a concurrent env read.
    KEYRING_INIT.get_or_init(|| {
        std::env::set_var("OPENHUMAN_KEYRING_BACKEND", "file");
    });
    guard
}

/// The RPC bearer this process actually validates against.
///
/// Deliberately *read* rather than chosen: `RPC_TOKEN` is a process-global
/// `OnceLock` and `init_rpc_token` returns early once it is set, so in the
/// aggregated binary the first suite to initialise fixes the token for everyone.
/// Asking for the live value makes these cases correct whether they win that
/// race or lose it. `init_rpc_token` with no `OPENHUMAN_CORE_TOKEN` set mints a
/// fresh token into a per-process directory, so we add no env racer of our own.
fn rpc_token() -> &'static str {
    AUTH_INIT.get_or_init(|| {
        let token_dir = std::env::temp_dir().join(format!("openhuman-w4-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&token_dir).expect("create the rpc token dir");
        init_rpc_token(&token_dir).expect("init the core rpc auth token");
        get_rpc_token()
            .expect("init_rpc_token must leave a token in place")
            .to_string()
    })
}

pub struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    pub fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, path.as_os_str());
        Self { key, old }
    }

    pub fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }

    pub fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

pub async fn serve_on_ephemeral(
    app: Router,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    let _ = rpc_token();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move { axum::serve(listener, app).await });
    (addr, handle)
}

pub async fn post_json_rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("reqwest client");
    let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    let url = format!("{}/rpc", rpc_base.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {}", rpc_token()))
        .json(&body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {url}: {e}"));
    assert!(
        resp.status().is_success(),
        "HTTP {} for {method}",
        resp.status()
    );
    resp.json::<Value>()
        .await
        .unwrap_or_else(|e| panic!("decoding the response to {method}: {e}"))
}

pub fn assert_no_error<'a>(v: &'a Value, context: &str) -> &'a Value {
    if let Some(err) = v.get("error") {
        panic!("{context}: JSON-RPC error: {err}");
    }
    v.get("result")
        .unwrap_or_else(|| panic!("{context}: response carried no `result`: {v}"))
}

pub fn assert_error<'a>(v: &'a Value, context: &str) -> &'a Value {
    v.get("error")
        .unwrap_or_else(|| panic!("{context}: expected a JSON-RPC error, got: {v}"))
}

/// The message text of a JSON-RPC error, for `contains` assertions.
pub fn error_message(v: &Value, context: &str) -> String {
    assert_error(v, context)
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: error object had no string `message`: {v}"))
        .to_string()
}

/// Peel the `{"result": inner, "logs": [...]}` envelope `into_cli_compatible_json`
/// adds when the outcome carries logs. Handlers that log conditionally return
/// both shapes, so every content assertion goes through here.
pub fn peel(v: &Value) -> &Value {
    if v.get("logs").is_some() {
        v.get("result").unwrap_or(v)
    } else {
        v
    }
}

/// The `logs` array of an outcome envelope, or empty when the handler logged nothing.
pub fn logs(v: &Value) -> Vec<String> {
    v.get("logs")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Write the minimal `config.toml` the RPC handlers need, plus any extra TOML
/// the caller appends (a `[dashboard.model_health]` table, `[[model_registry]]`
/// entries, an `[update]` override…).
///
/// Mirrors `tests/json_rpc_e2e.rs::write_min_config`, including the pre-login
/// `users/local` copy, and round-trips the result through `Config` so a typo in
/// a test fixture fails here rather than as a mystery default.
pub fn write_config(openhuman_dir: &Path, api_origin: &str, extra_toml: &str) {
    // `extra_toml` goes **before** `[secrets]`, not after. TOML scopes bare keys to
    // the most recent table header, so appending `schema_version = 99` after
    // `[secrets]` silently makes it `secrets.schema_version` — an unknown field
    // serde drops, leaving the real setting at its default. That failure is
    // invisible: the file looks right and the config parses.
    let cfg = format!(
        r#"api_url = "{api_origin}"
default_model = "w4-e2e-mock-model"
default_temperature = 0.7
chat_onboarding_completed = true
{extra_toml}
[secrets]
encrypt = false
"#
    );

    // Guard for exactly that class: every top-level key the caller declared must
    // still be top-level after assembly. A reparented key fails here, naming
    // itself, instead of surfacing as "the RPC handler ignored my config".
    let declared: toml::Value =
        toml::from_str(extra_toml).expect("the extra TOML fixture must parse on its own");
    let merged: toml::Value =
        toml::from_str(&cfg).expect("the assembled config.toml must parse");
    if let (Some(declared), Some(merged)) = (declared.as_table(), merged.as_table()) {
        for key in declared.keys() {
            assert!(
                merged.contains_key(key),
                "`{key}` was declared at the top level of the fixture but is not \
                 top-level in the assembled config.toml — it has been reparented \
                 into a preceding table and will be silently ignored.\n{cfg}"
            );
        }
    }

    fn write_one(dir: &Path, cfg: &str) {
        std::fs::create_dir_all(dir).expect("create config dir");
        std::fs::write(dir.join("config.toml"), cfg).expect("write config.toml");
    }

    write_one(openhuman_dir, &cfg);
    if openhuman_dir
        .file_name()
        .is_some_and(|name| name == std::ffi::OsStr::new(".openhuman"))
    {
        write_one(&openhuman_dir.join("users").join("local"), &cfg);
    }

    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(&cfg).expect("the fixture config.toml must match the Config schema");
}

/// A fully wired sandbox: temp `HOME`, a mock backend, and a live JSON-RPC server.
pub struct Harness {
    pub home: PathBuf,
    pub openhuman_dir: PathBuf,
    pub rpc_base: String,
    pub mock_origin: String,
    _tmp: tempfile::TempDir,
    _guards: Vec<EnvVarGuard>,
    mock_join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
    rpc_join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

impl Harness {
    /// `workspace_override`: when `Some`, exported as `OPENHUMAN_WORKSPACE`, which
    /// pins both `config.workspace_dir` and the config dir deterministically. Pass
    /// `None` for the session-backed cases, which need the `users/<id>` resolution
    /// path `auth_store_session` sets up.
    pub async fn start(extra_toml: &str, workspace_override: bool) -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        let openhuman_dir = home.join(".openhuman");

        let mut guards = vec![
            EnvVarGuard::set_to_path("HOME", &home),
            EnvVarGuard::unset("BACKEND_URL"),
            EnvVarGuard::unset("VITE_BACKEND_URL"),
        ];

        let (mock_addr, mock_join) = serve_on_ephemeral(mock_backend_router()).await;
        let mock_origin = format!("http://{mock_addr}");
        write_config(&openhuman_dir, &mock_origin, extra_toml);

        // The user-scoped config `auth_store_session` will resolve to once a
        // session is stored. Written unconditionally so a case can log in later.
        write_config(
            &openhuman_dir.join("users").join(SESSION_USER_ID),
            &mock_origin,
            extra_toml,
        );

        if workspace_override {
            // `<home>/workspace`, deliberately a **sibling** of `.openhuman`.
            // `resolve_config_dir_for_workspace` looks for the config dir at
            // `workspace.parent()/.openhuman`, so a workspace *inside* `.openhuman`
            // resolves to `<home>/.openhuman/.openhuman` — which does not exist, and
            // the whole `config.toml` is then silently replaced by schema defaults
            // while `workspace_dir` still looks correct. That failure is invisible
            // except as "my `[cost]` block did nothing".
            let workspace = home.join("workspace");
            std::fs::create_dir_all(&workspace).expect("create workspace dir");
            guards.push(EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace));
        } else {
            guards.push(EnvVarGuard::unset("OPENHUMAN_WORKSPACE"));
        }

        let (rpc_addr, rpc_join) =
            serve_on_ephemeral(openhuman_core::core::jsonrpc::build_core_http_router(false)).await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        Self {
            home,
            openhuman_dir,
            rpc_base: format!("http://{rpc_addr}"),
            mock_origin,
            _tmp: tmp,
            _guards: guards,
            mock_join,
            rpc_join,
        }
    }

    /// The workspace `OPENHUMAN_WORKSPACE` pins. Only meaningful when the harness
    /// was started with `workspace_override = true`.
    pub fn workspace(&self) -> PathBuf {
        self.home.join("workspace")
    }

    pub async fn call(&self, id: i64, method: &str, params: Value) -> Value {
        post_json_rpc(&self.rpc_base, id, method, params).await
    }

    /// Store `SESSION_JWT` so the hosted adapters have something to send.
    pub async fn login(&self) {
        let stored = self
            .call(
                1,
                "openhuman.auth_store_session",
                json!({ "token": SESSION_JWT, "user_id": SESSION_USER_ID }),
            )
            .await;
        assert_no_error(&stored, "auth_store_session");
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.mock_join.abort();
        self.rpc_join.abort();
    }
}

// ── Mock hosted backend ──────────────────────────────────────────────────────

fn err_json(status: StatusCode, message: &str) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(json!({ "success": false, "error": message, "message": message })),
    )
}

fn require_session(headers: &HeaderMap) -> Result<(), (StatusCode, Json<Value>)> {
    match headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
    {
        Some(value) if value == format!("Bearer {SESSION_JWT}") => Ok(()),
        Some(_) => Err(err_json(StatusCode::UNAUTHORIZED, "invalid session token")),
        None => Err(err_json(StatusCode::UNAUTHORIZED, "missing Authorization")),
    }
}

/// Records every request the core made, so a test can assert the adapter built
/// the path and body it claims to (query string included).
#[derive(Clone, Default)]
pub struct MockLog(pub std::sync::Arc<Mutex<Vec<Value>>>);

static MOCK_LOG: OnceLock<MockLog> = OnceLock::new();

pub fn mock_log() -> MockLog {
    MOCK_LOG.get_or_init(MockLog::default).clone()
}

impl MockLog {
    fn record(&self, method: &str, path: &str, body: Option<&Value>) {
        let mut guard = match self.0.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard.push(json!({ "method": method, "path": path, "body": body }));
    }

    pub fn clear(&self) {
        match self.0.lock() {
            Ok(mut g) => g.clear(),
            Err(p) => p.into_inner().clear(),
        }
    }

    pub fn entries(&self) -> Vec<Value> {
        match self.0.lock() {
            Ok(g) => g.clone(),
            Err(p) => p.into_inner().clone(),
        }
    }

    /// The recorded entry for the first request whose path starts with `prefix`.
    pub fn first_with_path_prefix(&self, prefix: &str) -> Option<Value> {
        self.entries().into_iter().find(|entry| {
            entry
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|p| p.starts_with(prefix))
        })
    }
}

async fn current_user(headers: HeaderMap) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    Ok(Json(json!({
        "success": true,
        "data": { "_id": SESSION_USER_ID, "username": "w4" }
    })))
}

macro_rules! logged_get {
    ($name:ident, $path:expr, $body:expr) => {
        async fn $name(
            State(log): State<MockLog>,
            headers: HeaderMap,
        ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
            require_session(&headers)?;
            log.record("GET", $path, None);
            Ok(Json(json!({ "success": true, "data": $body })))
        }
    };
}

logged_get!(
    credits_balance,
    "/payments/credits/balance",
    json!({ "balanceUsd": 42.5, "currency": "USD", "creditsCents": 4250 })
);
logged_get!(
    credits_auto_recharge,
    "/payments/credits/auto-recharge",
    json!({ "enabled": true, "thresholdUsd": 5.0, "rechargeAmountUsd": 25.0, "paymentMethodId": "pm_default" })
);
logged_get!(
    credits_cards,
    "/payments/credits/auto-recharge/cards",
    json!([
        { "id": "pm_1", "brand": "visa", "last4": "4242", "isDefault": true },
        { "id": "pm_2", "brand": "mastercard", "last4": "5555", "isDefault": false }
    ])
);
logged_get!(
    coupons_me,
    "/coupons/me",
    json!([{ "code": "WELCOME10", "creditsUsd": 10.0, "redeemedAt": "2026-01-02T03:04:05Z" }])
);
logged_get!(
    referral_stats,
    "/referral/stats",
    json!({ "code": "W4REF", "referredCount": 3, "creditsEarnedUsd": 15.0 })
);
logged_get!(
    teams_usage,
    "/teams/me/usage",
    json!({ "teamId": "team-1", "seatsUsed": 2, "seatsTotal": 5, "spendUsd": 12.25 })
);
logged_get!(
    teams_list,
    "/teams",
    json!([
        { "id": "team-1", "name": "Alpha", "role": "OWNER" },
        { "id": "team-2", "name": "Beta", "role": "MEMBER" }
    ])
);

async fn credits_transactions(
    State(log): State<MockLog>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    // Path *and* query — the limit/offset defaults are part of the contract this
    // records, and a test asserts on the exact query string.
    log.record("GET", &uri.to_string(), None);
    Ok(Json(json!({
        "success": true,
        "data": {
            "transactions": [
                { "id": "txn-1", "amountUsd": -1.25, "kind": "usage" },
                { "id": "txn-2", "amountUsd": 20.0, "kind": "top_up" }
            ],
            "total": 2
        }
    })))
}

async fn credits_setup_intent(
    State(log): State<MockLog>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    log.record("POST", "/payments/credits/auto-recharge/cards/setup-intent", None);
    Ok(Json(json!({
        "success": true,
        "data": { "clientSecret": "seti_w4_secret", "setupIntentId": "seti_w4" }
    })))
}

async fn credits_update_auto_recharge(
    State(log): State<MockLog>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    log.record("PATCH", "/payments/credits/auto-recharge", Some(&body));
    // Echo the request so the test can prove the adapter forwarded the payload
    // verbatim rather than reshaping it.
    let mut echoed = body.clone();
    if let Some(obj) = echoed.as_object_mut() {
        obj.insert("updated".to_string(), json!(true));
    }
    Ok(Json(json!({ "success": true, "data": echoed })))
}

async fn card_update(
    State(log): State<MockLog>,
    AxumPath(payment_method_id): AxumPath<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    log.record(
        "PATCH",
        &format!("/payments/credits/auto-recharge/cards/{payment_method_id}"),
        Some(&body),
    );
    Ok(Json(json!({
        "success": true,
        // `paymentMethodId` is echoed *decoded*, which is what proves the
        // adapter's percent-encoding survived a round trip through the path.
        "data": { "paymentMethodId": payment_method_id, "isDefault": body.get("isDefault") }
    })))
}

async fn card_delete(
    State(log): State<MockLog>,
    AxumPath(payment_method_id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    log.record(
        "DELETE",
        &format!("/payments/credits/auto-recharge/cards/{payment_method_id}"),
        None,
    );
    Ok(Json(json!({
        "success": true,
        "data": { "deleted": payment_method_id, "remaining": 1 }
    })))
}

async fn coupons_redeem(
    State(log): State<MockLog>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    log.record("POST", "/coupons/redeem", Some(&body));
    let code = body.get("code").and_then(Value::as_str).unwrap_or_default();
    if code != "WELCOME10" {
        return Err(err_json(
            StatusCode::BAD_REQUEST,
            &format!("coupon '{code}' is not redeemable"),
        ));
    }
    Ok(Json(json!({
        "success": true,
        "data": { "code": code, "creditsUsd": 10.0, "newBalanceUsd": 52.5 }
    })))
}

async fn referral_claim(
    State(log): State<MockLog>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    log.record("POST", "/referral/claim", Some(&body));
    let code = body.get("code").and_then(Value::as_str).unwrap_or_default();
    if code.is_empty() {
        return Err(err_json(StatusCode::BAD_REQUEST, "referral code is required"));
    }
    Ok(Json(json!({
        "success": true,
        "data": { "claimed": true, "code": code, "creditsUsd": 5.0 }
    })))
}

// ── Teams ────────────────────────────────────────────────────────────────────

fn team_row(id: &str) -> Value {
    json!({ "id": id, "name": format!("Team {id}"), "role": "OWNER", "memberCount": 2 })
}

async fn team_get(
    State(log): State<MockLog>,
    AxumPath(team_id): AxumPath<String>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    // The **raw** path, not the decoded id: the path-injection case turns on
    // what actually went over the wire, and a decoded id would hide it.
    log.record("GET", uri.path(), None);
    if team_id == "missing-team" {
        return Err(err_json(StatusCode::NOT_FOUND, "team not found"));
    }
    Ok(Json(json!({ "success": true, "data": team_row(&team_id) })))
}

async fn team_create(
    State(log): State<MockLog>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    log.record("POST", "/teams", Some(&body));
    let name = body.get("name").and_then(Value::as_str).unwrap_or_default();
    Ok(Json(json!({
        "success": true,
        "data": { "id": "team-new", "name": name, "role": "OWNER", "memberCount": 1 }
    })))
}

async fn team_update(
    State(log): State<MockLog>,
    AxumPath(team_id): AxumPath<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    log.record("PUT", &format!("/teams/{team_id}"), Some(&body));
    Ok(Json(json!({
        "success": true,
        // `name` is echoed straight from the body — absent when the adapter
        // dropped a blank name, which is exactly what one case asserts.
        "data": { "id": team_id, "name": body.get("name"), "updated": true }
    })))
}

async fn team_delete(
    State(log): State<MockLog>,
    AxumPath(team_id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    log.record("DELETE", &format!("/teams/{team_id}"), None);
    Ok(Json(json!({ "success": true, "data": { "deleted": team_id } })))
}

async fn team_switch(
    State(log): State<MockLog>,
    AxumPath(team_id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    log.record("POST", &format!("/teams/{team_id}/switch"), None);
    Ok(Json(json!({
        "success": true,
        "data": { "activeTeamId": team_id, "switched": true }
    })))
}

async fn team_leave(
    State(log): State<MockLog>,
    AxumPath(team_id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    log.record("POST", &format!("/teams/{team_id}/leave"), None);
    Ok(Json(json!({ "success": true, "data": { "left": team_id } })))
}

async fn team_join(
    State(log): State<MockLog>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    require_session(&headers)?;
    log.record("POST", "/teams/join", Some(&body));
    let code = body.get("code").and_then(Value::as_str).unwrap_or_default();
    if code != "JOIN-OK" {
        return Err(err_json(StatusCode::NOT_FOUND, "invite code not found"));
    }
    Ok(Json(json!({
        "success": true,
        "data": { "joined": true, "teamId": "team-1", "role": "MEMBER" }
    })))
}

pub fn mock_backend_router() -> Router {
    let log = mock_log();
    Router::new()
        .route("/settings", get(current_user))
        .route("/auth/me", get(current_user))
        // billing
        .route("/payments/credits/balance", get(credits_balance))
        .route("/payments/credits/transactions", get(credits_transactions))
        .route(
            "/payments/credits/auto-recharge",
            get(credits_auto_recharge).patch(credits_update_auto_recharge),
        )
        .route("/payments/credits/auto-recharge/cards", get(credits_cards))
        .route(
            "/payments/credits/auto-recharge/cards/setup-intent",
            post(credits_setup_intent),
        )
        .route(
            "/payments/credits/auto-recharge/cards/{payment_method_id}",
            patch(card_update).delete(card_delete),
        )
        .route("/coupons/redeem", post(coupons_redeem))
        .route("/coupons/me", get(coupons_me))
        // referral
        .route("/referral/stats", get(referral_stats))
        .route("/referral/claim", post(referral_claim))
        // teams
        .route("/teams", get(teams_list).post(team_create))
        .route("/teams/me/usage", get(teams_usage))
        .route("/teams/join", post(team_join))
        .route(
            "/teams/{team_id}",
            get(team_get).put(team_update).delete(team_delete),
        )
        .route("/teams/{team_id}/switch", post(team_switch))
        .route("/teams/{team_id}/leave", post(team_leave))
        .with_state(log)
}
