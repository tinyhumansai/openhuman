//! Wire-level e2e coverage for the agent / approval / memory RPC controllers
//! that `scripts/check-domain-e2e-coverage.mjs` reported as uncovered.
//!
//! # Why a new file rather than extending an existing one
//!
//! The eight controllers here span three families whose existing raw-coverage
//! files are already 2–4k lines each, and several of these tests need a
//! *clean* process-global approval gate. Sharing a file with
//! `tool_registry_approval_raw_coverage_e2e.rs` would make them order-dependent
//! on whether a sibling test already installed the gate — the exact reason that
//! file documents for skipping `preauthorize_flow`'s success path
//! (see its `approval_schema_handlers_validate_params_and_surface_empty_gate_state`).
//!
//! # What "covered" means here, and what the checker actually measures
//!
//! `check-domain-e2e-coverage.mjs` marks a controller covered when the string
//! literal `"openhuman.<method>"` appears anywhere in a `tests/**/*_e2e.rs`
//! file. It does not verify the method is invoked or asserted on — a comment
//! scores 100%. Every method named below is therefore driven over the real
//! JSON-RPC router (`build_core_http_router`) and asserted on, so the number the
//! gate reports and the coverage that exists are the same thing.

use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::http::header::AUTHORIZATION;
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;

const TEST_RPC_TOKEN: &str = "agent-approval-memory-coverage-e2e-token";

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static AUTH_INIT: OnceLock<()> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Restores an environment variable to its prior value on drop so a harness
/// cannot leak `HOME` into a sibling test in the same binary.
struct EnvVarGuard {
    key: String,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self {
            key: key.to_string(),
            previous,
        }
    }

    fn set_to_path(key: &str, value: &Path) -> Self {
        Self::set(key, &value.to_string_lossy())
    }

    fn unset(key: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::remove_var(key);
        Self {
            key: key.to_string(),
            previous,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(&self.key, value),
            None => std::env::remove_var(&self.key),
        }
    }
}

struct TestHarness {
    _tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    rpc_base: String,
    _rpc_join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

fn ensure_rpc_auth() {
    AUTH_INIT.get_or_init(|| {
        std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN);
        let token_dir = std::env::temp_dir().join("openhuman-agent-approval-memory-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
}

async fn serve_rpc() -> (
    std::net::SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    ensure_rpc_auth();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc listener");
    let addr = listener.local_addr().expect("rpc listener addr");
    let router = build_core_http_router(false);
    let join = tokio::spawn(async move { axum::serve(listener, router).await });
    (addr, join)
}

fn write_config(openhuman_dir: &Path) {
    std::fs::create_dir_all(openhuman_dir).expect("create .openhuman");
    // `provider = "none"` binds the null memory driver. That is deliberate for
    // `memory_provider_status`: the null driver still reports the three
    // MANDATORY capability families, which is what makes the assertion below a
    // real check rather than a shape check.
    let cfg = r#"api_url = "http://127.0.0.1:9"
default_model = "e2e-model"
default_temperature = 0.2

[secrets]
encrypt = false

[local_ai]
enabled = false

[memory]
provider = "none"
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0

[memory_tree]
embedding_strict = false

[autonomy]
level = "supervised"
workspace_only = false
max_actions_per_hour = 17
require_approval_for_medium_risk = false
block_high_risk_commands = false
auto_approve = []
"#;
    std::fs::write(openhuman_dir.join("config.toml"), cfg).expect("write config.toml");
}

async fn setup() -> TestHarness {
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let workspace = home.join("openhuman-workspace");
    write_config(&workspace);
    write_config(&home.join(".openhuman"));

    let guards = vec![
        EnvVarGuard::set_to_path("HOME", home),
        EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
    ];

    let (addr, rpc_join) = serve_rpc().await;
    TestHarness {
        _tmp: tmp,
        _guards: guards,
        rpc_base: format!("http://{addr}"),
        _rpc_join: rpc_join,
    }
}

async fn rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("client");
    let url = format!("{}/rpc", rpc_base.trim_end_matches('/'));
    let response = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {TEST_RPC_TOKEN}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .unwrap_or_else(|err| panic!("POST {url} {method}: {err}"));
    assert_eq!(response.status(), StatusCode::OK, "{method} HTTP status");
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

/// Peel the conditional `RpcOutcome` envelope. A handler that emits no log
/// lines returns the bare value; one that emits any returns
/// `{ result, logs }`. Both shapes are valid for the same method, so every
/// consumer has to tolerate both — see `src/rpc/mod.rs`.
fn payload<'a>(value: &'a Value, context: &str) -> &'a Value {
    let result = ok(value, context);
    result.get("result").unwrap_or(result)
}

fn error_message<'a>(value: &'a Value, context: &str) -> &'a str {
    value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: error missing message: {value}"))
}

// ---------------------------------------------------------------------------
// approval.get_gate_state
// ---------------------------------------------------------------------------

/// `openhuman.approval_get_gate_state` answers over the wire and the boot-state
/// snapshot is internally consistent.
///
/// The three booleans are not independent, and that is the part worth pinning:
/// `disabled_by_env` (the override was honoured, gate OFF) and
/// `override_ignored` (the override was seen and suppressed because the host is
/// the desktop shell) are mutually exclusive by construction, and an honoured
/// disable cannot coexist with `installed`. A regression that made the RPC
/// report both would render two contradictory banners in the UI.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approval_get_gate_state_returns_a_consistent_boot_snapshot() {
    let _lock = env_lock();
    let harness = setup().await;

    let response = rpc(
        &harness.rpc_base,
        1,
        "openhuman.approval_get_gate_state",
        json!({}),
    )
    .await;
    let state = payload(&response, "approval_get_gate_state");

    // NOTE the wire casing: `ApprovalGateBootState` is
    // `#[serde(rename_all = "camelCase")]` (`gate.rs:154`), so the RPC emits
    // `disabledByEnv` / `overrideIgnored` even though the Rust fields are
    // snake_case. Asserting the snake_case names would silently pass on
    // `Option::None` rather than fail, so read them explicitly.
    let installed = state
        .get("installed")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("installed must be a bool: {state}"));
    let disabled_by_env = state
        .get("disabledByEnv")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("disabledByEnv must be a bool: {state}"));
    let override_ignored = state
        .get("overrideIgnored")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("overrideIgnored must be a bool: {state}"));
    let host = state
        .get("host")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("host must be a string: {state}"));

    assert!(
        !(disabled_by_env && override_ignored),
        "disabled_by_env and override_ignored are mutually exclusive, got both in {state}"
    );
    assert!(
        !(installed && disabled_by_env),
        "an env-disabled gate cannot also be installed: {state}"
    );
    // `host` is a pinned string downstream consumers may switch on, so an
    // unrecognised tag is a contract break even though it is "just a string".
    //
    // Four values are reachable, not three. The struct's own doc comment
    // (`gate.rs:167-169`) enumerates only `tauri-shell` / `cli` / `docker`, but
    // `approval_get_gate_state` falls back to `host: "unknown"` when boot state
    // was never recorded (`approval/rpc.rs:30-35`) — any host that did not go
    // through `bootstrap_core_runtime`, which includes this harness. The
    // frontend already knows this (`approvalApi.ts:171` documents all four and
    // types `host` as a plain `string`); only the Rust doc comment is stale.
    assert!(
        matches!(host, "tauri-shell" | "cli" | "docker" | "unknown"),
        "host must be one of the pinned tags (tauri-shell|cli|docker|unknown), \
         got {host:?} in {state}"
    );
}

// ---------------------------------------------------------------------------
// approval.preauthorize_flow
// ---------------------------------------------------------------------------

/// `openhuman.approval_preauthorize_flow` honours its gate-absent contract:
/// it **succeeds** with `gate_installed: false` rather than erroring, and does
/// so identically on a repeat call.
///
/// # Scope — what this proves, and what it deliberately does not
///
/// No `ApprovalGate` is installed in this harness, so the handler takes the
/// documented gate-absent branch (`approval/rpc.rs:125-139`). That branch is a
/// real contract worth pinning — the schema states it in as many words
/// ("Succeeds with gate_installed=false when the approval gate is disabled"),
/// and a flow-save path that started erroring when the gate is off would break
/// every CLI and headless host. Mutating that branch to return an error turns
/// this test red, so the assertion is live.
///
/// It does **not** prove grant idempotency. An earlier draft claimed to, by
/// comparing `granted` counts across two calls; mutation testing showed that
/// was vacuous — with no gate installed both calls return `granted: []`, so
/// `0 <= 0` held no matter what the grant logic did.
///
/// Exercising the real grant path needs `ApprovalGate::init_global`, which
/// installs a **process-global** `OnceLock`. Doing that here would change what
/// `approval_get_gate_state_returns_a_consistent_boot_snapshot` observes in the
/// same binary, making the pair order-dependent — the identical reason
/// `tool_registry_approval_raw_coverage_e2e.rs` gives for skipping this path.
/// Grant idempotency ("already-trusted tools are reported, not re-granted") is
/// covered by the unit tests in `approval::rpc` / `approval::store`, which own
/// the gate lifecycle.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approval_preauthorize_flow_succeeds_without_a_gate_installed() {
    let _lock = env_lock();
    let harness = setup().await;

    let flow_id = format!("flow-{}", uuid::Uuid::new_v4());
    let args = json!({
        "flow_id": flow_id,
        "tool_names": ["slack_post_message", "gmail_send_email"],
    });

    let first = rpc(
        &harness.rpc_base,
        2,
        "openhuman.approval_preauthorize_flow",
        args.clone(),
    )
    .await;
    let first_payload = payload(&first, "approval_preauthorize_flow first call");

    assert_eq!(
        first_payload.get("gate_installed"),
        Some(&Value::Bool(false)),
        "with no gate installed the call must succeed reporting gate_installed=false, \
         not error and not claim a gate: {first_payload}"
    );
    assert_eq!(
        first_payload.get("granted").and_then(Value::as_array),
        Some(&vec![]),
        "no gate means nothing can be granted: {first_payload}"
    );
    assert_eq!(
        first_payload.get("flow_id").and_then(Value::as_str),
        Some(flow_id.as_str()),
        "the response must echo the flow_id it was asked about: {first_payload}"
    );

    // Repeating the call must not change the answer — the gate-absent branch
    // persists nothing, so there is no state for a second call to trip over.
    let second = rpc(
        &harness.rpc_base,
        3,
        "openhuman.approval_preauthorize_flow",
        args,
    )
    .await;
    let second_payload = payload(&second, "approval_preauthorize_flow repeat call");
    assert_eq!(
        first_payload, second_payload,
        "a repeat call with no gate installed must be byte-identical: \
         {first_payload} then {second_payload}"
    );
}

/// Required params are enforced at the wire boundary, not merely at the
/// handler. Registering a controller without wiring its param validation into
/// dispatch would leave these calls panicking or succeeding with defaults.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approval_preauthorize_flow_rejects_malformed_params_over_the_wire() {
    let _lock = env_lock();
    let harness = setup().await;

    let missing_flow = rpc(
        &harness.rpc_base,
        4,
        "openhuman.approval_preauthorize_flow",
        json!({}),
    )
    .await;
    assert!(
        error_message(&missing_flow, "missing flow_id").contains("flow_id"),
        "missing flow_id must name the param, got {missing_flow}"
    );

    let missing_tools = rpc(
        &harness.rpc_base,
        5,
        "openhuman.approval_preauthorize_flow",
        json!({ "flow_id": "flow-1" }),
    )
    .await;
    assert!(
        error_message(&missing_tools, "missing tool_names").contains("tool_names"),
        "missing tool_names must name the param, got {missing_tools}"
    );

    let scalar_tools = rpc(
        &harness.rpc_base,
        6,
        "openhuman.approval_preauthorize_flow",
        json!({ "flow_id": "flow-1", "tool_names": "slack_post_message" }),
    )
    .await;
    assert!(
        scalar_tools.get("error").is_some(),
        "a scalar tool_names must be rejected, not coerced: {scalar_tools}"
    );
}

// ---------------------------------------------------------------------------
// The approval gate's classification contract (openhuman#5862)
// ---------------------------------------------------------------------------

/// **This test reproduces openhuman#5862 and FAILS on `main` today. That is its
/// purpose.** It is `#[ignore]`d so it does not turn the shared lane red;
/// run it with `cargo test --test agent_approval_memory_coverage_e2e -- --ignored`.
/// Un-ignore it when #5863 (or an equivalent fix) lands — at that point it
/// becomes the regression guard.
///
/// # What is broken
///
/// `ApprovalSecurityMiddleware` decides whether to park a call on exactly one
/// predicate — `Tool::external_effect_with_args`
/// (`agent/tinyagents/middleware_part_02.rs:267-296`). The Composio tools
/// (`ComposioExecuteTool`, `ComposioActionTool`) declare
/// `permission_level() == PermissionLevel::Write` but never override
/// `external_effect*`, so they inherit the trait default of `false`
/// (`tinytools/src/tool/types.rs:147-158`). Agent-initiated Composio writes —
/// `GMAIL_SEND_EMAIL` and friends — therefore run with no approval card, no
/// audit row, and no denial path.
///
/// Three defences were checked and none applies:
/// 1. There is no compensating manual gate. The only `ApprovalGate` intercept
///    in `integrations/composio/` is `ComposioConnectTool`'s, whose
///    `external_effect = false` *is* deliberate and documented.
/// 2. The alternative gate adapter (`host/security_gate.rs:573`) keys off the
///    same predicate, so which gate is installed makes no difference.
/// 3. It is not that the codebase does not know how: the legacy
///    `ComposioTool` (`integrations/composio/tools/direct_part_03.rs:66-80`)
///    classifies *correctly* and arg-aware — `action: "execute"` is an external
///    effect, `list` and `connect` are not. That tool is **not** an agent tool
///    (it is an internal client, built only in `client_part_02.rs`). The
///    classification was simply not carried over when the surface was split
///    into per-purpose tools, which is what makes this a migration regression
///    rather than an oversight in a single file.
///
/// Both unclassified tools are on the live agent path:
/// `ComposioExecuteTool` via `all_composio_agent_tools`
/// (`tools_part_03.rs:339`), and `ComposioActionTool` via the subagent runner
/// (`agent/harness/subagent_runner/ops/provider.rs:193`, `runner.rs:1100`).
///
/// # Why it is written as a classification assertion
///
/// Driving a live Composio write end-to-end would need provider credentials and
/// would actually send mail. The classification *is* the bug — the gate is a
/// pure function of it — so asserting the classification tests the real defect
/// without a network round-trip, and it cannot pass vacuously: it reads the
/// same method the middleware reads.
#[test]
#[ignore = "reproduces openhuman#5862: Composio write tools bypass the approval gate; \
            un-ignore when #5863 or an equivalent classification fix lands"]
fn composio_write_tools_declare_an_external_effect_so_the_approval_gate_parks_them() {
    use std::sync::Arc;

    use openhuman_core::openhuman::config::Config;
    use openhuman_core::openhuman::integrations::composio::tools::ComposioExecuteTool;
    use openhuman_core::openhuman::tools::traits::Tool;

    let tool = ComposioExecuteTool::new(Arc::new(Config::default()));

    // A write to a third-party SaaS is the canonical external effect: it is not
    // reversible inside the user's own machine, which is the line the trait
    // documents (`types.rs:144-146`).
    let send_email = json!({
        "tool": "GMAIL_SEND_EMAIL",
        "arguments": { "recipient_email": "someone@example.com", "body": "sent by the agent" },
    });

    assert!(
        tool.external_effect_with_args(&send_email),
        "composio_execute(GMAIL_SEND_EMAIL) reports external_effect_with_args=false, so \
         ApprovalSecurityMiddleware never parks it and the write runs unprompted. \
         The tool already classifies itself as PermissionLevel::{:?}; the approval gate \
         reads external_effect*, not permission_level. See openhuman#5862.",
        tool.permission_level()
    );
}

// ---------------------------------------------------------------------------
// agent.run_events / agent.run_status / agent.runs_active
// ---------------------------------------------------------------------------

/// The three replay controllers are read-only projections over the durable
/// tinyagents journal. An unknown `run_id` is a normal, expected query (the UI
/// polls before a run has written anything), so it must return an empty page
/// rather than erroring — and `next_offset` must be `null`, meaning "drained",
/// not `0`, which a paging client would follow forever.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_run_events_returns_a_drained_empty_page_for_an_unknown_run() {
    let _lock = env_lock();
    let harness = setup().await;

    let response = rpc(
        &harness.rpc_base,
        10,
        "openhuman.agent_run_events",
        json!({ "run_id": "run-that-was-never-started" }),
    )
    .await;
    let page = payload(&response, "agent_run_events unknown run");

    let events = page
        .get("events")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("events must be an array: {page}"));
    assert!(
        events.is_empty(),
        "an unknown run must yield no events, got {events:?}"
    );
    assert_eq!(
        page.get("next_offset"),
        Some(&Value::Null),
        "a drained stream must report next_offset=null, not 0 — a client that follows \
         a 0 cursor would page forever: {page}"
    );
}

/// `run_id` is the one required param; omitting it must be rejected by the
/// **schema validator** at the wire boundary, before the handler runs.
///
/// # Why the assertion is on the exact validator string
///
/// An earlier draft asserted only `.contains("run_id")`. Mutation testing
/// showed that was vacuous: when the required-param check is defeated, the
/// handler proceeds with an empty id and fails downstream with
/// `"read run events failed: … run_id=: validation error: store namespace and
/// key must not be empty"` — which *also* contains `"run_id"`, because the
/// handler embeds `run_id=` as a debug label in its own error text. The loose
/// assertion could not tell "rejected the missing param" from "accepted it and
/// failed later", so it passed under both.
///
/// Matching `missing required param 'run_id'` — the exact format emitted by
/// `validate_params` (`src/core/all.rs:1339-1343`) — pins the layer that is
/// actually supposed to reject this.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_run_events_rejects_a_missing_run_id() {
    let _lock = env_lock();
    let harness = setup().await;

    let response = rpc(
        &harness.rpc_base,
        11,
        "openhuman.agent_run_events",
        json!({}),
    )
    .await;
    assert!(
        error_message(&response, "missing run_id").contains("missing required param 'run_id'"),
        "a missing run_id must be rejected by the schema validator before the handler runs; \
         a downstream failure that merely mentions run_id is not the same thing: {response}"
    );
}

/// An over-large `limit` is **accepted and clamped**, not rejected.
///
/// # What this test does and does not prove
///
/// It proves the *tolerance* half: an absurd `limit` must not become a param
/// error, because a paging client that guesses high should degrade to the
/// maximum page rather than fail. That is a real, falsifiable behaviour —
/// making the handler reject `limit > MAX_EVENTS_LIMIT` turns this test red.
///
/// It deliberately does **not** assert `events.len() <= MAX_EVENTS_LIMIT`. An
/// earlier draft did, and mutation testing showed the assertion was vacuous:
/// this harness queries an unknown run, so `events` is always empty and
/// `0 <= 1000` holds no matter what the clamp does. Proving the clamp itself
/// needs a journal seeded with more than 1000 events, which belongs in a
/// fixture-backed test rather than here. The clamp is pinned instead by
/// `replay::ops` (`limit.clamp(1, MAX_EVENTS_LIMIT)`, `ops.rs:56`) and by the
/// handler's own `.min(MAX_EVENTS_LIMIT)` (`replay/schemas.rs:248`) — note
/// there are two independent clamps, so removing either alone changes nothing
/// observable.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_run_events_accepts_an_oversized_limit_without_erroring() {
    let _lock = env_lock();
    let harness = setup().await;

    let response = rpc(
        &harness.rpc_base,
        12,
        "openhuman.agent_run_events",
        json!({ "run_id": "run-unknown", "offset": 0, "limit": 1_000_000 }),
    )
    .await;

    assert!(
        response.get("error").is_none(),
        "an oversized limit must be clamped and served, not rejected: {response}"
    );
    let page = payload(&response, "agent_run_events oversized limit");
    assert!(
        page.get("events").and_then(Value::as_array).is_some(),
        "an oversized limit must still return a well-formed page: {page}"
    );
}

/// An unknown run has no status snapshot. `null` is the documented answer —
/// distinct from an error, which the UI would surface as a failure.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_run_status_returns_null_for_an_unknown_run() {
    let _lock = env_lock();
    let harness = setup().await;

    let response = rpc(
        &harness.rpc_base,
        13,
        "openhuman.agent_run_status",
        json!({ "run_id": "run-that-was-never-started" }),
    )
    .await;
    assert!(
        response.get("error").is_none(),
        "an unknown run is a normal query, not an error: {response}"
    );
    assert_eq!(
        payload(&response, "agent_run_status unknown run"),
        &Value::Null,
        "an unknown run must report null status"
    );
}

/// `agent_runs_active` takes two optional filters and must accept the
/// unfiltered call, both filters, and either alone — always returning a `runs`
/// array. A filter that was wired as required would break the UI's default
/// unfiltered poll.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_runs_active_accepts_every_filter_combination() {
    let _lock = env_lock();
    let harness = setup().await;

    for (id, params, label) in [
        (14, json!({}), "no filter"),
        (15, json!({ "thread_id": "thread-1" }), "thread only"),
        (16, json!({ "root_run_id": "root-1" }), "root only"),
        (
            17,
            json!({ "thread_id": "thread-1", "root_run_id": "root-1" }),
            "both filters",
        ),
    ] {
        let response = rpc(&harness.rpc_base, id, "openhuman.agent_runs_active", params).await;
        let body = payload(&response, &format!("agent_runs_active ({label})"));
        assert!(
            body.get("runs").and_then(Value::as_array).is_some(),
            "runs must be an array for {label}: {body}"
        );
    }
}

// ---------------------------------------------------------------------------
// agent.graph_topologies / agent.registry_snapshot
// ---------------------------------------------------------------------------

/// `agent_graph_topologies` exports structure only. The contract in its own
/// schema is explicit — "never closure bodies or run state" — so this asserts
/// the negative as well as the positive: a topology export that started leaking
/// prompt text or run state would be a privacy regression that a
/// shape-only check would not catch.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_graph_topologies_exports_structure_without_run_state() {
    let _lock = env_lock();
    let harness = setup().await;

    let response = rpc(
        &harness.rpc_base,
        18,
        "openhuman.agent_graph_topologies",
        json!({}),
    )
    .await;
    let body = payload(&response, "agent_graph_topologies");

    // `is_some()` would accept `null` or a scalar, so a schema regression that
    // dropped the field could still pass. Require the array itself — the handler
    // builds `graphs` as a `Vec<Value>` and returns
    // `json!({ "graphs": graphs, "agents": agents })`
    // (`src/openhuman/agent/schemas.rs:486`), so it is an array, not an object.
    body.get("graphs")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("graph_topologies must return a `graphs` array: {body}"));

    let serialized = body.to_string();
    for leaked in ["system_prompt", "prompt_text", "api_key", "tool_arguments"] {
        assert!(
            !serialized.contains(leaked),
            "graph topology export is structure-only but contained {leaked:?}"
        );
    }
}

/// `agent_registry_snapshot` projects the capability registry as metadata only.
/// `counts` must agree with the length of `components` — they are derived from
/// the same inventory, so a mismatch means one of the two was assembled from a
/// stale or partial source.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_registry_snapshot_counts_agree_with_the_component_inventory() {
    let _lock = env_lock();
    let harness = setup().await;

    let response = rpc(
        &harness.rpc_base,
        19,
        "openhuman.agent_registry_snapshot",
        json!({}),
    )
    .await;
    let body = payload(&response, "agent_registry_snapshot");

    let components = body
        .get("components")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("components must be an array: {body}"));

    // `counts` carries the per-kind entries AND a `total` key
    // (`agent/schemas.rs:643-649`), so summing every value double-counts.
    // Check the two claims separately — that is stronger than one sum anyway,
    // because it catches `total` and the per-kind breakdown disagreeing with
    // each other as well as with `components`.
    // NOT `if let`: a missing or non-object `counts` would silently skip every
    // assertion below and the case would still pass on `components` alone,
    // which is precisely the agreement this test exists to pin.
    {
        let counts = body
            .get("counts")
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("registry_snapshot must return a `counts` object: {body}"));
        let per_kind: u64 = counts
            .iter()
            .filter(|(key, _)| key.as_str() != "total")
            .filter_map(|(_, value)| value.as_u64())
            .sum();
        assert_eq!(
            per_kind,
            components.len() as u64,
            "per-kind counts ({per_kind}) must sum to the component inventory ({}); \
             a mismatch means counts and components came from different sources",
            components.len()
        );
        assert_eq!(
            counts.get("total").and_then(Value::as_u64),
            Some(components.len() as u64),
            "counts.total must equal the component inventory ({}): {body}",
            components.len()
        );
    }

    // Metadata only — the schema is explicit that live per-run handles and run
    // state are excluded.
    for component in components {
        assert!(
            component.get("id").is_some(),
            "every component carries an id: {component}"
        );
    }
}

// ---------------------------------------------------------------------------
// memory.provider_status
// ---------------------------------------------------------------------------

/// `openhuman.memory_provider_status` is the RPC that *reports* the bound
/// driver's capability set, and it is deliberately the one memory controller
/// that is never capability-gated (`core/all.rs:775-780`: "Gating it on a
/// capability would be self-referential and would hide the explanation for
/// every other absence in this block").
///
/// # The capability assertion is conditional, deliberately
///
/// An unresolved slot (no workspace context) reports `class="null"`,
/// `health="down"`, **empty** capabilities and a `last_error` — pinned by
/// `memory::ops::provider_tests::status_without_a_context_reports_an_unresolved_slot`.
/// A bound driver reports its advertised families. Asserting the MANDATORY
/// three unconditionally would therefore be wrong, not strict: it would fail on
/// the legitimate unresolved path.
///
/// So this asserts the *invariant that holds either way* — a driver that
/// reports itself healthy must advertise the three MANDATORY families
/// (`Capabilities::validate` refuses to bind one that does not), and a driver
/// that advertises nothing must say why. That pairing is what a regression
/// would break: a driver going silently capability-less while still reporting
/// `ready` is exactly the shape of openhuman#5598, where `memory_tree` methods
/// answered `UnknownMethod` on staging with no visible explanation.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_provider_status_reports_the_bound_driver_and_its_mandatory_families() {
    let _lock = env_lock();
    let harness = setup().await;

    let response = rpc(
        &harness.rpc_base,
        20,
        "openhuman.memory_provider_status",
        json!({}),
    )
    .await;
    let status = payload(&response, "memory_provider_status");

    assert_eq!(
        status.get("slot").and_then(Value::as_str),
        Some("memory"),
        "slot is always \"memory\" for this controller: {status}"
    );

    let class = status
        .get("class")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("class must be a string: {status}"));
    assert!(
        matches!(class, "embedded" | "external" | "module" | "null"),
        "class must be one of the documented DriverClass tags, got {class:?} in {status}"
    );

    let health = status
        .get("health")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("health must be a string: {status}"));
    assert!(
        matches!(health, "ready" | "degraded" | "down"),
        "health must be ready|degraded|down, got {health:?} in {status}"
    );

    // `health_reason` is documented as null when ready — an operator-facing
    // reason attached to a healthy driver would be a contradiction the UI
    // renders as a warning banner on a working system.
    if health == "ready" {
        assert!(
            status
                .get("health_reason")
                .map(|reason| reason.is_null())
                .unwrap_or(true),
            "a ready driver must carry no health_reason: {status}"
        );
    }

    let contract_version = status
        .get("contract_version")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("contract_version must be a string: {status}"));
    let mut parts = contract_version.split('.');
    let (major, minor, extra) = (parts.next(), parts.next(), parts.next());
    assert!(
        major.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            && minor.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            && extra.is_none(),
        "contract_version must be exactly \"<major>.<minor>\", got {contract_version:?}"
    );

    let capabilities: Vec<&str> = status
        .get("capabilities")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("capabilities must be an array: {status}"))
        .iter()
        .filter_map(Value::as_str)
        .collect();

    if capabilities.is_empty() {
        // Nothing bound. That is a legitimate state, but it must be explained:
        // a silently capability-less driver is indistinguishable from a broken
        // one, which is the whole reason this RPC is never capability-gated.
        assert_ne!(
            health, "ready",
            "a driver advertising no capabilities must not report itself ready: {status}"
        );
        assert!(
            status
                .get("last_error")
                .is_some_and(|error| !error.is_null()),
            "an unresolved memory slot must report last_error explaining why nothing bound: \
             {status}"
        );
    } else {
        // Something bound and is serving. `Capabilities::validate` refuses to
        // bind a driver missing any MANDATORY family, so their presence is a
        // structural guarantee, not a property of the pinned artifact.
        for mandatory in ["core", "recall", "portability"] {
            assert!(
                capabilities.contains(&mandatory),
                "a bound driver must advertise the MANDATORY family {mandatory:?}; \
                 got {capabilities:?} in {status}"
            );
        }
    }
}
