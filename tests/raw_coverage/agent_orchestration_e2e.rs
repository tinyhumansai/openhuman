//! JSON-RPC E2E coverage for the agent-orchestration controllers that no e2e
//! target reached: durable workflow-run `stop` / `resume`, the command
//! center's `agent_work_control`, `agent_team_list` / `agent_team_close`, the
//! detached sub-agent controls (`subagent_cancel` / `subagent_steer`), and the
//! whole `agent_experience` store.
//!
//! Every case boots the real Axum JSON-RPC router over HTTP against an
//! isolated `HOME` and asserts on the **content** of the response. Nothing
//! here needs a model: the durable ledger writes, the control-verb validation
//! and the sub-agent registry lookups are all reachable offline, and `api_url`
//! points at a closed port so a spawned engine loop fails fast instead of
//! reaching the network.
//!
//! Aggregated into `tests/raw_coverage_all.rs` by `build.rs`. Run with:
//! `cargo test --test raw_coverage_all --features "$(bash scripts/ci/product-features.sh)" agent_orchestration_e2e`

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
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
const TEST_RPC_TOKEN: &str = "agent-orchestration-e2e-token";

/// The one builtin workflow definition (`ops::PARALLEL_RESEARCH_ID`).
const BUILTIN_WORKFLOW_ID: &str = "parallel_research_cross_check";

static AUTH_INIT: OnceLock<()> = OnceLock::new();
static MEMORY_SEAMS_INIT: OnceLock<()> = OnceLock::new();

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
        let token_dir = std::env::temp_dir().join("openhuman-agent-orchestration-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
    get_rpc_token().expect("rpc token initialized")
}

/// The transport-only JSON-RPC router builds no core runtime context, so
/// neither the memory host seams nor the **module host policy** a
/// driver-backed controller (`agent_experience`) needs are installed by booting
/// it. Without the policy every call fails with "the module host policy was
/// never published, so module 'tinymemory' cannot be loaded".
///
/// Both are installed on a thread with a stack of its own: `Config::default()`
/// is large enough to overflow the 2 MiB libtest stack if materialised inside
/// an async test frame.
///
/// `MODULES_POLICY` is a process-global `OnceLock` and several other aggregated
/// suites publish their own, so **this may lose the race** — `set_modules_policy`
/// silently ignores a later call, and the loaded module keeps whichever
/// workspace won. The experience cases below are therefore written not to
/// depend on owning the store: they use ids unique to this suite and assert on
/// their own records rather than on the store being empty.
fn ensure_memory_seams() {
    MEMORY_SEAMS_INIT.get_or_init(|| {
        std::thread::Builder::new()
            .name("agent-orchestration-e2e-memory-seams".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                let workspace = tempfile::tempdir()
                    .expect("module workspace tempdir")
                    .keep()
                    .join("workspace");
                std::fs::create_dir_all(&workspace).expect("create module workspace");
                let config = Arc::new(openhuman_core::openhuman::config::Config {
                    workspace_dir: workspace,
                    ..openhuman_core::openhuman::config::Config::default()
                });
                #[cfg(feature = "modules")]
                openhuman_core::openhuman::modules::memory::set_modules_policy(config);
            })
            .expect("spawn agent orchestration e2e seam installer")
            .join()
            .expect("agent orchestration e2e seam installer panicked");
    });
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

/// `api_url` points at a closed port on purpose: a workflow run's spawned
/// engine loop must fail fast rather than reach the network.
fn write_min_config(openhuman_dir: &Path) {
    let cfg = r#"api_url = "http://127.0.0.1:9"
default_model = "orchestration-e2e-model"
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

[memory_tree]
embedding_strict = false
spacy_enabled = false
"#;
    let write = |dir: &Path| {
        std::fs::create_dir_all(dir).expect("create config dir");
        std::fs::write(dir.join("config.toml"), cfg).expect("write config.toml");
    };
    write(openhuman_dir);
    // Runtime config resolution is user-scoped before login, so the pre-login
    // `users/local` layer needs the same file or the RPC handlers load defaults.
    write(&openhuman_dir.join("users").join("local"));
    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(cfg).expect("test config must match the Config schema");
}

struct Harness {
    _tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    rpc_base: String,
    token: &'static str,
    join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

impl Harness {
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

    // Left at its default, `action_dir` is `~/OpenHuman/projects` — the
    // developer's real directory. Pin it inside the tempdir.
    let action_dir = home.join("actions");
    std::fs::create_dir_all(&action_dir).expect("create action dir");

    let guards = vec![
        EnvVarGuard::set_to_path("HOME", &home),
        EnvVarGuard::set_to_path("OPENHUMAN_ACTION_DIR", &action_dir),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
    ];

    let (addr, token, join) = serve_rpc().await;
    Harness {
        _tmp: tmp,
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

// ── workflow_run: stop / resume ─────────────────────────────────────────────

/// Stop marks a live run **interrupted** and aborts its children; resume picks
/// it back up from the first incomplete phase. Neither is a no-op, and the
/// phase ledger must survive both — that is what makes a resume a resume
/// rather than a restart.
///
/// The engine loop this starts cannot reach a model (`api_url` is a closed
/// port), so the run settles quickly on its own. The assertion that bites
/// either way is that after a stop the run is **not** `running`.
#[tokio::test]
async fn workflow_run_stop_then_resume_preserves_the_phase_ledger() {
    let _lock = env_lock();
    let h = setup().await;

    let started = h
        .ok(
            3001,
            "openhuman.workflow_run_start",
            json!({
                "definitionId": BUILTIN_WORKFLOW_ID,
                "input": { "question": "what does stop do?" },
                "parentThreadId": "thread-orchestration-e2e",
            }),
        )
        .await;
    let run = started
        .get("workflowRun")
        .unwrap_or_else(|| panic!("start returns the created run: {started}"));
    let run_id = str_at(run, "/id").to_string();
    assert!(
        run_id.starts_with("wfrun-"),
        "a durable run id is namespaced: {run_id}"
    );
    assert_eq!(
        run.get("definitionId").and_then(Value::as_str),
        Some(BUILTIN_WORKFLOW_ID)
    );
    assert_eq!(
        run.get("status").and_then(Value::as_str),
        Some("running"),
        "start returns the run already Running: {started}"
    );
    assert_eq!(
        run.get("parentThreadId").and_then(Value::as_str),
        Some("thread-orchestration-e2e"),
        "lineage is recorded: {started}"
    );
    let phase_states = run
        .get("phaseStates")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("the run seeds a phase ledger: {started}"));
    for phase in ["decompose", "research", "cross_check", "synthesize"] {
        assert_eq!(
            phase_states
                .get(phase)
                .and_then(|state| state.get("status"))
                .and_then(Value::as_str),
            Some("pending"),
            "every phase starts pending: {started}"
        );
    }

    let stopped = h
        .ok(3002, "openhuman.workflow_run_stop", json!({ "id": run_id }))
        .await;
    let stopped_run = stopped
        .get("workflowRun")
        .filter(|run| !run.is_null())
        .unwrap_or_else(|| panic!("stopping a known run returns it: {stopped}"));
    assert_eq!(str_at(stopped_run, "/id"), run_id);
    let stopped_status = str_at(stopped_run, "/status").to_string();
    assert_ne!(
        stopped_status, "running",
        "after a stop the run is no longer running: {stopped}"
    );

    // The stop is persisted, not just returned.
    let fetched = h
        .ok(3003, "openhuman.workflow_run_get", json!({ "id": run_id }))
        .await;
    let fetched_run = fetched
        .get("workflowRun")
        .filter(|run| !run.is_null())
        .unwrap_or_else(|| panic!("the run is readable after the stop: {fetched}"));
    assert_ne!(
        str_at(fetched_run, "/status"),
        "running",
        "the interrupt was written to the ledger: {fetched}"
    );

    // Resume is refused only for a COMPLETED run. Offline the loop cannot have
    // completed, so this run must be resumable — assert that premise rather
    // than assume it.
    assert_ne!(
        str_at(fetched_run, "/status"),
        "completed",
        "an offline run cannot complete; the resume below assumes it did not: {fetched}"
    );

    let resumed = h
        .ok(3004, "openhuman.workflow_run_resume", json!({ "id": run_id }))
        .await;
    let resumed_run = resumed
        .get("workflowRun")
        .unwrap_or_else(|| panic!("resume returns the run: {resumed}"));
    assert_eq!(str_at(resumed_run, "/id"), run_id, "the SAME run resumed");
    assert_eq!(
        resumed_run.get("status").and_then(Value::as_str),
        Some("running"),
        "resume puts the run back to Running: {resumed}"
    );
    assert_eq!(
        resumed_run
            .get("phaseStates")
            .and_then(Value::as_object)
            .map(|states| states.len()),
        Some(4),
        "resume carries the phase ledger forward rather than reseeding it: {resumed}"
    );
    assert_eq!(
        resumed_run.get("definitionId").and_then(Value::as_str),
        Some(BUILTIN_WORKFLOW_ID)
    );

    // Leave no live engine loop behind for the next case.
    h.ok(3005, "openhuman.workflow_run_stop", json!({ "id": run_id }))
        .await;

    h.join.abort();
}

/// The two controllers disagree on purpose about an unknown id: `stop` is
/// idempotent (nothing to stop is not a failure) while `resume` is not (there
/// is nothing to resume, and the caller asked for something specific).
#[tokio::test]
async fn workflow_run_stop_is_idempotent_where_resume_is_not() {
    let _lock = env_lock();
    let h = setup().await;

    let stopped = h
        .ok(
            3101,
            "openhuman.workflow_run_stop",
            json!({ "id": "wfrun-does-not-exist" }),
        )
        .await;
    assert!(
        stopped
            .get("workflowRun")
            .is_some_and(Value::is_null),
        "stopping an unknown run yields a null run, not an error: {stopped}"
    );

    let resume_error = h
        .err(
            3102,
            "openhuman.workflow_run_resume",
            json!({ "id": "wfrun-does-not-exist" }),
        )
        .await;
    assert!(
        resume_error.contains("wfrun-does-not-exist") && resume_error.contains("unknown"),
        "resume names the run it could not find: {resume_error}"
    );

    for (id, method) in [
        (3103, "openhuman.workflow_run_stop"),
        (3104, "openhuman.workflow_run_resume"),
    ] {
        let missing = h.err(id, method, json!({})).await;
        assert!(
            missing.contains("id"),
            "{method} names its required param: {missing}"
        );
    }

    let unknown_definition = h
        .err(
            3105,
            "openhuman.workflow_run_start",
            json!({ "definitionId": "no-such-definition" }),
        )
        .await;
    assert!(
        unknown_definition.contains("no-such-definition"),
        "start names the definition it could not resolve: {unknown_definition}"
    );

    h.join.abort();
}

// ── agent_work_control ──────────────────────────────────────────────────────

/// `agent_work_control` is the command center's only write. Its guards run in a
/// deliberate order — the message requirement is checked BEFORE the run is
/// looked up — so a `continue` with no message is rejected on its own terms
/// rather than as a missing run.
#[tokio::test]
async fn agent_work_control_validates_verb_and_message_before_touching_the_ledger() {
    let _lock = env_lock();
    let h = setup().await;

    let unknown_verb = h
        .err(
            3201,
            "openhuman.agent_work_control",
            json!({ "runId": "run-1", "action": "detonate" }),
        )
        .await;
    assert!(
        unknown_verb.contains("detonate"),
        "an unknown verb is quoted back: {unknown_verb}"
    );

    // `continue` and `follow_up` both require a message. The run does not
    // exist, so a "run not found" here would prove the guard ran too late.
    for (id, action) in [(3202, "continue"), (3203, "follow_up")] {
        let no_message = h
            .err(
                id,
                "openhuman.agent_work_control",
                json!({ "runId": "run-does-not-exist", "action": action }),
            )
            .await;
        assert!(
            no_message.to_lowercase().contains("message"),
            "{action} without a message is rejected for the message, not the run: {no_message}"
        );
        assert!(
            !no_message.contains("run-does-not-exist"),
            "the message guard runs before the ledger lookup: {no_message}"
        );

        // A whitespace-only message is not a message.
        let blank_message = h
            .err(
                id + 10,
                "openhuman.agent_work_control",
                json!({ "runId": "run-does-not-exist", "action": action, "message": "   " }),
            )
            .await;
        assert!(
            blank_message.to_lowercase().contains("message"),
            "{action} treats a blank message as absent: {blank_message}"
        );
    }

    // `stop` needs no message, so it reaches the ledger and fails there.
    let unknown_run = h
        .err(
            3204,
            "openhuman.agent_work_control",
            json!({ "runId": "run-does-not-exist", "action": "stop" }),
        )
        .await;
    assert!(
        unknown_run.contains("run-does-not-exist"),
        "a verb needing no message reaches the lookup and names the run: {unknown_run}"
    );

    for (id, key) in [(3205, "runId"), (3206, "action")] {
        let params = if key == "runId" {
            json!({ "action": "stop" })
        } else {
            json!({ "runId": "run-1" })
        };
        let missing = h.err(id, "openhuman.agent_work_control", params).await;
        assert!(
            missing.contains(key),
            "the required `{key}` param is named: {missing}"
        );
    }

    // The read side agrees there is nothing to control.
    let view = h.ok(3207, "openhuman.agent_work_list", json!({})).await;
    assert_eq!(
        view.get("total").and_then(Value::as_u64),
        Some(0),
        "a fresh workspace has no background agent runs: {view}"
    );

    h.join.abort();
}

// ── agent_team: list / close ────────────────────────────────────────────────

/// The team lifecycle across the two uncovered controllers: a created team is
/// `active` and listed; closing it flips the status, stamps `closedAt` and
/// records the summary; and the status filter must then move it from one
/// bucket to the other.
#[tokio::test]
async fn agent_team_close_flips_the_status_the_list_filter_selects_on() {
    let _lock = env_lock();
    let h = setup().await;

    let created = h
        .ok(
            3301,
            "openhuman.agent_team_create",
            json!({
                "leadAgentId": "planner",
                "parentThreadId": "thread-team-e2e",
                "summary": "ship the e2e wave",
                "members": [
                    { "name": "Ada", "agentId": "researcher" },
                    { "name": "Grace" }
                ],
            }),
        )
        .await;
    let team_id = str_at(&created, "/team/id").to_string();
    assert_eq!(
        created.pointer("/team/status").and_then(Value::as_str),
        Some("active"),
        "a new team is active: {created}"
    );
    assert_eq!(
        created.pointer("/team/leadAgentId").and_then(Value::as_str),
        Some("planner")
    );
    assert_eq!(
        created.pointer("/members").and_then(Value::as_array).map(Vec::len),
        Some(2),
        "both seeded members are created: {created}"
    );
    assert!(
        created.pointer("/team/closedAt").is_none_or(Value::is_null),
        "an active team has no close stamp: {created}"
    );

    let listed = h.ok(3302, "openhuman.agent_team_list", json!({})).await;
    assert_eq!(
        listed.get("count").and_then(Value::as_u64),
        Some(1),
        "the new team is listed: {listed}"
    );
    assert_eq!(str_at(&listed, "/teams/0/id"), team_id);

    let by_thread = h
        .ok(
            3303,
            "openhuman.agent_team_list",
            json!({ "parentThreadId": "thread-team-e2e" }),
        )
        .await;
    assert_eq!(by_thread.get("count").and_then(Value::as_u64), Some(1));

    let other_thread = h
        .ok(
            3304,
            "openhuman.agent_team_list",
            json!({ "parentThreadId": "some-other-thread" }),
        )
        .await;
    assert_eq!(
        other_thread.get("count").and_then(Value::as_u64),
        Some(0),
        "the parent-thread filter actually filters: {other_thread}"
    );

    let active_only = h
        .ok(
            3305,
            "openhuman.agent_team_list",
            json!({ "status": "active" }),
        )
        .await;
    assert_eq!(active_only.get("count").and_then(Value::as_u64), Some(1));

    let closed = h
        .ok(
            3306,
            "openhuman.agent_team_close",
            json!({ "teamId": team_id, "summary": "wave landed" }),
        )
        .await;
    assert_eq!(
        closed.pointer("/team/status").and_then(Value::as_str),
        Some("closed"),
        "close flips the status: {closed}"
    );
    assert_eq!(
        closed.pointer("/team/summary").and_then(Value::as_str),
        Some("wave landed"),
        "the closing summary replaces the opening one: {closed}"
    );
    assert!(
        closed
            .pointer("/team/closedAt")
            .and_then(Value::as_str)
            .is_some(),
        "closing stamps a time: {closed}"
    );

    let after_active = h
        .ok(
            3307,
            "openhuman.agent_team_list",
            json!({ "status": "active" }),
        )
        .await;
    assert_eq!(
        after_active.get("count").and_then(Value::as_u64),
        Some(0),
        "the closed team left the active bucket: {after_active}"
    );

    let after_closed = h
        .ok(
            3308,
            "openhuman.agent_team_list",
            json!({ "status": "closed" }),
        )
        .await;
    assert_eq!(
        after_closed.get("count").and_then(Value::as_u64),
        Some(1),
        "and entered the closed one: {after_closed}"
    );
    assert_eq!(str_at(&after_closed, "/teams/0/id"), team_id);

    h.join.abort();
}

/// Both controllers refuse malformed input rather than degrading to a default:
/// a wrongly typed pagination field must not silently become "all teams".
#[tokio::test]
async fn agent_team_list_and_close_reject_malformed_input() {
    let _lock = env_lock();
    let h = setup().await;

    let bad_limit = h
        .err(
            3401,
            "openhuman.agent_team_list",
            json!({ "limit": "not-a-number" }),
        )
        .await;
    assert!(
        bad_limit.contains("limit") && bad_limit.contains("agent_team.list"),
        "a mistyped filter is rejected, not ignored: {bad_limit}"
    );
    assert!(
        bad_limit.contains("expected unsigned integer"),
        "and the rejection says what the field should have been: {bad_limit}"
    );

    let unknown_team = h
        .err(
            3402,
            "openhuman.agent_team_close",
            json!({ "teamId": "team-does-not-exist" }),
        )
        .await;
    assert!(
        unknown_team.contains("team-does-not-exist"),
        "close names the team it could not find: {unknown_team}"
    );

    let no_team_id = h.err(3403, "openhuman.agent_team_close", json!({})).await;
    assert!(
        no_team_id.contains("teamId"),
        "the required `teamId` param is named: {no_team_id}"
    );

    let blank_team_id = h
        .err(
            3404,
            "openhuman.agent_team_close",
            json!({ "teamId": "   " }),
        )
        .await;
    assert!(
        blank_team_id.contains("teamId"),
        "a blank teamId is treated as absent: {blank_team_id}"
    );

    h.join.abort();
}

// ── subagent: cancel / steer ────────────────────────────────────────────────

/// The background-tasks drawer's Cancel and steer controls. Both answer
/// **structurally** for a task that is not running rather than erroring — the
/// drawer's row may be stale, and a stale row must not raise. `steer` goes
/// further and says *why* it did nothing.
#[tokio::test]
async fn subagent_cancel_and_steer_report_structurally_for_an_unknown_task() {
    let _lock = env_lock();
    let h = setup().await;

    let cancelled = h
        .ok(
            3501,
            "openhuman.subagent_cancel",
            json!({ "taskId": "sub-not-running" }),
        )
        .await;
    assert_eq!(
        cancelled.get("cancelled").and_then(Value::as_bool),
        Some(false),
        "nothing was running, and that is an answer not a failure: {cancelled}"
    );
    assert_eq!(
        cancelled.get("taskId").and_then(Value::as_str),
        Some("sub-not-running"),
        "the answer echoes the task asked about: {cancelled}"
    );

    // A reason is accepted on the cancel path and must not change the verdict.
    let with_reason = h
        .ok(
            3502,
            "openhuman.subagent_cancel",
            json!({ "taskId": "sub-not-running", "reason": "changed my mind" }),
        )
        .await;
    assert_eq!(
        with_reason.get("cancelled").and_then(Value::as_bool),
        Some(false)
    );

    let steered = h
        .ok(
            3503,
            "openhuman.subagent_steer",
            json!({ "taskId": "sub-not-running", "message": "focus on the failing test" }),
        )
        .await;
    assert_eq!(
        steered.get("steered").and_then(Value::as_bool),
        Some(false),
        "an unknown task cannot be steered: {steered}"
    );
    assert_eq!(
        steered.get("reason").and_then(Value::as_str),
        Some("unknown"),
        "and the caller is told which of the failure modes it hit: {steered}"
    );
    assert_eq!(
        steered.get("mode").and_then(Value::as_str),
        Some("steer"),
        "steer is the default queue mode: {steered}"
    );

    let collect = h
        .ok(
            3504,
            "openhuman.subagent_steer",
            json!({
                "taskId": "sub-not-running",
                "message": "note this for later",
                "mode": "collect",
            }),
        )
        .await;
    assert_eq!(
        collect.get("mode").and_then(Value::as_str),
        Some("collect"),
        "an explicit queue mode is honoured and echoed: {collect}"
    );

    // An unrecognised mode falls back to steer rather than erroring.
    let bogus_mode = h
        .ok(
            3505,
            "openhuman.subagent_steer",
            json!({ "taskId": "sub-not-running", "message": "hi", "mode": "teleport" }),
        )
        .await;
    assert_eq!(
        bogus_mode.get("mode").and_then(Value::as_str),
        Some("steer"),
        "an unknown mode degrades to the default: {bogus_mode}"
    );

    h.join.abort();
}

/// `taskId` is **trimmed** before it reaches the registry, so a whitespace-
/// padded id must be rejected up front — it would otherwise pass a naive
/// non-empty check and then silently never match a registry key.
#[tokio::test]
async fn subagent_controls_reject_blank_and_absent_required_params() {
    let _lock = env_lock();
    let h = setup().await;

    // Each method gets its own param shape: the RPC layer rejects an unknown
    // param, and `subagent_cancel` declares no `message`.
    for (id, method, extra) in [
        (3601, "openhuman.subagent_cancel", json!({})),
        (3602, "openhuman.subagent_steer", json!({ "message": "hi" })),
    ] {
        let mut blank = extra.as_object().cloned().expect("params object");
        blank.insert("taskId".to_string(), json!("   "));
        let blank = h.err(id, method, Value::Object(blank)).await;
        assert!(
            blank.contains("taskId"),
            "{method} rejects a whitespace-only taskId: {blank}"
        );

        let absent = h.err(id + 10, method, extra).await;
        assert!(
            absent.contains("taskId"),
            "{method} names its required taskId: {absent}"
        );
    }

    let no_message = h
        .err(
            3603,
            "openhuman.subagent_steer",
            json!({ "taskId": "sub-1" }),
        )
        .await;
    assert!(
        no_message.contains("message"),
        "steer names its required message: {no_message}"
    );

    let blank_message = h
        .err(
            3604,
            "openhuman.subagent_steer",
            json!({ "taskId": "sub-1", "message": "  " }),
        )
        .await;
    assert!(
        blank_message.contains("message"),
        "a blank steer message is treated as absent: {blank_message}"
    );

    h.join.abort();
}

// ── agent_experience ────────────────────────────────────────────────────────

fn experience(id: &str, summary: &str, lesson: &str, tools: &[&str], tags: &[&str]) -> Value {
    let now = 1_760_000_000_000i64;
    json!({
        "id": id,
        "created_at_ms": now,
        "updated_at_ms": now,
        "source": "manual",
        "agent_id": "planner",
        "entrypoint": "chat",
        "task_fingerprint": format!("fp-{id}"),
        "task_summary": summary,
        "tools_used": tools,
        "tool_sequence": tools,
        "outcome": "success",
        "error_class": null,
        "lesson": lesson,
        "reuse_hint": "reuse when the same tools are available",
        "avoid_hint": null,
        "confidence": 0.9,
        "tags": tags,
        "payload_hash": null,
        "dismissed": false
    })
}

/// The procedural-experience store end to end: capture persists, list reads
/// back, retrieve ranks against a task query, and dismiss takes a record out of
/// retrieval **without** deleting it.
#[tokio::test]
async fn agent_experience_capture_list_retrieve_and_dismiss_round_trip() {
    let _lock = env_lock();
    ensure_memory_seams();
    let h = setup().await;

    let before: Vec<String> = h
        .ok(3701, "openhuman.agent_experience_list", json!({}))
        .await
        .as_array()
        .expect("list returns an array")
        .iter()
        .map(|entry| str_at(entry, "/id").to_string())
        .collect();
    assert!(
        !before.iter().any(|id| id.starts_with("w1exp-")),
        "this suite's ids are unique to it, so none may pre-exist: {before:?}"
    );

    let stored = h
        .ok(
            3702,
            "openhuman.agent_experience_capture",
            json!({
                "experience": experience(
                    "w1exp-deploy",
                    "deploy the staging build",
                    "run the migration before restarting the service",
                    &["shell_exec", "http_request"],
                    &["deploy", "staging"],
                ),
            }),
        )
        .await;
    assert_eq!(
        stored.get("id").and_then(Value::as_str),
        Some("w1exp-deploy"),
        "capture returns the stored record: {stored}"
    );
    assert_eq!(
        stored.get("dismissed").and_then(Value::as_bool),
        Some(false)
    );

    h.ok(
        3703,
        "openhuman.agent_experience_capture",
        json!({
            "experience": experience(
                "w1exp-unrelated",
                "reconcile the invoice ledger",
                "always reconcile before closing the month",
                &["read_file"],
                &["finance"],
            ),
        }),
    )
    .await;

    let listed = h
        .ok(3704, "openhuman.agent_experience_list", json!({}))
        .await;
    let mine: Vec<&str> = listed
        .as_array()
        .expect("list returns an array")
        .iter()
        .map(|entry| str_at(entry, "/id"))
        .filter(|id| id.starts_with("w1exp-"))
        .collect();
    assert_eq!(mine.len(), 2, "both captures are readable: {listed}");
    assert!(mine.contains(&"w1exp-deploy") && mine.contains(&"w1exp-unrelated"));

    // Retrieval must RANK, not just return everything: the deploy query has to
    // put the deploy experience first.
    let hits = h
        .ok(
            3705,
            "openhuman.agent_experience_retrieve",
            json!({
                "query": "deploy the staging build",
                "tools": ["shell_exec"],
                "tags": ["deploy"],
                "max_hits": 5,
            }),
        )
        .await;
    let hits = hits.as_array().expect("retrieve returns an array").clone();
    let rank = |id: &str| {
        hits.iter()
            .position(|hit| hit.pointer("/experience/id").and_then(Value::as_str) == Some(id))
    };
    let deploy_rank = rank("w1exp-deploy")
        .unwrap_or_else(|| panic!("the captured experience is recalled: {hits:?}"));
    if let Some(unrelated_rank) = rank("w1exp-unrelated") {
        assert!(
            deploy_rank < unrelated_rank,
            "retrieval RANKS: the deploy query puts the deploy experience above \
             the finance one ({deploy_rank} vs {unrelated_rank}): {hits:?}"
        );
    }
    let hit = &hits[deploy_rank];
    assert!(
        hit.get("score")
            .and_then(Value::as_f64)
            .is_some_and(|score| score > 0.0),
        "a hit carries a positive score: {hit:?}"
    );
    assert!(
        hit.get("match_reasons")
            .and_then(Value::as_array)
            .is_some_and(|reasons| !reasons.is_empty()),
        "and says why it matched: {hit:?}"
    );

    // `max_hits` is a cap, not a hint.
    let capped = h
        .ok(
            3706,
            "openhuman.agent_experience_retrieve",
            json!({ "query": "deploy the staging build", "max_hits": 1 }),
        )
        .await;
    assert!(
        capped.as_array().map(Vec::len).unwrap_or(0) <= 1,
        "retrieve honours max_hits: {capped}"
    );

    let dismissed = h
        .ok(
            3707,
            "openhuman.agent_experience_dismiss",
            json!({ "id": "w1exp-deploy" }),
        )
        .await;
    assert_eq!(
        dismissed.get("id").and_then(Value::as_str),
        Some("w1exp-deploy")
    );
    assert_eq!(
        dismissed.get("dismissed").and_then(Value::as_bool),
        Some(true),
        "an existing experience is marked dismissed: {dismissed}"
    );

    let after = h
        .ok(
            3708,
            "openhuman.agent_experience_retrieve",
            json!({ "query": "deploy the staging build", "max_hits": 5 }),
        )
        .await;
    assert!(
        !after
            .as_array()
            .expect("retrieve array")
            .iter()
            .any(|hit| hit.pointer("/experience/id").and_then(Value::as_str) == Some("w1exp-deploy")),
        "a dismissed experience is out of retrieval: {after}"
    );

    // Dismissing something absent is reported, not raised.
    let absent = h
        .ok(
            3709,
            "openhuman.agent_experience_dismiss",
            json!({ "id": "w1exp-does-not-exist" }),
        )
        .await;
    assert_eq!(
        absent.get("dismissed").and_then(Value::as_bool),
        Some(false),
        "dismissing an unknown id reports false rather than erroring: {absent}"
    );

    h.join.abort();
}

/// Every `agent_experience` controller deserializes its params as a typed
/// struct, so a missing or wrongly shaped field is rejected at the boundary
/// instead of being defaulted into a silently wrong record.
#[tokio::test]
async fn agent_experience_controllers_reject_malformed_params() {
    let _lock = env_lock();
    ensure_memory_seams();
    let h = setup().await;

    let no_experience = h
        .err(3801, "openhuman.agent_experience_capture", json!({}))
        .await;
    assert!(
        no_experience.contains("experience"),
        "capture names the field it needs: {no_experience}"
    );

    // A record missing a required field must not be defaulted into existence.
    let incomplete = h
        .err(
            3802,
            "openhuman.agent_experience_capture",
            json!({ "experience": { "id": "w1exp-partial", "task_summary": "half a record" } }),
        )
        .await;
    assert!(
        !incomplete.is_empty(),
        "an incomplete experience is refused: {incomplete}"
    );

    let no_query = h
        .err(3803, "openhuman.agent_experience_retrieve", json!({}))
        .await;
    assert!(
        no_query.contains("query"),
        "retrieve names its required query: {no_query}"
    );

    let no_id = h
        .err(3804, "openhuman.agent_experience_dismiss", json!({}))
        .await;
    assert!(
        no_id.contains("id"),
        "dismiss names its required id: {no_id}"
    );

    // An unknown profile partition is an error, not an empty result — a typo
    // must not read as "this profile has no experiences".
    let unknown_profile = h
        .err(
            3805,
            "openhuman.agent_experience_list",
            json!({ "profile_id": "profile-does-not-exist" }),
        )
        .await;
    assert!(
        unknown_profile.contains("profile-does-not-exist"),
        "an unknown profile is named rather than silently empty: {unknown_profile}"
    );

    h.join.abort();
}
