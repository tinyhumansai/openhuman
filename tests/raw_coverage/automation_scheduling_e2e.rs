//! JSON-RPC E2E coverage for the automation/scheduling controllers that no
//! e2e target reached: `cron_remove` / `cron_run` / `cron_runs`,
//! `task_sources_sync` / `task_sources_list_databases`, the whole `hooks`
//! namespace, and `harness_init_run`.
//!
//! Every case boots the real Axum JSON-RPC router over HTTP against an
//! isolated `HOME` and asserts on the **content** of the response. Nothing
//! here reaches the network: `api_url` points at a closed port, and the
//! provisioning switches harness-init would act on are asserted OFF in the
//! parsed config before a single step runs.
//!
//! Aggregated into `tests/raw_coverage_all.rs` by `build.rs`. Run with:
//! `cargo test --test raw_coverage_all --features "$(bash scripts/ci/product-features.sh)" automation_scheduling_e2e`

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::http::header::AUTHORIZATION;
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::core::auth::{get_rpc_token, init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;

/// Seeded only if this suite is the first in the aggregated binary to
/// initialise the token; the bearer actually sent is always read back from
/// `get_rpc_token()`, because `RPC_TOKEN` is a process-global `OnceLock` and
/// whichever aggregated suite calls `init_rpc_token` first wins it for all.
const TEST_RPC_TOKEN: &str = "automation-scheduling-e2e-token";

static AUTH_INIT: OnceLock<()> = OnceLock::new();

/// The crate-wide env lock, not a private one. Every aggregated suite in
/// `raw_coverage_all` shares one process, so libtest runs them concurrently
/// and a lock local to this file would isolate nothing.
static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, path.as_os_str());
        Self { key, old }
    }

    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
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

/// Initialise the process RPC token (idempotent) and return the bearer the
/// router will actually accept.
fn ensure_rpc_auth() -> &'static str {
    AUTH_INIT.get_or_init(|| {
        if get_rpc_token().is_none() {
            std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN);
        }
        let token_dir = std::env::temp_dir().join("openhuman-automation-scheduling-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
    get_rpc_token().expect("rpc token initialized")
}

async fn serve_rpc() -> (
    SocketAddr,
    &'static str,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    let token = ensure_rpc_auth();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc listener");
    let addr = listener.local_addr().expect("rpc listener addr");
    let join =
        tokio::spawn(async move { axum::serve(listener, build_core_http_router(false)).await });
    (addr, token, join)
}

/// The config every case here runs against.
///
/// `[node] enabled = false` and `[runtime_python] enabled = false` are load-
/// bearing: `harness_init_run` would otherwise **download** a managed Node.js
/// and CPython. `Config` does not `deny_unknown_fields`, so a mistyped table
/// name would be silently ignored and the download would happen anyway —
/// [`assert_provisioning_is_disabled`] is the guard against exactly that.
const TEST_CONFIG_TOML: &str = r#"api_url = "http://127.0.0.1:9"
default_model = "automation-e2e-model"
default_temperature = 0.2
chat_onboarding_completed = true

[secrets]
encrypt = false

[local_ai]
enabled = false

[node]
enabled = false

[runtime_python]
enabled = false

[memory]
provider = "none"
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0

[memory_tree]
embedding_strict = false
spacy_enabled = false
"#;

/// Prove the disable switches actually bound to the fields harness-init reads,
/// rather than being silently dropped as unknown keys.
fn assert_provisioning_is_disabled() -> openhuman_core::openhuman::config::Config {
    let parsed: openhuman_core::openhuman::config::Config =
        toml::from_str(TEST_CONFIG_TOML).expect("test config must match the Config schema");
    assert!(
        !parsed.node.enabled,
        "[node] enabled=false must bind — otherwise harness_init downloads Node.js"
    );
    assert!(
        !parsed.runtime_python.enabled,
        "[runtime_python] enabled=false must bind — otherwise harness_init downloads CPython"
    );
    assert!(
        !parsed.memory_tree.spacy_enabled,
        "[memory_tree] spacy_enabled=false must bind — otherwise harness_init provisions spaCy"
    );
    parsed
}

fn write_min_config(openhuman_dir: &Path) {
    let write = |dir: &Path| {
        std::fs::create_dir_all(dir).expect("create config dir");
        std::fs::write(dir.join("config.toml"), TEST_CONFIG_TOML).expect("write config.toml");
    };
    write(openhuman_dir);
    // Runtime config resolution is user-scoped before login, so the pre-login
    // `users/local` layer needs the same file or the RPC handlers load defaults.
    write(&openhuman_dir.join("users").join("local"));
    assert_provisioning_is_disabled();
}

struct Harness {
    tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    rpc_base: String,
    token: &'static str,
    join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

impl Harness {
    fn home(&self) -> &Path {
        self.tmp.path()
    }

    async fn rpc(&self, id: i64, method: &str, params: Value) -> Value {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("build rpc client");
        let url = format!("{}/rpc", self.rpc_base);
        let response = client
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {}", self.token))
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

    async fn ok(&self, id: i64, method: &str, params: Value) -> Value {
        let response = self.rpc(id, method, params).await;
        if let Some(error) = response.get("error") {
            panic!("{method}: unexpected JSON-RPC error: {error}");
        }
        let result = response
            .get("result")
            .unwrap_or_else(|| panic!("{method}: missing result: {response}"));
        match result.get("result") {
            Some(inner) if result.get("logs").is_some() => inner.clone(),
            _ => result.clone(),
        }
    }

    async fn err(&self, id: i64, method: &str, params: Value) -> String {
        let response = self.rpc(id, method, params).await;
        let error = response
            .get("error")
            .unwrap_or_else(|| panic!("{method}: expected a JSON-RPC error, got: {response}"));
        error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{method}: error carries no message: {error}"))
            .to_string()
    }
}

async fn setup() -> Harness {
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    write_min_config(&home.join(".openhuman"));

    // A cron shell job spawns `sh` with `current_dir(config.action_dir)`, and a
    // hook's project layer is read from `<action_dir>/.openhuman/hooks.json`.
    // Left at its default that is `~/OpenHuman/projects` — the developer's real
    // directory, which is both a leak and (when absent) an ENOENT that surfaces
    // only as "spawn error: No such file or directory".
    let action_dir = home.join("actions");
    std::fs::create_dir_all(&action_dir).expect("create action dir");

    let guards = vec![
        EnvVarGuard::set_to_path("HOME", &home),
        EnvVarGuard::set_to_path("OPENHUMAN_ACTION_DIR", &action_dir),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        EnvVarGuard::unset("COMPOSIO_API_KEY"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
    ];

    let (addr, token, join) = serve_rpc().await;
    Harness {
        tmp,
        _guards: guards,
        rpc_base: format!("http://{addr}"),
        token,
        join,
    }
}

fn str_at<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("expected a string at {pointer} in {value}"))
}

// ── cron ────────────────────────────────────────────────────────────────────

/// The half of the cron surface no e2e target reached: run-now, its run
/// history, and removal. A shell job is used because it is the only job type
/// that completes without a model.
#[tokio::test]
async fn cron_run_records_history_and_remove_is_not_idempotent() {
    let _lock = env_lock();
    let h = setup().await;

    let job = h
        .ok(
            2001,
            "openhuman.cron_add",
            json!({
                "name": "e2e echo",
                "schedule": { "kind": "every", "every_ms": 3_600_000u64 },
                "job_type": "shell",
                "command": "echo automation-e2e-marker",
            }),
        )
        .await;
    let job_id = str_at(&job, "/id").to_string();
    assert_eq!(job.get("name").and_then(Value::as_str), Some("e2e echo"));

    // `cron_run` enqueues and returns immediately — the run itself is a
    // background task, so the RPC's own answer is the queue acknowledgement.
    let queued = h
        .ok(2002, "openhuman.cron_run", json!({ "job_id": job_id }))
        .await;
    assert_eq!(
        queued.get("job_id").and_then(Value::as_str),
        Some(job_id.as_str()),
        "cron_run echoes the job it enqueued: {queued}"
    );
    assert_eq!(
        queued.get("status").and_then(Value::as_str),
        Some("queued"),
        "cron_run reports the enqueue, not the outcome: {queued}"
    );

    // The queued placeholder is written synchronously, so history is non-empty
    // the instant the RPC returns — that is the property the frontend poller
    // depends on.
    let immediate = h
        .ok(2003, "openhuman.cron_runs", json!({ "job_id": job_id }))
        .await;
    assert!(
        !immediate
            .as_array()
            .expect("cron_runs returns an array")
            .is_empty(),
        "the queued placeholder is recorded before cron_run returns: {immediate}"
    );

    // Wait for the background execution to replace the placeholder with a real
    // result. `echo` is immediate; the generous budget is for a loaded machine.
    let mut settled: Option<Value> = None;
    for _ in 0..120 {
        let runs = h
            .ok(2004, "openhuman.cron_runs", json!({ "job_id": job_id }))
            .await;
        let runs = runs.as_array().expect("cron_runs array").clone();
        if let Some(run) = runs
            .iter()
            .find(|run| run.get("status").and_then(Value::as_str) != Some("queued"))
        {
            settled = Some(run.clone());
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let settled = settled.expect("the shell job must finish and record a terminal run");
    assert_eq!(
        settled.get("status").and_then(Value::as_str),
        Some("ok"),
        "`echo` succeeds: {settled}"
    );
    assert!(
        settled
            .get("output")
            .and_then(Value::as_str)
            .is_some_and(|out| out.contains("automation-e2e-marker")),
        "the run record captures the command's own output: {settled}"
    );
    assert_eq!(
        settled.get("job_id").and_then(Value::as_str),
        Some(job_id.as_str())
    );

    // `limit` is honoured rather than ignored.
    let capped = h
        .ok(
            2005,
            "openhuman.cron_runs",
            json!({ "job_id": job_id, "limit": 1 }),
        )
        .await;
    assert_eq!(
        capped.as_array().map(Vec::len),
        Some(1),
        "cron_runs honours limit: {capped}"
    );

    let removed = h
        .ok(2006, "openhuman.cron_remove", json!({ "job_id": job_id }))
        .await;
    assert_eq!(
        removed.get("job_id").and_then(Value::as_str),
        Some(job_id.as_str())
    );
    assert_eq!(
        removed.get("removed").and_then(Value::as_bool),
        Some(true),
        "removal is reported explicitly: {removed}"
    );

    let listed = h.ok(2007, "openhuman.cron_list", json!({})).await;
    assert!(
        !listed
            .as_array()
            .expect("cron_list array")
            .iter()
            .any(|entry| entry.get("id").and_then(Value::as_str) == Some(job_id.as_str())),
        "the removed job is gone from the list: {listed}"
    );

    // A second removal is an ERROR, not a silent success — the caller is told
    // its id no longer names anything.
    let again = h
        .err(2008, "openhuman.cron_remove", json!({ "job_id": job_id }))
        .await;
    assert!(
        again.contains(&job_id) && again.to_lowercase().contains("not found"),
        "removing an absent job names it: {again}"
    );

    h.join.abort();
}

/// Every cron controller validates its `job_id` before touching the store, and
/// a whitespace-only id must not slip through as a valid one.
#[tokio::test]
async fn cron_controllers_reject_absent_and_blank_job_ids() {
    let _lock = env_lock();
    let h = setup().await;

    for (id, method) in [
        (2101, "openhuman.cron_remove"),
        (2102, "openhuman.cron_run"),
        (2103, "openhuman.cron_runs"),
    ] {
        let blank = h.err(id, method, json!({ "job_id": "   " })).await;
        assert!(
            blank.contains("job_id"),
            "{method} rejects a blank job_id by name: {blank}"
        );

        let absent = h.err(id + 10, method, json!({})).await;
        assert!(
            absent.contains("job_id"),
            "{method} names its required param: {absent}"
        );
    }

    // The three diverge on an id that does not name a job, and the divergence
    // is worth pinning. `remove` and `run` both go through `cron::get_job` /
    // `remove_job` and raise; `runs` queries the run table by id and cannot
    // tell "no such job" from "job with no runs", so a typo reads as an empty
    // history. See `~/tinyhuman/bugs/e2e-wave-cron-run-schema-declares-an-outcome-it-never-returns.md`.
    for (id, method) in [
        (2121, "openhuman.cron_remove"),
        (2122, "openhuman.cron_run"),
    ] {
        let unknown = h
            .err(id, method, json!({ "job_id": "no-such-job" }))
            .await;
        assert!(
            unknown.contains("no-such-job"),
            "{method} names the unknown job: {unknown}"
        );
    }

    let unknown_history = h
        .ok(
            2123,
            "openhuman.cron_runs",
            json!({ "job_id": "no-such-job" }),
        )
        .await;
    assert_eq!(
        unknown_history.as_array().map(Vec::len),
        Some(0),
        "cron_runs cannot distinguish an unknown job from one with no runs: {unknown_history}"
    );

    h.join.abort();
}

// ── task_sources ────────────────────────────────────────────────────────────

/// `task_sources_sync` fans out over every ENABLED source and reports one
/// outcome each, so an empty workspace yields an empty list rather than an
/// error.
///
/// With a source configured the outcome carries the fetch failure instead of
/// raising it — one broken source must not abort the sweep. That failure is
/// currently unconditional: see
/// `~/tinyhuman/bugs/e2e-wave-task_sources-fetch-pipeline-unavailable.md`.
#[tokio::test]
async fn task_sources_sync_reports_one_outcome_per_enabled_source() {
    let _lock = env_lock();
    let h = setup().await;

    let empty = h.ok(2201, "openhuman.task_sources_sync", json!({})).await;
    assert_eq!(
        empty.as_array().map(Vec::len),
        Some(0),
        "no sources configured means no outcomes, not an error: {empty}"
    );

    let source = h
        .ok(
            2202,
            "openhuman.task_sources_add",
            json!({
                "provider": "github",
                "name": "e2e source",
                "filter": {
                    "provider": "github",
                    "repo": "tinyhumansai/openhuman",
                    "labels": ["bug"],
                },
            }),
        )
        .await;
    let source_id = str_at(&source, "/id").to_string();
    assert_eq!(
        source.get("enabled").and_then(Value::as_bool),
        Some(true),
        "a new source is enabled, so sync must pick it up: {source}"
    );

    let synced = h.ok(2203, "openhuman.task_sources_sync", json!({})).await;
    let outcomes = synced.as_array().expect("sync returns an outcomes array");
    assert_eq!(outcomes.len(), 1, "one outcome per enabled source: {synced}");
    let outcome = &outcomes[0];
    assert_eq!(
        outcome.get("sourceId").and_then(Value::as_str),
        Some(source_id.as_str()),
        "the outcome names the source it swept: {outcome}"
    );
    assert_eq!(
        outcome.get("provider").and_then(Value::as_str),
        Some("github")
    );
    assert_eq!(outcome.get("fetched").and_then(Value::as_u64), Some(0));
    assert_eq!(outcome.get("routed").and_then(Value::as_u64), Some(0));
    let error = outcome
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("a failing fetch is carried on the outcome: {outcome}"));
    assert!(
        error.contains("unavailable"),
        "the fetch failure is reported per-source, not raised: {error}"
    );

    // A disabled source is skipped entirely rather than reported as a failure.
    // `update` takes a partial-patch object, not loose fields.
    let disabled = h
        .ok(
            2204,
            "openhuman.task_sources_update",
            json!({ "id": source_id, "patch": { "enabled": false } }),
        )
        .await;
    assert_eq!(
        disabled.get("enabled").and_then(Value::as_bool),
        Some(false),
        "the patch is applied and echoed: {disabled}"
    );
    let after = h.ok(2205, "openhuman.task_sources_sync", json!({})).await;
    assert_eq!(
        after.as_array().map(Vec::len),
        Some(0),
        "a disabled source contributes no outcome: {after}"
    );

    h.join.abort();
}

/// `task_sources_list_databases` promises a `databases` array but cannot
/// deliver one for any provider — the underlying `ComposioProvider::
/// list_databases` was deleted upstream with no replacement. The controller is
/// reachable and returns a diagnosable error rather than an empty list, which
/// is what this pins.
///
/// Documented at
/// `~/tinyhuman/bugs/e2e-wave-task_sources-fetch-pipeline-unavailable.md`.
#[tokio::test]
async fn task_sources_list_databases_is_unavailable_for_every_provider() {
    let _lock = env_lock();
    let h = setup().await;

    for (id, provider) in [
        (2301, "notion"),
        (2302, "github"),
        (2303, "linear"),
        (2304, "clickup"),
    ] {
        let error = h
            .err(
                id,
                "openhuman.task_sources_list_databases",
                json!({ "provider": provider }),
            )
            .await;
        assert!(
            error.contains(provider) && error.contains("unavailable"),
            "list_databases names the toolkit it cannot serve: {error}"
        );
    }

    let unknown = h
        .err(
            2305,
            "openhuman.task_sources_list_databases",
            json!({ "provider": "jira" }),
        )
        .await;
    assert!(
        unknown.to_lowercase().contains("jira") || unknown.to_lowercase().contains("provider"),
        "an unsupported provider is rejected before the unavailability: {unknown}"
    );

    let missing = h
        .err(2306, "openhuman.task_sources_list_databases", json!({}))
        .await;
    assert!(
        missing.contains("provider"),
        "the required `provider` param is named: {missing}"
    );

    h.join.abort();
}

// ── hooks ───────────────────────────────────────────────────────────────────

/// Write a `hooks.json` into the user layer (`~/.openhuman/hooks.json`) with a
/// single `beforeShellExecution` hook that denies anything matching `^rm `.
fn write_user_hooks(home: &Path) {
    let hooks = json!({
        "version": 1,
        "hooks": {
            "beforeShellExecution": [
                {
                    "command": r#"printf '{"permission":"deny","agent_message":"blocked by the e2e hook"}'"#,
                    "matcher": "^rm ",
                    "timeout": 15
                }
            ]
        }
    });
    let dir = home.join(".openhuman");
    std::fs::create_dir_all(&dir).expect("create .openhuman");
    std::fs::write(
        dir.join("hooks.json"),
        serde_json::to_string_pretty(&hooks).expect("serialize hooks.json"),
    )
    .expect("write hooks.json");
}

/// The whole `hooks` namespace: `reload` re-reads the layer files, `list`
/// answers "what is configured and where did it come from", and `test` fires
/// one synthetic event so an author can see the verdict without provoking the
/// real moment.
#[tokio::test]
async fn hooks_reload_list_and_test_round_trip_a_deny_rule() {
    let _lock = env_lock();
    let h = setup().await;

    // Before the file exists, reload must produce an empty, warning-free config
    // — a host with no hooks is the common case, not a misconfiguration.
    let bare = h.ok(2401, "openhuman.hooks_reload", json!({})).await;
    assert_eq!(
        bare.pointer("/hooks/total").and_then(Value::as_u64),
        Some(0),
        "no hooks.json anywhere means an empty config: {bare}"
    );
    assert_eq!(
        bare.pointer("/hooks/warnings").and_then(Value::as_array).map(Vec::len),
        Some(0),
        "a missing file is not a warning: {bare}"
    );

    write_user_hooks(h.home());

    // `list` reads the in-memory snapshot, so it must still be empty until a
    // reload swaps the new file in. That is the distinction between the two.
    let stale = h.ok(2402, "openhuman.hooks_list", json!({})).await;
    assert_eq!(
        stale.pointer("/hooks/total").and_then(Value::as_u64),
        Some(0),
        "list reports the live snapshot, not the files on disk: {stale}"
    );

    let reloaded = h.ok(2403, "openhuman.hooks_reload", json!({})).await;
    assert_eq!(
        reloaded.pointer("/hooks/total").and_then(Value::as_u64),
        Some(1),
        "reload picked up the new user-layer hook: {reloaded}"
    );
    let sources = reloaded
        .pointer("/hooks/sources")
        .and_then(Value::as_array)
        .expect("sources array");
    assert!(
        sources
            .iter()
            .filter_map(Value::as_str)
            .any(|source| source.ends_with("hooks.json")),
        "the config names the file every definition came from: {sources:?}"
    );

    let definition = reloaded
        .pointer("/hooks/by_event/beforeShellExecution/definitions/0")
        .unwrap_or_else(|| panic!("the hook is filed under its event: {reloaded}"));
    assert_eq!(
        definition.get("matcher").and_then(Value::as_str),
        Some("^rm ")
    );
    assert_eq!(definition.get("timeout").and_then(Value::as_u64), Some(15));
    assert_eq!(
        definition.get("layer").and_then(Value::as_str),
        Some("user"),
        "the layer is resolved from the file's location, never read from it: {definition}"
    );
    assert_eq!(
        definition.get("enabled").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        definition.get("fail_closed").and_then(Value::as_bool),
        Some(false),
        "fail_closed is opt-in, so a broken audit script cannot brick the agent: {definition}"
    );

    // `list` now agrees with the reload — the snapshot was swapped, not copied.
    let listed = h.ok(2404, "openhuman.hooks_list", json!({})).await;
    assert_eq!(
        listed.pointer("/hooks/total").and_then(Value::as_u64),
        Some(1),
        "list reflects the reloaded config: {listed}"
    );

    // A command that matches: the hook runs and its verdict is the decision.
    let denied = h
        .ok(
            2405,
            "openhuman.hooks_test",
            json!({
                "event": "beforeShellExecution",
                "payload": { "command": "rm -rf /", "sandbox": false },
            }),
        )
        .await;
    assert_eq!(
        denied.pointer("/result/event").and_then(Value::as_str),
        Some("beforeShellExecution")
    );
    assert_eq!(
        denied.pointer("/result/decision/permission").and_then(Value::as_str),
        Some("deny"),
        "the matching hook's verdict is the merged decision: {denied}"
    );
    assert_eq!(
        denied
            .pointer("/result/decision/agent_message")
            .and_then(Value::as_str),
        Some("blocked by the e2e hook"),
        "the model-facing reason survives the round trip: {denied}"
    );
    let runs = denied
        .pointer("/result/runs")
        .and_then(Value::as_array)
        .expect("runs array");
    assert_eq!(runs.len(), 1, "exactly the one matching hook ran: {denied}");
    assert!(
        runs[0].get("error").is_none_or(Value::is_null),
        "the hook ran cleanly: {denied}"
    );

    // A command that does NOT match the matcher: the hook is not run at all.
    let allowed = h
        .ok(
            2406,
            "openhuman.hooks_test",
            json!({
                "event": "beforeShellExecution",
                "payload": { "command": "ls -la", "sandbox": false },
            }),
        )
        .await;
    assert_eq!(
        allowed.pointer("/result/runs").and_then(Value::as_array).map(Vec::len),
        Some(0),
        "the matcher gates which occurrences reach the hook: {allowed}"
    );
    assert!(
        allowed
            .pointer("/result/decision/permission")
            .is_none_or(Value::is_null),
        "no hook ran, so nothing was decided: {allowed}"
    );

    // A different event the hook is not registered for is likewise untouched.
    let other_event = h
        .ok(
            2407,
            "openhuman.hooks_test",
            json!({ "event": "sessionStart", "payload": { "entrypoint": "e2e" } }),
        )
        .await;
    assert_eq!(
        other_event.pointer("/result/runs").and_then(Value::as_array).map(Vec::len),
        Some(0),
        "an event with no hooks configured runs nothing: {other_event}"
    );

    // Teardown, and an assertion in its own right. The hook engine and the
    // harness bridge are PROCESS-global: a loaded config here would stay
    // installed for every other suite in the aggregated binary, and a deny rule
    // pointing at this test's since-deleted tempdir would then judge their
    // shell calls. Removing the file and reloading must empty the engine again
    // — which is also the only way to observe that `reload` *unloads*, not just
    // loads.
    std::fs::remove_file(h.home().join(".openhuman").join("hooks.json"))
        .expect("remove hooks.json");
    let unloaded = h.ok(2408, "openhuman.hooks_reload", json!({})).await;
    assert_eq!(
        unloaded.pointer("/hooks/total").and_then(Value::as_u64),
        Some(0),
        "reload unloads a removed layer rather than keeping the last good one: {unloaded}"
    );

    h.join.abort();
}

/// `hooks_test` refuses an event name it does not know, and says which names it
/// does — the endpoint exists so an author never has to guess.
#[tokio::test]
async fn hooks_test_rejects_unknown_events_and_malformed_payloads() {
    let _lock = env_lock();
    let h = setup().await;

    let unknown = h
        .err(
            2501,
            "openhuman.hooks_test",
            json!({ "event": "beforeLaunchingRockets" }),
        )
        .await;
    assert!(
        unknown.contains("beforeLaunchingRockets"),
        "the rejection quotes the name given: {unknown}"
    );
    assert!(
        unknown.contains("beforeShellExecution") && unknown.contains("preToolUse"),
        "and lists the known events so the author can correct it: {unknown}"
    );

    // The payload is parsed against the event's own body type, so a shell
    // payload missing its required `command` is a typed error, not a panic.
    let malformed = h
        .err(
            2502,
            "openhuman.hooks_test",
            json!({ "event": "beforeShellExecution", "payload": { "sandbox": false } }),
        )
        .await;
    assert!(
        malformed.contains("beforeShellExecution") && malformed.contains("payload"),
        "the payload error names the event it failed to parse for: {malformed}"
    );

    let no_event = h.err(2503, "openhuman.hooks_test", json!({})).await;
    assert!(
        no_event.contains("event"),
        "the required `event` param is named: {no_event}"
    );

    h.join.abort();
}

/// A malformed `hooks.json` is a **warning**, not a silent drop: the author
/// believes that file is running, so `list` must say why it is not.
#[tokio::test]
async fn hooks_reload_surfaces_a_malformed_file_as_a_warning() {
    let _lock = env_lock();
    let h = setup().await;

    let dir = h.home().join(".openhuman");
    std::fs::create_dir_all(&dir).expect("create .openhuman");
    std::fs::write(dir.join("hooks.json"), "{ this is not json").expect("write bad hooks.json");

    let reloaded = h.ok(2601, "openhuman.hooks_reload", json!({})).await;
    assert_eq!(
        reloaded.pointer("/hooks/total").and_then(Value::as_u64),
        Some(0),
        "an unparseable file contributes no definitions: {reloaded}"
    );
    let warnings = reloaded
        .pointer("/hooks/warnings")
        .and_then(Value::as_array)
        .expect("warnings array");
    assert!(
        warnings
            .iter()
            .filter_map(Value::as_str)
            .any(|warning| warning.contains("hooks.json")),
        "the warning names the file that failed to load: {warnings:?}"
    );

    h.join.abort();
}

// ── harness_init ────────────────────────────────────────────────────────────

/// `harness_init_run` is the retry behind the first-run setup overlay. With
/// every provisioning backend switched off, each step's cheap probe reports it
/// satisfied and the run must settle `done` **without downloading anything**.
///
/// `force` is the interesting parameter: it must bypass the probe and actually
/// invoke each step, which is observable in the per-step message — the
/// probe-satisfied path stamps "already provisioned" and the forced path does
/// not.
#[tokio::test]
async fn harness_init_run_completes_offline_and_force_bypasses_the_probes() {
    let _lock = env_lock();
    let h = setup().await;

    let snapshot = h
        .ok(2701, "openhuman.harness_init_run", json!({}))
        .await;
    let steps = snapshot
        .pointer("/snapshot/steps")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("the snapshot carries a steps array: {snapshot}"));
    assert_eq!(
        snapshot.pointer("/snapshot/overall").and_then(Value::as_str),
        Some("done"),
        "no required step can fail when every backend is off: {snapshot}"
    );

    let step_ids: Vec<&str> = steps.iter().map(|step| str_at(step, "/id")).collect();
    for expected in [
        "python_runtime",
        "spacy",
        "kompress",
        "runtime_python_server",
    ] {
        assert!(
            step_ids.contains(&expected),
            "the registry step {expected} must appear in the snapshot, got {step_ids:?}"
        );
    }

    for step in steps {
        assert_eq!(
            step.get("state").and_then(Value::as_str),
            Some("done"),
            "every disabled step probes satisfied: {step}"
        );
        assert_eq!(
            step.get("message").and_then(Value::as_str),
            Some("already provisioned"),
            "the unforced path marks Done from the probe, without running: {step}"
        );
        assert_eq!(step.get("percent").and_then(Value::as_u64), Some(100));
        assert_eq!(
            step.get("required").and_then(Value::as_bool),
            Some(false),
            "every registered step is non-required today: {step}"
        );
        assert!(
            step.get("updated_at").and_then(Value::as_str).is_some(),
            "each state change is stamped: {step}"
        );
    }

    let forced = h
        .ok(2702, "openhuman.harness_init_run", json!({ "force": true }))
        .await;
    assert_eq!(
        forced.pointer("/snapshot/overall").and_then(Value::as_str),
        Some("done"),
        "a forced re-run of disabled steps still settles done: {forced}"
    );
    let forced_steps = forced
        .pointer("/snapshot/steps")
        .and_then(Value::as_array)
        .expect("forced steps array");
    for step in forced_steps {
        assert_eq!(
            step.get("state").and_then(Value::as_str),
            Some("done"),
            "a forced disabled step returns Ok immediately: {step}"
        );
        assert!(
            step.get("message").and_then(Value::as_str) != Some("already provisioned"),
            "force must bypass the probe, so the probe's message cannot appear: {step}"
        );
    }
    assert!(
        forced.pointer("/snapshot/finished_at").and_then(Value::as_str).is_some(),
        "a finished run is stamped: {forced}"
    );

    // `harness_init_status` must report the same snapshot the run just left
    // behind, not recompute one.
    let status = h.ok(2703, "openhuman.harness_init_status", json!({})).await;
    assert_eq!(
        status.pointer("/snapshot/overall").and_then(Value::as_str),
        Some("done"),
        "status reads the store the run wrote: {status}"
    );
    assert_eq!(
        status.pointer("/snapshot/steps").and_then(Value::as_array).map(Vec::len),
        forced_steps.len().into(),
        "status and run agree on the step list: {status}"
    );

    h.join.abort();
}
