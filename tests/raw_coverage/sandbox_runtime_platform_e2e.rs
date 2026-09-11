//! JSON-RPC E2E coverage for the platform-runtime namespaces that had **zero**
//! `tests/**/*_e2e.rs` reach before this file: `sandbox`, `worktree`,
//! `http_host`, `workspace`, and `modules`.
//!
//! Every case dispatches over the real `build_core_http_router` HTTP surface —
//! the same transport the desktop shell uses — and asserts on the *content* of
//! the reply, not merely that it is `Ok`. Each namespace gets at least one
//! failure path, because that is where the bugs in an untested controller live.
//!
//! This file is a **module** of the aggregated `raw_coverage_all` target, not a
//! target of its own — `build.rs` globs `tests/raw_coverage/` and generates the
//! `mod` list, so adding the file is all the registration there is.
//!
//! Run with:
//! `cargo test --test raw_coverage_all --features "$(bash scripts/ci/product-features.sh)"`
//!
//! Isolation rules this file obeys:
//! - `HOME`, `OPENHUMAN_WORKSPACE`, `OPENHUMAN_ACTION_DIR` and `PATH` are
//!   process-global, and every aggregated suite shares this one process, so
//!   every case takes [`env_lock`] — which is a reference to the **crate-wide**
//!   `SHARED_ENV_LOCK`, not a private mutex — and restores via [`EnvVarGuard`].
//!   A file-local lock would compile, pass alone, and race another worker's
//!   suite under load.
//! - `[modules] enabled = false` in the test config: `modules_load` with modules
//!   enabled would try to *download* a pinned release artifact. The disabled
//!   branch is the one an offline CI can assert on deterministically.
//! - `sandbox_cleanup_orphans` shells out to `docker kill`. Both cases below
//!   point `PATH` at a temp dir so the real daemon is never reached — a test
//!   that kills the developer's containers is not a test.

use std::net::SocketAddr;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::http::header::AUTHORIZATION;
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::core::auth::{get_rpc_token, init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;

const TEST_RPC_TOKEN: &str = "sandbox-runtime-platform-e2e-token";

static AUTH_INIT: OnceLock<()> = OnceLock::new();

/// Every aggregated suite shares one process, so a private env lock would not
/// mutually exclude with anyone else's. Bind to the crate-wide mutex.
static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

struct EnvVarGuard {
    key: &'static str,
    old: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var_os(key);
        std::env::set_var(key, path.as_os_str());
        Self { key, old }
    }

    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var_os(key);
        std::env::remove_var(key);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Initialise the core RPC bearer and return **the token this process actually
/// validates against**.
///
/// `core::auth::RPC_TOKEN` is a process-global `OnceLock` and `init_rpc_token`
/// is deliberately idempotent, so inside the aggregated `raw_coverage_all`
/// binary the *first* suite to initialise pins the bearer for every later one.
/// Sending our own `TEST_RPC_TOKEN` unconditionally would therefore 401 for
/// whichever suites lose that race — an order-dependent failure. Ask for the
/// active token instead of assuming ours won.
/// See `~/tinyhuman/bugs/e2e-wave-raw-coverage-shared-rpc-token.md`.
fn ensure_rpc_auth() -> String {
    AUTH_INIT.get_or_init(|| {
        if std::env::var(CORE_TOKEN_ENV_VAR)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .is_none()
        {
            std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN);
        }
        let token_dir = std::env::temp_dir().join("openhuman-sandbox-runtime-platform-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
    get_rpc_token()
        .expect("core RPC token must be initialised before serving")
        .to_string()
}

async fn serve_rpc() -> (
    SocketAddr,
    String,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    let token = ensure_rpc_auth();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc listener");
    let addr = listener.local_addr().expect("rpc listener addr");
    let router = build_core_http_router(false);
    let join = tokio::spawn(async move { axum::serve(listener, router).await });
    (addr, token, join)
}

/// `[modules] enabled = false` is load-bearing, not incidental — see the module
/// docs. `[local_ai] enabled = false` and `memory.provider = "none"` keep the
/// config load from reaching for a model or an embedding endpoint.
fn write_min_config(openhuman_dir: &Path) {
    std::fs::create_dir_all(openhuman_dir).expect("create .openhuman");
    let cfg = r#"api_url = "http://127.0.0.1:9"
default_model = "e2e-model"
default_temperature = 0.2

[secrets]
encrypt = false

[local_ai]
enabled = false

[modules]
enabled = false
allow_download = false

[memory]
provider = "none"
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0

[memory_tree]
embedding_strict = false
"#;
    std::fs::write(openhuman_dir.join("config.toml"), cfg).expect("write config.toml");
    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(cfg).expect("test config must match schema");
}

struct TestHarness {
    tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    rpc_base: String,
    token: String,
    join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

impl TestHarness {
    fn home(&self) -> &Path {
        self.tmp.path()
    }

    fn workspace(&self) -> std::path::PathBuf {
        self.tmp.path().join("workspace")
    }

    /// Dispatch one JSON-RPC call over the harness's router with the bearer
    /// this process actually validates against.
    async fn rpc(&self, id: i64, method: &str, params: Value) -> Value {
        rpc_with_token(&self.rpc_base, &self.token, id, method, params).await
    }
}

/// Boot a router over a fresh temp `HOME`. `action_dir` pins
/// `OPENHUMAN_ACTION_DIR` (the repo root every `worktree` op anchors on);
/// `None` points it at an empty non-git directory, which is the "user has not
/// opened a repo" state `worktree_list` must degrade over.
async fn setup(action_dir: Option<&Path>) -> TestHarness {
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    write_min_config(&openhuman_home);
    let workspace = home.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let empty_action_dir = home.join("no-repo");
    std::fs::create_dir_all(&empty_action_dir).expect("create action dir");

    let guards = vec![
        EnvVarGuard::set_to_path("HOME", home),
        EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace),
        EnvVarGuard::set_to_path(
            "OPENHUMAN_ACTION_DIR",
            action_dir.unwrap_or(&empty_action_dir),
        ),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
    ];

    let (addr, token, join) = serve_rpc().await;
    TestHarness {
        tmp,
        _guards: guards,
        rpc_base: format!("http://{addr}"),
        token,
        join,
    }
}

async fn rpc_with_token(
    rpc_base: &str,
    token: &str,
    id: i64,
    method: &str,
    params: Value,
) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("client");
    let url = format!("{}/rpc", rpc_base.trim_end_matches('/'));
    let response = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .unwrap_or_else(|err| panic!("POST {url} {method}: {err}"));
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "HTTP transport should accept {method}"
    );
    response
        .json::<Value>()
        .await
        .unwrap_or_else(|err| panic!("json for {method}: {err}"))
}

fn ok<'a>(value: &'a Value, context: &str) -> &'a Value {
    if let Some(error) = value.get("error") {
        panic!("{context}: unexpected JSON-RPC error: {error}");
    }
    value
        .get("result")
        .unwrap_or_else(|| panic!("{context}: missing result: {value}"))
}

/// The JSON-RPC error *message* for a call that must fail. Panics with the full
/// envelope when the call unexpectedly succeeded — a controller that silently
/// accepts bad input is exactly what these cases exist to catch.
fn err_message(value: &Value, context: &str) -> String {
    let error = value
        .get("error")
        .unwrap_or_else(|| panic!("{context}: expected JSON-RPC error, got: {value}"));
    error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: error without message: {error}"))
        .to_string()
}

/// Controllers wrap their payload in `RpcOutcome::into_cli_compatible_json`,
/// which nests the value under `result` and carries `logs` alongside. Handlers
/// that return a bare `serde_json::json!` do not. Unwrap one level when it is
/// there so a case can assert on the payload either way.
fn payload<'a>(value: &'a Value, context: &str) -> &'a Value {
    let result = ok(value, context);
    result.get("result").unwrap_or(result)
}

/// A directory containing only the executables this test wants found. Used to
/// keep `docker` off `PATH` (or to substitute a stub for it).
fn stub_bin_dir(tmp: &Path, name: &str, script: Option<&str>) -> std::path::PathBuf {
    let dir = tmp.join(format!("stubbin-{name}"));
    std::fs::create_dir_all(&dir).expect("create stub bin dir");
    if let Some(body) = script {
        let path = dir.join(name);
        std::fs::write(&path, body).expect("write stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod stub");
        }
    }
    dir
}

// ── sandbox ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn sandbox_resolve_policy_maps_each_mode_and_remote_flag_to_a_backend() {
    let _lock = env_lock();
    let harness = setup(None).await;

    // `none` → no backend, network unrestricted.
    let none = harness
        .rpc(
            60_001,
            "openhuman.sandbox_resolve_policy",
            json!({ "sandbox_mode": "none" }),
        )
        .await;
    let none = payload(&none, "resolve_policy none");
    assert_eq!(none.get("backend"), Some(&json!("none")));
    assert_eq!(none.get("allow_network"), Some(&json!(true)));
    assert_eq!(
        none.get("docker_overrides"),
        Some(&Value::Null),
        "a non-docker backend must not carry docker overrides: {none}"
    );

    // `read_only` collapses onto the same backend but is a distinct input.
    let read_only = harness
        .rpc(
            60_002,
            "openhuman.sandbox_resolve_policy",
            json!({ "sandbox_mode": "read_only" }),
        )
        .await;
    assert_eq!(
        payload(&read_only, "resolve_policy read_only").get("backend"),
        Some(&json!("none"))
    );

    // `sandboxed` on a local session → the OS jail, network still allowed.
    let local = harness
        .rpc(
            60_003,
            "openhuman.sandbox_resolve_policy",
            json!({ "sandbox_mode": "sandboxed", "is_remote": false }),
        )
        .await;
    let local = payload(&local, "resolve_policy sandboxed/local");
    assert_eq!(local.get("backend"), Some(&json!("local")));
    assert_eq!(local.get("allow_network"), Some(&json!(true)));

    // `sandboxed` + remote → Docker, and network is cut. This is the security
    // decision the whole namespace exists to make, so assert both halves.
    let remote = harness
        .rpc(
            60_004,
            "openhuman.sandbox_resolve_policy",
            json!({ "sandbox_mode": "sandboxed", "is_remote": true }),
        )
        .await;
    let remote = payload(&remote, "resolve_policy sandboxed/remote");
    assert_eq!(remote.get("backend"), Some(&json!("docker")));
    assert_eq!(
        remote.get("allow_network"),
        Some(&json!(false)),
        "a remote sandboxed session must not get outbound network: {remote}"
    );
    let overrides = remote
        .get("docker_overrides")
        .unwrap_or_else(|| panic!("docker backend must carry overrides: {remote}"));
    assert!(
        overrides.get("image").and_then(Value::as_str).is_some(),
        "docker overrides must name an image: {overrides}"
    );
    assert_eq!(overrides.get("read_only_rootfs"), Some(&json!(true)));

    // An unknown mode string degrades to `none` rather than erroring — assert
    // the documented degradation so a future strict-parse change is visible.
    let unknown = harness
        .rpc(
            60_005,
            "openhuman.sandbox_resolve_policy",
            json!({ "sandbox_mode": "not-a-mode" }),
        )
        .await;
    assert_eq!(
        payload(&unknown, "resolve_policy unknown").get("backend"),
        Some(&json!("none"))
    );

    // Failure path: the required param is enforced at the dispatch boundary
    // (`core::all::validate_params`), before the handler — which would
    // otherwise have defaulted it to `none` and answered a policy the caller
    // never asked for.
    let missing = harness
        .rpc(60_006, "openhuman.sandbox_resolve_policy", json!({}))
        .await;
    assert!(
        err_message(&missing, "resolve_policy missing mode")
            .contains("missing required param 'sandbox_mode'"),
        "an omitted `sandbox_mode` must be refused, not silently defaulted"
    );

    // …and the same gate rejects a param the schema does not declare, so a
    // stale frontend cannot smuggle an ignored field past it.
    let unknown_param = harness
        .rpc(
            60_007,
            "openhuman.sandbox_resolve_policy",
            json!({ "sandbox_mode": "none", "not_a_field": true }),
        )
        .await;
    assert!(
        err_message(&unknown_param, "resolve_policy unknown param")
            .contains("unknown param 'not_a_field' for sandbox.resolve_policy"),
        "an undeclared param must be refused by name"
    );

    harness.join.abort();
}

#[tokio::test]
async fn sandbox_validate_policy_flags_host_network_and_dangerous_roots() {
    let _lock = env_lock();
    let harness = setup(None).await;

    // A clean policy validates.
    let safe = harness
        .rpc(
            60_010,
            "openhuman.sandbox_validate_policy",
            json!({
                "policy": {
                    "backend": "docker",
                    "workspace_root": "/home/user/project",
                    "read_only_mounts": ["/usr/lib"],
                    "allow_network": false,
                    "env_passthrough": [],
                    "docker_overrides": { "network": "bridge", "extra_caps_drop": [] }
                }
            }),
        )
        .await;
    let safe = payload(&safe, "validate_policy safe");
    assert_eq!(safe.get("valid"), Some(&json!(true)));
    assert_eq!(safe.get("issues"), Some(&json!([])));

    // Host networking + a docker-socket mount + `/` as the workspace root:
    // three independent findings, all of which must be reported.
    let unsafe_policy = harness
        .rpc(
            60_011,
            "openhuman.sandbox_validate_policy",
            json!({
                "policy": {
                    "backend": "docker",
                    "workspace_root": "/",
                    "read_only_mounts": ["/var/run/docker.sock", "/etc"],
                    "allow_network": true,
                    "env_passthrough": [],
                    "docker_overrides": { "network": "host", "extra_caps_drop": [] }
                }
            }),
        )
        .await;
    let unsafe_payload = payload(&unsafe_policy, "validate_policy unsafe");
    assert_eq!(unsafe_payload.get("valid"), Some(&json!(false)));
    let issues = unsafe_payload
        .get("issues")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("issues must be an array: {unsafe_payload}"));
    let joined = issues
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        joined.contains("host network"),
        "host networking must be flagged: {joined}"
    );
    assert!(
        joined.contains("/var/run/docker.sock"),
        "the docker socket mount must be flagged: {joined}"
    );
    assert!(
        joined.contains("/etc"),
        "the /etc mount must be flagged: {joined}"
    );
    assert!(
        joined.contains("Workspace root is a dangerous path"),
        "`/` as the workspace root must be flagged: {joined}"
    );
    assert_eq!(
        issues.len(),
        4,
        "expected exactly the four findings above: {joined}"
    );

    // Failure path: a policy that is not a SandboxPolicy at all.
    let bad = harness
        .rpc(
            60_012,
            "openhuman.sandbox_validate_policy",
            json!({ "policy": { "backend": "not-a-backend" } }),
        )
        .await;
    let message = err_message(&bad, "validate_policy bad backend");
    assert!(
        message.contains("Invalid policy"),
        "unparseable policy must be rejected, got: {message}"
    );

    // Failure path: the required `policy` param omitted entirely.
    let missing = harness
        .rpc(60_013, "openhuman.sandbox_validate_policy", json!({}))
        .await;
    let message = err_message(&missing, "validate_policy missing");
    assert!(
        message.contains("missing required param 'policy'"),
        "a missing `policy` is caught by the dispatch-boundary gate, got: {message}"
    );

    harness.join.abort();
}

#[tokio::test]
async fn sandbox_status_reports_a_backend_handle_for_each_requested_kind() {
    let _lock = env_lock();
    let harness = setup(None).await;

    let none = harness
        .rpc(
            60_020,
            "openhuman.sandbox_status",
            json!({ "backend": "none" }),
        )
        .await;
    let none = payload(&none, "sandbox_status none");
    assert_eq!(
        none.get("kind"),
        Some(&json!("none")),
        "backend=none must report the none handle: {none}"
    );
    assert!(
        none.get("status").is_some(),
        "a backend handle must carry a status: {none}"
    );

    // `local` resolves the OS jail. Availability is host-dependent (Landlock on
    // Linux, Seatbelt on macOS), so assert the handle's identity, not its
    // verdict.
    let local = harness
        .rpc(
            60_021,
            "openhuman.sandbox_status",
            json!({ "backend": "local" }),
        )
        .await;
    let local = payload(&local, "sandbox_status local");
    assert_eq!(local.get("kind"), Some(&json!("local")));

    // An unrecognised backend string falls back to `none` rather than erroring.
    let bogus = harness
        .rpc(
            60_022,
            "openhuman.sandbox_status",
            json!({ "backend": "kubernetes" }),
        )
        .await;
    assert_eq!(
        payload(&bogus, "sandbox_status bogus").get("kind"),
        Some(&json!("none")),
        "an unknown backend name must degrade to `none`, not error"
    );

    harness.join.abort();
}

/// What `sandbox_status` *should* answer for a local backend whose OS jail is
/// not available on this host.
///
/// Ignored because it fails today: `ops::create_sandbox_backend` returns
/// `SandboxStatus::Ready` from **both** arms of its availability check, so the
/// reply says the confinement is ready on a host where it degraded to a noop.
/// See `~/tinyhuman/bugs/e2e-wave-sandbox-local-jail-always-reports-ready.md`.
/// Un-ignore when that is fixed; the assertion below is the contract.
#[tokio::test]
#[ignore = "documents a bug: local jail unavailability is reported as `ready` \
            (bugs/e2e-wave-sandbox-local-jail-always-reports-ready.md)"]
async fn sandbox_status_local_backend_reports_an_unavailable_jail_as_not_ready() {
    let _lock = env_lock();
    let harness = setup(None).await;

    let local = harness
        .rpc(
            60_025,
            "openhuman.sandbox_status",
            json!({ "backend": "local" }),
        )
        .await;
    let local = payload(&local, "sandbox_status local");
    assert_eq!(local.get("kind"), Some(&json!("local")));

    // `is_available` lives on the `JailBackend` trait, which must be in scope
    // for the method call on the returned `Arc<dyn JailBackend>`.
    use openhuman_core::openhuman::sandbox::cwd_jail::{default_backend, JailBackend as _};
    let jail_available = default_backend().is_available();
    if jail_available {
        assert_eq!(local.get("status"), Some(&json!("ready")));
    } else {
        assert_ne!(
            local.get("status"),
            Some(&json!("ready")),
            "an unavailable OS jail must not be reported as ready — the caller \
             would believe commands are confined when they run unconfined: {local}"
        );
    }

    harness.join.abort();
}

#[tokio::test]
async fn sandbox_cleanup_orphans_counts_zero_with_a_stub_docker_and_errors_without_one() {
    let _lock = env_lock();
    let harness = setup(None).await;

    // Happy path: a `docker` that lists nothing. The handler must report zero
    // cleaned — and must never reach `docker kill`, which is why the stub is a
    // stub. `docker ps -q` printing nothing is exactly the no-orphans case.
    let stub_dir = stub_bin_dir(harness.home(), "docker", Some("#!/bin/sh\nexit 0\n"));
    {
        let _path = EnvVarGuard::set_to_path("PATH", &stub_dir);
        let clean = harness
            .rpc(60_030, "openhuman.sandbox_cleanup_orphans", json!({}))
            .await;
        assert_eq!(
            payload(&clean, "cleanup_orphans stub").get("cleaned"),
            Some(&json!(0)),
            "a docker that lists no containers must report zero cleaned"
        );
    }

    // Failure path: no `docker` on PATH at all. The spawn error must surface as
    // an RPC error rather than a silent zero — "cleaned 0" and "could not look"
    // are different answers and the UI must be able to tell them apart.
    let empty_dir = stub_bin_dir(harness.home(), "nodocker", None);
    {
        let _path = EnvVarGuard::set_to_path("PATH", &empty_dir);
        let failed = harness
            .rpc(60_031, "openhuman.sandbox_cleanup_orphans", json!({}))
            .await;
        let message = err_message(&failed, "cleanup_orphans no docker");
        assert!(
            message.contains("Cleanup failed"),
            "a missing docker binary must surface as a cleanup failure, got: {message}"
        );
    }

    harness.join.abort();
}

// ── worktree ───────────────────────────────────────────────────────────────

/// Build a real git repo with one committed file and an isolated worker
/// worktree under the `.claude/worktrees` convention path the RPC manages.
/// Returns the repo root and the managed worktree path.
fn init_repo_with_managed_worktree(root: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let run = |args: &[&str], cwd: &Path| {
        let out = Command::new("git")
            .args(args)
            .current_dir(cwd)
            // A developer's global hooks / gpg signing config would otherwise
            // decide whether this fixture builds.
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "e2e")
            .env("GIT_AUTHOR_EMAIL", "e2e@example.test")
            .env("GIT_COMMITTER_NAME", "e2e")
            .env("GIT_COMMITTER_EMAIL", "e2e@example.test")
            .output()
            .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };

    // Canonicalise before git ever sees the path: on macOS a tempdir lives
    // under `/var/folders/…`, `/var` is a symlink to `/private/var`, and
    // `git worktree list --porcelain` reports the *resolved* path. Comparing a
    // symlinked path against a resolved one is how this fixture would fail for
    // a reason that has nothing to do with the controller under test.
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    let repo = std::fs::canonicalize(&repo).expect("canonicalise repo root");
    run(&["init", "-b", "main", "."], &repo);
    std::fs::write(repo.join("README.md"), "hello\n").expect("write README");
    run(&["add", "README.md"], &repo);
    run(&["commit", "--no-gpg-sign", "-m", "initial"], &repo);

    let worktree = repo.join(".claude").join("worktrees").join("worker-1");
    run(
        &[
            "worktree",
            "add",
            "-b",
            "worker/1",
            worktree.to_str().expect("worktree path utf-8"),
        ],
        &repo,
    );
    (repo, worktree)
}

#[tokio::test]
async fn worktree_list_status_diff_and_remove_round_trip_over_a_real_repo() {
    let outer = tempdir().expect("tempdir");
    let (repo, worktree) = init_repo_with_managed_worktree(outer.path());

    let _lock = env_lock();
    let harness = setup(Some(&repo)).await;

    // list — the managed worktree is discovered and reported with its branch.
    let listed = harness
        .rpc(61_001, "openhuman.worktree_list", json!({}))
        .await;
    let listed = payload(&listed, "worktree_list");
    let worktrees = listed
        .get("worktrees")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("worktrees must be an array: {listed}"));
    assert_eq!(
        worktrees.len(),
        1,
        "only the managed worker worktree is listed, never the main checkout: {listed}"
    );
    assert_eq!(
        worktrees[0].get("branch"),
        Some(&json!("worker/1")),
        "the worktree's branch must be reported: {listed}"
    );
    assert_eq!(
        worktrees[0].get("isDirty"),
        Some(&json!(false)),
        "a freshly added worktree is clean: {listed}"
    );
    assert_eq!(
        listed.get("overlaps"),
        Some(&json!([])),
        "one worktree cannot overlap with itself: {listed}"
    );

    // status — clean tree, no changed files.
    let status = harness
        .rpc(
            61_002,
            "openhuman.worktree_status",
            json!({ "path": worktree.to_str().unwrap() }),
        )
        .await;
    let status = payload(&status, "worktree_status clean");
    assert_eq!(status.get("branch"), Some(&json!("worker/1")));
    assert_eq!(status.get("isDirty"), Some(&json!(false)));
    assert_eq!(status.get("changedFiles"), Some(&json!([])));

    // diff — an empty summary for a clean tree.
    let diff = harness
        .rpc(
            61_003,
            "openhuman.worktree_diff",
            json!({ "path": worktree.to_str().unwrap() }),
        )
        .await;
    assert_eq!(
        payload(&diff, "worktree_diff clean")
            .get("summary")
            .and_then(Value::as_str),
        Some(""),
        "a clean worktree diffs to nothing"
    );

    // Dirty it, then re-read: status and diff must both notice.
    std::fs::write(worktree.join("README.md"), "hello\nchanged\n").expect("dirty the worktree");
    std::fs::write(worktree.join("NEW.md"), "untracked\n").expect("add untracked");

    let dirty = harness
        .rpc(
            61_004,
            "openhuman.worktree_status",
            json!({ "path": worktree.to_str().unwrap() }),
        )
        .await;
    let dirty = payload(&dirty, "worktree_status dirty");
    assert_eq!(dirty.get("isDirty"), Some(&json!(true)));
    let changed = dirty
        .get("changedFiles")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("changedFiles must be an array: {dirty}"))
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(
        changed.iter().any(|f| f.ends_with("README.md")),
        "the modified tracked file must be listed: {changed:?}"
    );

    let dirty_diff = harness
        .rpc(
            61_005,
            "openhuman.worktree_diff",
            json!({ "path": worktree.to_str().unwrap() }),
        )
        .await;
    let summary = payload(&dirty_diff, "worktree_diff dirty")
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    assert!(
        summary.contains("README.md"),
        "the diff summary must name the changed file: {summary}"
    );

    // remove without force must refuse a dirty worktree — the whole point of
    // the flag is that uncommitted work needs an explicit decision.
    let refused = harness
        .rpc(
            61_006,
            "openhuman.worktree_remove",
            json!({ "path": worktree.to_str().unwrap() }),
        )
        .await;
    let message = err_message(&refused, "worktree_remove dirty without force");
    assert!(
        message.to_lowercase().contains("dirty") || message.to_lowercase().contains("uncommitted"),
        "removing a dirty worktree must say why it refused, got: {message}"
    );
    assert!(
        worktree.exists(),
        "a refused remove must leave the checkout on disk"
    );

    // remove with force succeeds and the checkout is gone.
    let removed = harness
        .rpc(
            61_007,
            "openhuman.worktree_remove",
            json!({ "path": worktree.to_str().unwrap(), "force": true }),
        )
        .await;
    assert_eq!(
        payload(&removed, "worktree_remove forced").get("removed"),
        Some(&json!(true))
    );
    assert!(
        !worktree.exists(),
        "a forced remove must delete the checkout: {}",
        worktree.display()
    );

    let after = harness
        .rpc(61_008, "openhuman.worktree_list", json!({}))
        .await;
    assert_eq!(
        payload(&after, "worktree_list after remove")
            .get("worktrees")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0),
        "the removed worktree must be gone from the listing"
    );

    harness.join.abort();
}

#[tokio::test]
async fn worktree_path_guard_rejects_relative_and_unmanaged_paths() {
    let outer = tempdir().expect("tempdir");
    let (repo, _worktree) = init_repo_with_managed_worktree(outer.path());

    let _lock = env_lock();
    let harness = setup(Some(&repo)).await;

    // The guard is the security boundary for this namespace: without it,
    // `worktree_remove` would happily delete the user's main checkout.
    for (id, method) in [
        (62_001, "openhuman.worktree_status"),
        (62_002, "openhuman.worktree_diff"),
        (62_003, "openhuman.worktree_remove"),
    ] {
        // Two distinct gates, two distinct messages. Omitting `path` never
        // reaches the handler — `core::all::validate_params` rejects it from
        // the schema. A *blank* path does reach the handler, and is refused
        // by `require_managed_worktree_path` further down.
        let missing = harness.rpc(id, method, json!({})).await;
        assert!(
            err_message(&missing, method).contains("missing required param 'path'"),
            "{method} must require `path` at the dispatch boundary"
        );

        let relative = harness
            .rpc(id + 10, method, json!({ "path": "relative/dir" }))
            .await;
        assert!(
            err_message(&relative, method).contains("absolute path required"),
            "{method} must reject a relative path"
        );

        // The repo root itself is absolute *and* real — and must still be
        // refused, because it is not under `.claude/worktrees`.
        let unmanaged = harness
            .rpc(id + 20, method, json!({ "path": repo.to_str().unwrap() }))
            .await;
        assert!(
            err_message(&unmanaged, method).contains("not a managed worker worktree"),
            "{method} must refuse the main checkout"
        );

        let blank = harness.rpc(id + 30, method, json!({ "path": "   " })).await;
        assert!(
            err_message(&blank, method).contains("missing required param: path"),
            "{method} must treat a blank path as missing"
        );
    }

    harness.join.abort();
}

#[tokio::test]
async fn worktree_list_degrades_to_empty_outside_a_git_repo() {
    let _lock = env_lock();
    // `action_dir` = an empty non-git directory: the state of a user who has
    // not opened a project. The panel must render, not error.
    let harness = setup(None).await;

    let listed = harness
        .rpc(63_001, "openhuman.worktree_list", json!({}))
        .await;
    let listed = payload(&listed, "worktree_list non-git");
    assert_eq!(listed.get("worktrees"), Some(&json!([])));
    assert_eq!(listed.get("overlaps"), Some(&json!([])));

    harness.join.abort();
}

// ── http_host ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn http_host_start_list_get_serves_the_directory_then_stop_retires_it() {
    let _lock = env_lock();
    let harness = setup(None).await;

    let served_dir = harness.home().join("served");
    std::fs::create_dir_all(&served_dir).expect("create served dir");
    std::fs::write(served_dir.join("hello.txt"), "hosted-body\n").expect("write served file");

    // start — port 0 lets the OS pick, so this never collides with a parallel
    // worker's test.
    let started = harness
        .rpc(
            64_001,
            "openhuman.http_host_start",
            json!({
                "directory": served_dir.to_str().unwrap(),
                "port": 0,
                "server_name": "e2e-hosted",
                "disable_auth": true
            }),
        )
        .await;
    let server = payload(&started, "http_host_start")
        .get("server")
        .unwrap_or_else(|| panic!("start must return a server: {started}"))
        .clone();
    let server_id = server
        .get("server_id")
        .and_then(Value::as_str)
        .expect("server_id")
        .to_string();
    let port = server.get("port").and_then(Value::as_u64).expect("port");
    assert!(
        port > 0,
        "port 0 must be replaced by the bound port: {server}"
    );
    assert_eq!(server.get("bind_host"), Some(&json!("127.0.0.1")));
    assert_eq!(server.get("server_name"), Some(&json!("e2e-hosted")));
    assert_eq!(
        server.get("local_url").and_then(Value::as_str),
        Some(format!("http://127.0.0.1:{port}/").as_str())
    );
    assert_eq!(
        server.pointer("/auth/enabled"),
        Some(&json!(false)),
        "disable_auth=true must be honoured: {server}"
    );

    // The server is real: fetch the file it hosts over HTTP.
    let body = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client")
        .get(format!("http://127.0.0.1:{port}/hello.txt"))
        .send()
        .await
        .expect("GET hosted file")
        .text()
        .await
        .expect("hosted body");
    assert_eq!(
        body, "hosted-body\n",
        "http_host_start must actually serve the directory"
    );

    // list — the running server is present.
    let listed = harness
        .rpc(64_002, "openhuman.http_host_list", json!({}))
        .await;
    let servers = payload(&listed, "http_host_list")
        .get("servers")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("servers must be an array: {listed}"))
        .clone();
    assert!(
        servers
            .iter()
            .any(|s| s.get("server_id").and_then(Value::as_str) == Some(server_id.as_str())),
        "the started server must appear in the listing: {listed}"
    );

    // get — the same record by id.
    let fetched = harness
        .rpc(
            64_003,
            "openhuman.http_host_get",
            json!({ "server_id": server_id }),
        )
        .await;
    let fetched = payload(&fetched, "http_host_get")
        .get("server")
        .unwrap_or_else(|| panic!("get must return a server: {fetched}"))
        .clone();
    assert_eq!(fetched.get("port"), Some(&json!(port)));
    assert_eq!(
        fetched.get("directory"),
        server.get("directory"),
        "get must return the same canonicalised directory as start"
    );

    // stop — reports the final snapshot, and the id stops resolving.
    let stopped = harness
        .rpc(
            64_004,
            "openhuman.http_host_stop",
            json!({ "server_id": server_id }),
        )
        .await;
    let stopped = payload(&stopped, "http_host_stop");
    assert_eq!(stopped.get("stopped"), Some(&json!(true)));
    assert_eq!(
        stopped.pointer("/server/server_id").and_then(Value::as_str),
        Some(server_id.as_str())
    );

    let gone = harness
        .rpc(
            64_005,
            "openhuman.http_host_get",
            json!({ "server_id": server_id }),
        )
        .await;
    assert!(
        err_message(&gone, "http_host_get after stop").contains("not found"),
        "a stopped server must no longer resolve"
    );

    harness.join.abort();
}

#[tokio::test]
async fn http_host_rejects_a_missing_directory_and_an_unknown_server_id() {
    let _lock = env_lock();
    let harness = setup(None).await;

    let absent = harness.home().join("does-not-exist");
    let failed = harness
        .rpc(
            65_001,
            "openhuman.http_host_start",
            json!({ "directory": absent.to_str().unwrap(), "port": 0, "disable_auth": true }),
        )
        .await;
    let message = err_message(&failed, "http_host_start missing dir");
    assert!(
        !message.is_empty(),
        "starting on a missing directory must fail with a reason"
    );

    // A file is not a directory — the canonicalisation guard must say so
    // rather than binding a server that serves nothing.
    let file = harness.home().join("a-file.txt");
    std::fs::write(&file, "x").expect("write file");
    let not_a_dir = harness
        .rpc(
            65_002,
            "openhuman.http_host_start",
            json!({ "directory": file.to_str().unwrap(), "port": 0, "disable_auth": true }),
        )
        .await;
    assert!(
        !err_message(&not_a_dir, "http_host_start on a file").is_empty(),
        "hosting a regular file must be refused"
    );

    let unknown = harness
        .rpc(
            65_003,
            "openhuman.http_host_get",
            json!({ "server_id": "00000000-0000-0000-0000-000000000000" }),
        )
        .await;
    assert!(
        err_message(&unknown, "http_host_get unknown").contains("not found"),
        "an unknown server id must report not-found"
    );

    let stop_unknown = harness
        .rpc(
            65_004,
            "openhuman.http_host_stop",
            json!({ "server_id": "00000000-0000-0000-0000-000000000000" }),
        )
        .await;
    assert!(
        !err_message(&stop_unknown, "http_host_stop unknown").is_empty(),
        "stopping an unknown server must fail rather than report success"
    );

    // A malformed request (no `server_id`) must be a params error.
    let malformed = harness
        .rpc(65_005, "openhuman.http_host_get", json!({}))
        .await;
    assert!(
        err_message(&malformed, "http_host_get no id")
            .contains("missing required param 'server_id'"),
        "a missing server_id must be caught by the dispatch-boundary gate"
    );

    harness.join.abort();
}

#[tokio::test]
async fn http_host_start_mints_basic_auth_credentials_by_default() {
    let _lock = env_lock();
    let harness = setup(None).await;

    let served_dir = harness.home().join("guarded");
    std::fs::create_dir_all(&served_dir).expect("create dir");
    std::fs::write(served_dir.join("secret.txt"), "guarded-body\n").expect("write file");

    // No `disable_auth` — the documented default is auth ON with a generated
    // password. A regression that flipped this default would silently expose a
    // user's directory on their LAN.
    let started = harness
        .rpc(
            66_001,
            "openhuman.http_host_start",
            json!({
                "directory": served_dir.to_str().unwrap(),
                "port": 0,
                "username": "e2e-user"
            }),
        )
        .await;
    let server = payload(&started, "http_host_start auth")
        .get("server")
        .expect("server")
        .clone();
    let server_id = server
        .get("server_id")
        .and_then(Value::as_str)
        .expect("server_id")
        .to_string();
    let port = server.get("port").and_then(Value::as_u64).expect("port");
    assert_eq!(server.pointer("/auth/enabled"), Some(&json!(true)));
    assert_eq!(server.pointer("/auth/username"), Some(&json!("e2e-user")));
    let password = server
        .pointer("/auth/password")
        .and_then(Value::as_str)
        .expect("a generated password")
        .to_string();
    assert!(
        !password.is_empty(),
        "auth-enabled hosting must mint a password"
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client");

    // Unauthenticated request is refused...
    let refused = client
        .get(format!("http://127.0.0.1:{port}/secret.txt"))
        .send()
        .await
        .expect("unauthenticated GET");
    assert_eq!(
        refused.status(),
        StatusCode::UNAUTHORIZED,
        "an auth-enabled hosted dir must reject an anonymous request"
    );

    // ...and the minted credentials work.
    let allowed = client
        .get(format!("http://127.0.0.1:{port}/secret.txt"))
        .basic_auth("e2e-user", Some(&password))
        .send()
        .await
        .expect("authenticated GET");
    assert_eq!(allowed.status(), StatusCode::OK);
    assert_eq!(allowed.text().await.expect("body"), "guarded-body\n");

    let _ = harness
        .rpc(
            66_002,
            "openhuman.http_host_stop",
            json!({ "server_id": server_id }),
        )
        .await;

    harness.join.abort();
}

// ── workspace ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn workspace_persona_files_round_trip_read_write_and_reset() {
    let _lock = env_lock();
    let harness = setup(None).await;

    // read on a fresh workspace falls back to the bundled default.
    let fresh = harness
        .rpc(
            67_001,
            "openhuman.workspace_file_read",
            json!({ "filename": "SOUL.md" }),
        )
        .await;
    let fresh = payload(&fresh, "workspace_file_read fresh");
    assert_eq!(fresh.get("filename"), Some(&json!("SOUL.md")));
    assert_eq!(
        fresh.get("is_default"),
        Some(&json!(true)),
        "a missing workspace copy must be reported as the bundled default: {fresh}"
    );
    let bundled = fresh
        .get("contents")
        .and_then(Value::as_str)
        .expect("contents")
        .to_string();
    assert!(
        !bundled.is_empty(),
        "the bundled default must not be empty — the editor renders it"
    );

    // write persists and flips is_default off.
    let written = harness
        .rpc(
            67_002,
            "openhuman.workspace_file_write",
            json!({ "filename": "SOUL.md", "contents": "# e2e soul\nBe brief.\n" }),
        )
        .await;
    let written = payload(&written, "workspace_file_write");
    assert_eq!(written.get("is_default"), Some(&json!(false)));
    assert_eq!(
        written.get("contents"),
        Some(&json!("# e2e soul\nBe brief.\n"))
    );
    assert_eq!(
        std::fs::read_to_string(harness.workspace().join("SOUL.md")).expect("SOUL.md on disk"),
        "# e2e soul\nBe brief.\n",
        "the write must land in the workspace, not just in the reply"
    );

    // read returns what was written.
    let read_back = harness
        .rpc(
            67_003,
            "openhuman.workspace_file_read",
            json!({ "filename": "SOUL.md" }),
        )
        .await;
    let read_back = payload(&read_back, "workspace_file_read after write");
    assert_eq!(read_back.get("is_default"), Some(&json!(false)));
    assert_eq!(
        read_back.get("contents"),
        Some(&json!("# e2e soul\nBe brief.\n"))
    );

    // reset restores the bundled default, on disk as well as in the reply.
    let reset = harness
        .rpc(
            67_004,
            "openhuman.workspace_file_reset",
            json!({ "filename": "SOUL.md" }),
        )
        .await;
    let reset = payload(&reset, "workspace_file_reset");
    assert_eq!(reset.get("is_default"), Some(&json!(true)));
    assert_eq!(reset.get("contents"), Some(&json!(bundled.clone())));
    assert_eq!(
        std::fs::read_to_string(harness.workspace().join("SOUL.md")).expect("SOUL.md on disk"),
        bundled,
        "reset must rewrite the file, not only report the default"
    );

    // IDENTITY.md is the second allowlisted file and must behave the same.
    let identity = harness
        .rpc(
            67_005,
            "openhuman.workspace_file_read",
            json!({ "filename": "IDENTITY.md" }),
        )
        .await;
    assert_eq!(
        payload(&identity, "workspace_file_read IDENTITY").get("filename"),
        Some(&json!("IDENTITY.md"))
    );

    harness.join.abort();
}

#[tokio::test]
async fn workspace_allowlist_refuses_arbitrary_paths_and_oversize_writes() {
    let _lock = env_lock();
    let harness = setup(None).await;

    // The allowlist is the only thing standing between this RPC and arbitrary
    // read/write under the workspace, so exercise the shapes an attacker would
    // reach for.
    for (id, filename) in [
        (68_001, "config.toml"),
        (68_002, "../../etc/passwd"),
        (68_003, "soul.md"), // case matters — the allowlist is exact
        (68_004, ""),
    ] {
        let refused = harness
            .rpc(
                id,
                "openhuman.workspace_file_read",
                json!({ "filename": filename }),
            )
            .await;
        let message = err_message(&refused, &format!("workspace_file_read {filename}"));
        assert!(
            message.contains("is not an editable workspace file"),
            "reading '{filename}' must be refused by the allowlist, got: {message}"
        );

        let refused_write = harness
            .rpc(
                id + 100,
                "openhuman.workspace_file_write",
                json!({ "filename": filename, "contents": "x" }),
            )
            .await;
        assert!(
            err_message(&refused_write, "workspace_file_write")
                .contains("is not an editable workspace file"),
            "writing '{filename}' must be refused by the allowlist"
        );

        let refused_reset = harness
            .rpc(
                id + 200,
                "openhuman.workspace_file_reset",
                json!({ "filename": filename }),
            )
            .await;
        assert!(
            err_message(&refused_reset, "workspace_file_reset")
                .contains("is not an editable workspace file"),
            "resetting '{filename}' must be refused by the allowlist"
        );
    }

    // A missing required param is a distinct failure from a rejected one.
    let no_filename = harness
        .rpc(68_010, "openhuman.workspace_file_read", json!({}))
        .await;
    assert!(
        err_message(&no_filename, "workspace_file_read no filename")
            .contains("missing required param 'filename'"),
        "an omitted filename must be reported as missing, not as non-editable"
    );

    let no_contents = harness
        .rpc(
            68_011,
            "openhuman.workspace_file_write",
            json!({ "filename": "SOUL.md" }),
        )
        .await;
    assert!(
        err_message(&no_contents, "workspace_file_write no contents")
            .contains("missing required param 'contents'"),
        "an omitted body must be reported as missing"
    );

    // The 256 KiB cap bounds a runaway paste from the UI.
    let oversize = "x".repeat(256 * 1024 + 1);
    let refused = harness
        .rpc(
            68_012,
            "openhuman.workspace_file_write",
            json!({ "filename": "SOUL.md", "contents": oversize }),
        )
        .await;
    assert!(
        err_message(&refused, "workspace_file_write oversize").contains("exceed the"),
        "a write over the byte limit must be refused"
    );
    assert!(
        !harness.workspace().join("SOUL.md").exists(),
        "a refused oversize write must not have touched the file"
    );

    harness.join.abort();
}

// ── modules ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn modules_list_and_status_report_every_known_module() {
    let _lock = env_lock();
    let harness = setup(None).await;

    let listed = harness
        .rpc(69_001, "openhuman.modules_list", json!({}))
        .await;
    let modules = payload(&listed, "modules_list")
        .get("modules")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("modules must be an array: {listed}"))
        .clone();
    assert!(
        !modules.is_empty(),
        "this build knows at least one loadable module: {listed}"
    );
    let ids: Vec<&str> = modules
        .iter()
        .filter_map(|m| m.get("id").and_then(Value::as_str))
        .collect();
    for expected in ["tinydocs", "tinymemory", "tinymcp"] {
        assert!(
            ids.contains(&expected),
            "the compiled-in registry must list '{expected}': {ids:?}"
        );
    }
    // Every entry carries the fields the settings screen renders.
    for module in &modules {
        for field in ["id", "description", "version", "bus_name", "state"] {
            assert!(
                module.get(field).is_some(),
                "module status is missing `{field}`: {module}"
            );
        }
    }
    // The test config disables modules, so every entry must say so — and say
    // *why*, since "unavailable" without a reason is the failure mode this
    // namespace exists to avoid.
    for module in &modules {
        assert_eq!(
            module.get("state"),
            Some(&json!("unsupported")),
            "with `[modules] enabled = false` every module is unsupported: {module}"
        );
        assert_eq!(
            module.get("detail"),
            Some(&json!("modules are disabled in configuration")),
            "the reason must be reported, not just the state: {module}"
        );
    }

    // status for one id returns exactly that module.
    let status = harness
        .rpc(
            69_002,
            "openhuman.modules_status",
            json!({ "id": "tinydocs" }),
        )
        .await;
    let module = payload(&status, "modules_status")
        .get("module")
        .unwrap_or_else(|| panic!("status must return a module: {status}"));
    assert_eq!(module.get("id"), Some(&json!("tinydocs")));
    assert_eq!(module.get("state"), Some(&json!("unsupported")));

    harness.join.abort();
}

#[tokio::test]
async fn modules_status_and_load_reject_an_unknown_id_and_a_blank_one() {
    let _lock = env_lock();
    let harness = setup(None).await;

    for (id, method) in [
        (70_001, "openhuman.modules_status"),
        (70_002, "openhuman.modules_load"),
    ] {
        let unknown = harness.rpc(id, method, json!({ "id": "tinynope" })).await;
        assert!(
            err_message(&unknown, method).contains("unknown module 'tinynope'"),
            "{method} must name the unknown module in its error"
        );

        // Omitted entirely: caught at the dispatch boundary by the schema's
        // `required` flag, before the handler runs.
        let missing = harness.rpc(id + 10, method, json!({})).await;
        assert!(
            err_message(&missing, method).contains("missing required param 'id'"),
            "{method} must require an id"
        );

        let blank = harness.rpc(id + 20, method, json!({ "id": "   " })).await;
        assert!(
            err_message(&blank, method).contains("`id` is required"),
            "{method} must treat a blank id as missing"
        );
    }

    // `modules_load` on a known module reports the *state after the attempt*
    // rather than failing the transport — with modules disabled the attempt
    // cannot succeed, and the answer is a state plus a reason. (Downloads stay
    // off: `[modules] enabled = false` short-circuits before any network I/O.)
    let attempted = harness
        .rpc(
            70_003,
            "openhuman.modules_load",
            json!({ "id": "tinydocs" }),
        )
        .await;
    let module = payload(&attempted, "modules_load disabled")
        .get("module")
        .unwrap_or_else(|| panic!("load must return a module status: {attempted}"));
    assert_eq!(module.get("id"), Some(&json!("tinydocs")));
    assert_eq!(
        module.get("state"),
        Some(&json!("unsupported")),
        "a load that could not run must not report ready: {module}"
    );
    assert_eq!(
        module.get("detail"),
        Some(&json!("modules are disabled in configuration"))
    );

    harness.join.abort();
}
