//! Doctor JSON-RPC and CLI end-to-end coverage against the shared local mock.
//!
//! The test starts its own copy of `scripts/mock-api-server.mjs` when no
//! verified loopback mock was supplied by `scripts/test-rust-with-mock.sh`.
//! This keeps the same target hermetic under the direct CI Lite coverage runner.

#[path = "support/tinyhumans_boot.rs"]
mod tinyhumans_boot;

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::http::header::AUTHORIZATION;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::sync::MutexGuard;

const MOCK_TOKEN_HEADER: &str = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0";
const MOCK_TOKEN_PAYLOAD: &str = "eyJzdWIiOiJ1c2VyLTEyMyIsInVzZXJJZCI6InVzZXItMTIzIiwidGdVc2VySWQiOiJ1c2VyLTEyMyIsImV4cCI6NDEwMjQ0NDgwMH0";
const EXPIRED_TOKEN_PAYLOAD: &str =
    "eyJzdWIiOiJ1c2VyLTEyMyIsInVzZXJJZCI6InVzZXItMTIzIiwidGdVc2VySWQiOiJ1c2VyLTEyMyIsImV4cCI6MX0";
const MOCK_USER_ID: &str = "user-123";
const RPC_TOKEN: &str = "doctor-e2e-in-process-rpc-token";
const MOCK_BACKEND_PLACEHOLDER: &str = "__MOCK_BACKEND_URL__";

static RPC_AUTH_INIT: OnceLock<()> = OnceLock::new();
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A loopback server for one serialized fixture.
struct MockBackend {
    origin: String,
    child: Option<Child>,
}

impl MockBackend {
    async fn start(inherited_backend_url: Option<&str>) -> Self {
        let inherited_origin = inherited_backend_url.and_then(loopback_origin);
        if let Some(origin) = inherited_origin {
            if is_shared_mock(&origin).await {
                return Self {
                    origin,
                    child: None,
                };
            }
        }

        let port = TcpListener::bind("127.0.0.1:0")
            .expect("reserve a loopback mock port")
            .local_addr()
            .expect("mock listener address")
            .port();
        let script = Path::new(env!("OPENHUMAN_REPOSITORY_ROOT"))
            .join("scripts/mock-api-server.mjs")
            .canonicalize()
            .expect("shared mock server script exists");
        let child = Command::new("node")
            .arg(script)
            .arg("--port")
            .arg(port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the shared Node mock (node must be on PATH)");
        let mut mock = Self {
            origin: format!("http://127.0.0.1:{port}"),
            child: Some(child),
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if is_shared_mock(&mock.origin).await {
                return mock;
            }
            if mock
                .child
                .as_mut()
                .and_then(|process| process.try_wait().ok())
                .flatten()
                .is_some()
            {
                panic!("the shared mock server exited before becoming healthy");
            }
            assert!(
                Instant::now() < deadline,
                "the shared mock did not become healthy on loopback"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn reset(&self) {
        let response = reqwest::Client::new()
            .post(format!("{}/__admin/reset", self.origin))
            .json(&json!({}))
            .send()
            .await
            .expect("reset shared mock state");
        assert!(response.status().is_success(), "mock reset succeeded");
    }

    async fn set_behavior(&self, behavior: Value, replace: bool) {
        let response = reqwest::Client::new()
            .post(format!("{}/__admin/behavior", self.origin))
            .json(&json!({
                "behavior": behavior,
                "mode": if replace { "replace" } else { "merge" },
            }))
            .send()
            .await
            .expect("configure shared mock behavior");
        assert!(response.status().is_success(), "mock behavior accepted");
    }

    async fn request_paths(&self) -> Vec<(String, String)> {
        let body: Value = reqwest::get(format!("{}/__admin/requests", self.origin))
            .await
            .expect("read shared mock request log")
            .json()
            .await
            .expect("mock request log is JSON");
        body["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| {
                Some((
                    row["method"].as_str()?.to_owned(),
                    row["url"].as_str()?.split('?').next()?.to_owned(),
                ))
            })
            .collect()
    }

    async fn memory_scopes(&self) -> Vec<Value> {
        let response = reqwest::Client::new()
            .get(format!("{}/memory/scopes", self.origin))
            .bearer_auth(mock_token())
            .send()
            .await
            .expect("inspect mock memory after doctor cleanup");
        assert!(
            response.status().is_success(),
            "mock memory scopes succeeded"
        );
        let body: Value = response.json().await.expect("mock memory scopes are JSON");
        body.pointer("/data/items")
            .and_then(Value::as_array)
            .cloned()
            .expect("mock memory scopes contain an item list")
    }
}

impl Drop for MockBackend {
    fn drop(&mut self) {
        if let Some(mut process) = self.child.take() {
            let _ = process.kill();
            let _ = process.wait();
        }
    }
}

fn loopback_origin(raw: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(raw).ok()?;
    if url.scheme() != "http"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
        || !matches!(url.host_str()?, "127.0.0.1" | "localhost" | "::1")
    {
        return None;
    }
    url.set_path("");
    Some(url.as_str().trim_end_matches('/').to_owned())
}

fn mock_token() -> String {
    format!("{MOCK_TOKEN_HEADER}.{MOCK_TOKEN_PAYLOAD}.e2e")
}

fn expired_mock_token() -> String {
    format!("{MOCK_TOKEN_HEADER}.{EXPIRED_TOKEN_PAYLOAD}.e2e")
}

async fn is_shared_mock(origin: &str) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };
    let Ok(response) = client.get(format!("{origin}/__admin/health")).send().await else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    response
        .json::<Value>()
        .await
        .is_ok_and(|body| body["ok"] == true && body["port"].as_u64().is_some())
}

/// Removes inherited OpenHuman/backend credentials and config overrides for
/// the duration of one serialized case, then restores the parent environment.
struct IsolatedEnvironment {
    previous: Vec<(OsString, OsString)>,
    managed: Vec<OsString>,
}

impl IsolatedEnvironment {
    fn install(home: &Path, action_dir: &Path, backend_url: &str) -> Self {
        let mut previous = Vec::new();
        let mut managed = [
            "HOME",
            "USERPROFILE",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "APPDATA",
            "LOCALAPPDATA",
            "BACKEND_URL",
            "VITE_BACKEND_URL",
            "OPENHUMAN_ACTION_DIR",
            "OPENHUMAN_KEYRING_BACKEND",
        ]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
        let inherited = std::env::vars_os().collect::<Vec<_>>();
        for (key, value) in inherited {
            if is_isolated_environment_key(&key) || managed.contains(&key) {
                if !managed.contains(&key) {
                    managed.push(key.clone());
                }
                previous.push((key.clone(), value));
                // SAFETY: Fixture holds ENV_LOCK from setup through Drop and
                // all test functions use Tokio's current-thread runtime. No
                // peer test task can read or mutate this process environment
                // during the mutation; the CLI child receives its own env.
                unsafe { std::env::remove_var(key) };
            }
        }

        // SAFETY: same serialized, current-thread fixture guarantee as above.
        unsafe {
            std::env::set_var("HOME", home);
            std::env::set_var("USERPROFILE", home);
            std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
            std::env::set_var("XDG_DATA_HOME", home.join(".local/share"));
            std::env::set_var("XDG_CACHE_HOME", home.join(".cache"));
            std::env::set_var("XDG_STATE_HOME", home.join(".local/state"));
            std::env::set_var("APPDATA", home.join("AppData/Roaming"));
            std::env::set_var("LOCALAPPDATA", home.join("AppData/Local"));
            std::env::set_var("BACKEND_URL", backend_url);
            std::env::set_var("VITE_BACKEND_URL", backend_url);
            std::env::set_var("OPENHUMAN_ACTION_DIR", action_dir);
            std::env::set_var("OPENHUMAN_KEYRING_BACKEND", "file");
        }
        Self { previous, managed }
    }
}

fn is_isolated_environment_key(key: &OsStr) -> bool {
    let key = key.to_string_lossy();
    key == "HOME"
        || key == "USERPROFILE"
        || key.starts_with("OPENHUMAN_")
        || key.starts_with("BACKEND_")
        || key == "VITE_BACKEND_URL"
}

impl Drop for IsolatedEnvironment {
    fn drop(&mut self) {
        // SAFETY: Fixture still owns ENV_LOCK while this guard is dropped, so
        // no concurrent test task reads or mutates the process environment.
        unsafe {
            for key in &self.managed {
                std::env::remove_var(key);
            }
            for (key, value) in &self.previous {
                std::env::set_var(key, value);
            }
        }
    }
}

struct Fixture {
    _home: TempDir,
    mock: MockBackend,
    rpc_base: String,
    rpc_task: tokio::task::JoinHandle<Result<(), std::io::Error>>,
    _environment: IsolatedEnvironment,
    _environment_lock: MutexGuard<'static, ()>,
}

impl Fixture {
    async fn new(config: &str) -> Self {
        let environment_lock = ENV_LOCK.lock().await;
        let inherited_backend_url = std::env::var("BACKEND_URL").ok();
        let mock = MockBackend::start(inherited_backend_url.as_deref()).await;
        let home = tempfile::tempdir().expect("fixture home");
        let action_dir = home.path().join("action");
        fs::create_dir_all(&action_dir).expect("create fixture action directory");
        let environment = IsolatedEnvironment::install(home.path(), &action_dir, &mock.origin);
        write_profile(home.path(), MOCK_USER_ID, &mock.origin, config, true);

        tinyhumans_boot::boot();
        ensure_rpc_auth();
        mock.reset().await;
        mock.set_behavior(healthy_mock_behavior(), true).await;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind in-process JSON-RPC listener");
        let address = listener.local_addr().expect("RPC listener address");
        let router = openhuman_rpc::server::build_core_http_router(false);
        let rpc_task = tokio::spawn(async move { axum::serve(listener, router).await });

        Self {
            _home: home,
            mock,
            rpc_base: format!("http://{address}"),
            rpc_task,
            _environment: environment,
            _environment_lock: environment_lock,
        }
    }

    async fn report(&self, params: Value) -> Value {
        let before = snapshot_tree(self._home.path());
        let response = reqwest::Client::new()
            .post(format!("{}/rpc", self.rpc_base))
            .header(AUTHORIZATION, format!("Bearer {RPC_TOKEN}"))
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "openhuman.doctor_report",
                "params": params,
            }))
            .send()
            .await
            .expect("call doctor JSON-RPC");
        assert!(
            response.status().is_success(),
            "doctor RPC transport status"
        );
        let envelope: Value = response.json().await.expect("doctor RPC JSON");
        assert!(envelope.get("error").is_none(), "doctor RPC succeeded");
        let result = envelope
            .get("result")
            .unwrap_or_else(|| panic!("doctor RPC has result: {envelope}"));
        let report = if result.get("logs").is_some() {
            result
                .get("result")
                .unwrap_or_else(|| panic!("doctor outcome has report: {result}"))
        } else {
            result
        }
        .clone();
        let serialized = serde_json::to_string(&report).expect("serialize doctor report");
        assert!(
            !serialized.contains(&mock_token()),
            "the report does not contain the synthetic session token"
        );
        assert_unchanged(&before, &snapshot_tree(self._home.path()));
        report
    }

    async fn models(&self, params: Value) -> Value {
        let before = snapshot_tree(self._home.path());
        let response = reqwest::Client::new()
            .post(format!("{}/rpc", self.rpc_base))
            .header(AUTHORIZATION, format!("Bearer {RPC_TOKEN}"))
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "openhuman.doctor_models",
                "params": params,
            }))
            .send()
            .await
            .expect("call doctor models JSON-RPC");
        assert!(
            response.status().is_success(),
            "doctor models transport status"
        );
        let envelope: Value = response.json().await.expect("doctor models JSON");
        assert!(
            envelope.get("error").is_none(),
            "doctor models RPC succeeded"
        );
        let result = envelope
            .get("result")
            .unwrap_or_else(|| panic!("doctor models RPC has result: {envelope}"));
        let models = if result.get("logs").is_some() {
            result
                .get("result")
                .unwrap_or_else(|| panic!("doctor models outcome has report: {result}"))
        } else {
            result
        }
        .clone();
        assert!(
            !serde_json::to_string(&models)
                .expect("serialize doctor models report")
                .contains(&mock_token()),
            "the report does not contain the synthetic session token"
        );
        assert_unchanged(&before, &snapshot_tree(self._home.path()));
        models
    }

    fn run_cli(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_openhuman-core"));
        command
            .args(args)
            .current_dir(self._home.path())
            .env_clear()
            .env("HOME", self._home.path())
            .env("USERPROFILE", self._home.path())
            .env("BACKEND_URL", &self.mock.origin)
            .env("VITE_BACKEND_URL", &self.mock.origin)
            .env("OPENHUMAN_ACTION_DIR", self._home.path().join("action"))
            .env("OPENHUMAN_KEYRING_BACKEND", "file");

        for name in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP", "TMPDIR"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command.output().expect("run openhuman-core doctor CLI")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.rpc_task.abort();
    }
}

fn healthy_mock_behavior() -> Value {
    json!({
        "httpFaultRules": serde_json::to_string(&json!([
            {"method": "GET", "path": "/", "status": 200, "body": {"status": "ok"}}
        ])).expect("encode mock health response"),
        "llmStreamScript": serde_json::to_string(&json!([
            {"text": "OK"},
            {"finish": "stop"}
        ])).expect("encode mock completion script"),
    })
}

fn write_profile(home: &Path, profile_id: &str, backend_url: &str, config: &str, activate: bool) {
    let root = home.join(".openhuman");
    let profile_dir = root.join("users").join(profile_id);
    let files_dir = profile_dir.join("files");
    let sandbox_dir = profile_dir.join("sandbox");
    fs::create_dir_all(&files_dir).expect("create profile files directory");
    fs::create_dir_all(&sandbox_dir).expect("create profile sandbox directory");
    if activate {
        fs::write(
            root.join("active_user.toml"),
            format!("user_id = \"{profile_id}\"\n"),
        )
        .expect("write active user fixture");
    }
    fs::write(profile_dir.join(".layout"), "2\n").expect("mark profile layout v2");

    let content = config.replace(MOCK_BACKEND_PLACEHOLDER, backend_url);
    fs::write(profile_dir.join("config.toml"), content).expect("write doctor config fixture");
    let auth_profiles = json!({
        "schema_version": 1,
        "active_profiles": {"app-session": "app-session:default"},
        "profiles": {
            "app-session:default": {
                "provider": "app-session",
                "profile_name": "default",
                "kind": "token",
                "token": mock_token(),
                "metadata": {"user_id": MOCK_USER_ID},
                "created_at": "2025-01-01T00:00:00Z",
                "updated_at": "2025-01-01T00:00:00Z"
            }
        }
    });
    fs::write(
        profile_dir.join("auth-profiles.json"),
        serde_json::to_vec(&auth_profiles).expect("serialize synthetic auth profile"),
    )
    .expect("write synthetic auth profile");
}

fn write_expired_session(home: &Path) {
    let path = home
        .join(".openhuman/users")
        .join(MOCK_USER_ID)
        .join("auth-profiles.json");
    let mut profiles: Value =
        serde_json::from_slice(&fs::read(&path).expect("read synthetic auth profile"))
            .expect("synthetic auth profile is JSON");
    profiles["profiles"]["app-session:default"]["token"] = json!(expired_mock_token());
    profiles["profiles"]["app-session:default"]["metadata"]["session_expires_at"] =
        json!("2000-01-01T00:00:00Z");
    fs::write(
        path,
        serde_json::to_vec(&profiles).expect("serialize expired auth profile"),
    )
    .expect("write expired synthetic auth profile");
}

async fn assert_expired_session_is_seeded(home: &Path) {
    let before = snapshot_tree(home);
    let config = openhuman_core::doctor::profile::resolve_desktop_profile_for_doctor(
        &home.join(".openhuman"),
        Some(MOCK_USER_ID),
    )
    .await
    .expect("resolve isolated doctor profile read-only")
    .config;
    let profile =
        openhuman_core::security::credentials::session_support::load_app_session_profile_read_only(
            &config,
        )
        .expect("read session profile without mutation")
        .expect("synthetic session profile exists");
    let token = openhuman_core::security::credentials::session_support::session_token_from_profile(
        Some(&profile),
    )
    .expect("session profile carries a token");
    let now = chrono::Utc::now();
    assert!(
        openhuman_core::security::credentials::jwt::decode_jwt_exp(&token)
            .expect("synthetic JWT has an expiration")
            < now,
        "fixture JWT itself is expired"
    );
    assert_eq!(
        openhuman_core::security::credentials::session_support::classify_session_token(
            Some(&profile),
            now,
        ),
        openhuman_core::security::credentials::session_support::SessionTokenCheck::Expired,
        "the read-only session classifier sees an expired profile before doctor runs"
    );
    assert_unchanged(&before, &snapshot_tree(home));
}

fn snapshot_tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, current: &Path, snapshot: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let Ok(entries) = fs::read_dir(current) else {
            return;
        };
        let mut entries = entries
            .map(|entry| entry.expect("read fixture directory entry").path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            // LLVM coverage writes these process-local files into the test's
            // current directory; they are instrumentation output, not doctor
            // state or a mutation of the selected profile.
            if path
                .extension()
                .is_some_and(|extension| extension == "profraw")
            {
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .expect("path remains under fixture root");
            let metadata = fs::symlink_metadata(&path).expect("fixture path metadata");
            if metadata.file_type().is_symlink() {
                let target = fs::read_link(&path).expect("read fixture symlink");
                snapshot.insert(
                    relative.to_path_buf(),
                    target.as_os_str().as_encoded_bytes().to_vec(),
                );
            } else if metadata.is_dir() {
                snapshot.insert(relative.to_path_buf(), b"directory".to_vec());
                visit(root, &path, snapshot);
            } else {
                snapshot.insert(
                    relative.to_path_buf(),
                    fs::read(path).expect("read fixture file"),
                );
            }
        }
    }

    let mut snapshot = BTreeMap::new();
    visit(root, root, &mut snapshot);
    snapshot
}

fn assert_unchanged(before: &BTreeMap<PathBuf, Vec<u8>>, after: &BTreeMap<PathBuf, Vec<u8>>) {
    if before != after {
        let changed = before
            .keys()
            .chain(after.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter(|path| before.get(*path) != after.get(*path))
            .cloned()
            .collect::<Vec<_>>();
        panic!("doctor changed fixture files: {changed:?}");
    }
}

fn finding<'a>(report: &'a Value, check_id: &str) -> &'a Value {
    report["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|finding| finding["check_id"] == check_id)
        .unwrap_or_else(|| panic!("doctor report includes {check_id}"))
}

fn assert_finding(report: &Value, check_id: &str, severity: &str) {
    let selected = finding(report, check_id);
    assert_eq!(
        selected["severity"],
        severity,
        "{check_id}: {}",
        selected["message"]
            .as_str()
            .unwrap_or("<no safe finding message>")
    );
}

fn fixture(name: &str) -> &'static str {
    match name {
        "healthy" => include_str!("fixtures/doctor/healthy.toml"),
        "unknown-key" => include_str!("fixtures/doctor/unknown-key.toml"),
        "unknown-provider" => include_str!("fixtures/doctor/unknown-provider.toml"),
        "byok-no-key" => include_str!("fixtures/doctor/byok-no-key.toml"),
        "disabled-component-reference" => {
            include_str!("fixtures/doctor/disabled-component-reference.toml")
        }
        "storage-missing-driver" => {
            include_str!("fixtures/doctor/storage-missing-driver.toml")
        }
        _ => panic!("unknown doctor fixture"),
    }
}

async fn fixture_named(name: &str) -> Fixture {
    Fixture::new(fixture(name)).await
}

fn assert_cli_report(output: &Output, expected_code: i32, check_id: Option<&str>) -> Value {
    assert_eq!(
        output.status.code(),
        Some(expected_code),
        "doctor CLI exit code; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("doctor CLI stdout is JSON ({error})"));
    assert_eq!(report["schema_version"], 2);
    if let Some(check_id) = check_id {
        assert!(report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item["check_id"] == check_id }));
    }
    report
}

fn ensure_rpc_auth() {
    RPC_AUTH_INIT.get_or_init(|| {
        openhuman_core::core::auth::init_rpc_token_with_value(RPC_TOKEN)
            .expect("initialize in-memory RPC bearer for doctor E2E");
    });
}

#[tokio::test(flavor = "current_thread")]
async fn models_rpc_inspects_the_selected_profile_without_writing_it() {
    let fixture = fixture_named("healthy").await;
    let models = fixture.models(json!({ "use_cache": false })).await;
    assert!(models["summary"].is_object());
    assert!(models["entries"].is_array());
}

#[path = "doctor_e2e/cases.rs"]
mod cases;
