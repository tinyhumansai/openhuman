//! JSON-RPC E2E coverage for the skill execution surface: `skill_runtime_*`,
//! the uncovered half of `skills_*`, `skill_registry_categories`, and the
//! `javascript_*` runtime bridge.
//!
//! Run:
//! `cargo test --test raw_coverage_all --features "$(bash scripts/ci/product-features.sh)" skill_runtime_e2e`
//!
//! ## Hermetic by construction
//!
//! Three fixtures, no network out:
//!
//! * A **planted skill** at `<workspace>/skills/<slug>/{SKILL.md,skill.toml}`.
//!   That directory is the legacy skill root — `discover_filtered` scans it with
//!   no trust marker required (`ops_discover.rs:250-261`), which makes it the
//!   cheapest honest way to give the registry something real to resolve.
//! * **Planted run logs** in `<workspace>/skills/.runs/`, written in the exact
//!   `write_header` / `write_footer` format (`run_log.rs:146-172`, `:598-612`).
//!   `scan_runs` and `read_run_log_slice` parse those files, so a planted log is
//!   precisely what the code under test consumes.
//! * A **loopback fixture catalog** for `skill_registry_categories` and the
//!   `skills_install_from_url` happy path, reached through the existing
//!   `OPENHUMAN_SKILL_REGISTRY_CATALOG_URL` /
//!   `OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP` escape hatches.
//!
//! ## Why the live `run` arc is asserted at its validation boundary, not end to end
//!
//! `spawn_workflow_run_background` validates the skill id and its required
//! inputs **synchronously**, then `tokio::spawn`s a detached orchestrator that
//! writes the run-log header, calls `Config::load_or_init()` (reading
//! process-global env), and drives a real agent turn
//! (`run_machinery.rs:184-230`). Driving that here would (a) race the header
//! write that a following `read_run_log` needs, and (b) leave a detached task
//! reading `HOME` / `OPENHUMAN_WORKSPACE` after this case has released the env
//! lock — which, now that all ~77 suites share one process, is contamination
//! aimed at every other suite in the binary. So `run` is asserted on the two
//! errors it raises before spawning, and the log-reading methods are asserted
//! against planted logs where the content is knowable.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::http::header::AUTHORIZATION;
use axum::routing::get;
use axum::Router;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;

/// The bearer this suite *proposes*. It is only used if this suite happens to
/// be the first in the aggregated binary to initialise the token subsystem —
/// see `rpc_token()` below, which is what actually gets sent.
const PROPOSED_RPC_TOKEN: &str = "skill-runtime-e2e-token";

static AUTH_INIT: OnceLock<()> = OnceLock::new();

/// The crate-wide env lock, not a private one — every `tests/raw_coverage/`
/// suite shares one process, so a file-local lock would isolate nothing.
static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ── Env plumbing ────────────────────────────────────────────────────────────

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

/// The bearer the running process will actually validate.
///
/// `core::auth::RPC_TOKEN` is a process-global `OnceLock` and `init_rpc_token`
/// returns early once it is set. Every `tests/raw_coverage/` suite now shares
/// one process, so the FIRST suite to initialise pins the token for all of
/// them; a suite that sent its own hard-coded literal would get 401 whenever it
/// lost that race, in an order that depends on libtest scheduling and load.
///
/// So: propose a token, initialise, then ask what the process settled on and
/// send that. Correct whether this suite wins the race or loses it.
fn rpc_token() -> &'static str {
    AUTH_INIT.get_or_init(|| {
        if std::env::var(CORE_TOKEN_ENV_VAR)
            .map(|v| v.trim().is_empty())
            .unwrap_or(true)
        {
            std::env::set_var(CORE_TOKEN_ENV_VAR, PROPOSED_RPC_TOKEN);
        }
        let token_dir = std::env::temp_dir().join("openhuman-skill-runtime-e2e-auth");
        std::fs::create_dir_all(&token_dir).expect("token dir");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
    openhuman_core::core::auth::get_rpc_token()
        .expect("the token subsystem is initialised by the line above")
}

// ── Harness ─────────────────────────────────────────────────────────────────

const MIN_CONFIG: &str = r#"api_url = "http://127.0.0.1:9"
default_model = "skill-runtime-e2e-model"

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
"#;

struct Harness {
    _tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    workspace: PathBuf,
    rpc_base: String,
    join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

async fn setup(extra: Vec<EnvVarGuard>) -> Harness {
    let _ = rpc_token();

    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    std::fs::create_dir_all(&openhuman_home).expect("create .openhuman");
    std::fs::write(openhuman_home.join("config.toml"), MIN_CONFIG).expect("write config.toml");
    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(MIN_CONFIG).expect("test config must match the config schema");

    let workspace = home.join("workspace");
    std::fs::create_dir_all(workspace.join("skills")).expect("create workspace skills dir");
    // Inside the workspace on purpose. `file_read` resolves relative paths
    // against `action_dir`, but `SecurityPolicy` jails the resolved path to
    // `workspace_dir` — an action dir outside the workspace makes every read
    // fail with `[policy-blocked] Resolved path escapes workspace`, which is
    // exactly what the first run of this suite hit. The tool's own description
    // ("your working directory (the action sandbox)") does not mention the
    // second, tighter boundary.
    let action_dir = workspace.join("actions");
    std::fs::create_dir_all(&action_dir).expect("create action dir");

    let mut guards = vec![
        EnvVarGuard::set_to_path("HOME", home),
        EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace),
        EnvVarGuard::set_to_path("OPENHUMAN_ACTION_DIR", &action_dir),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
    ];
    guards.extend(extra);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc listener");
    let addr: SocketAddr = listener.local_addr().expect("rpc listener addr");
    let router = build_core_http_router(false);
    let join = tokio::spawn(async move { axum::serve(listener, router).await });

    Harness {
        _tmp: tmp,
        _guards: guards,
        workspace,
        rpc_base: format!("http://{addr}"),
        join,
    }
}

async fn rpc(base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("build client");
    let url = format!("{}/rpc", base.trim_end_matches('/'));
    let response = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {}", rpc_token()))
        .json(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
        .send()
        .await
        .unwrap_or_else(|err| panic!("POST {url} {method}: {err}"));
    assert!(
        response.status().is_success(),
        "HTTP {} for {method}",
        response.status()
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

/// The controller payload, unwrapping the `{result, logs}` envelope when one
/// is present.
///
/// `RpcOutcome::into_cli_compatible_json` (`src/rpc/mod.rs:54-62`) wraps the
/// value in `{result, logs}` **only when the handler emitted at least one log
/// line**, and returns it bare otherwise. So the response shape of a single
/// namespace varies with whether its handler happened to log — `javascript_*`
/// always logs and is always wrapped; the `skills_*` handlers pass
/// `Vec::new()` and never are. A caller cannot write one parser for the RPC
/// surface without knowing that. See
/// `~/tinyhuman/bugs/e2e-wave-rpc-envelope-shape-varies-with-logging.md`.
fn payload<'a>(value: &'a Value, context: &str) -> &'a Value {
    let result = ok(value, context);
    match (result.get("result"), result.get("logs")) {
        (Some(inner), Some(_)) => inner,
        _ => result,
    }
}

fn error_message<'a>(value: &'a Value, context: &str) -> &'a str {
    value
        .get("error")
        .unwrap_or_else(|| panic!("{context}: expected a JSON-RPC error, got: {value}"))
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: error carries no message: {value}"))
}

// ── Fixtures ────────────────────────────────────────────────────────────────

/// Plant a runnable skill under the legacy `<workspace>/skills/` root.
///
/// `skill.toml` supplies the runnable id and the declared `[[inputs]]`;
/// `SKILL.md` supplies the body that becomes the inline system prompt. The
/// resource file is what `skills_read_resource` reads back.
fn plant_skill(workspace: &Path, slug: &str, required_input: &str) -> PathBuf {
    let dir = workspace.join("skills").join(slug);
    std::fs::create_dir_all(dir.join("references")).expect("create skill dir");
    std::fs::write(
        dir.join("SKILL.md"),
        format!(
            "---\nname: {slug}\ndescription: A planted skill for the skill-runtime e2e suite.\n---\n\n\
             # {slug}\n\nDo the planted thing.\n"
        ),
    )
    .expect("write SKILL.md");
    std::fs::write(
        dir.join("skill.toml"),
        format!(
            "id = \"{slug}\"\nwhen_to_use = \"When the e2e suite says so.\"\n\n\
             [[inputs]]\nname = \"{required_input}\"\ndescription = \"The one input this skill insists on.\"\n\
             required = true\ntype = \"string\"\n"
        ),
    )
    .expect("write skill.toml");
    std::fs::write(
        dir.join("references").join("note.md"),
        "PLANTED_RESOURCE_BODY\n",
    )
    .expect("write resource");
    dir
}

/// Write a run log in the on-disk format `write_header` / `write_footer`
/// produce, so `scan_runs` / `read_run_log_slice` parse it exactly as they
/// would a real run's.
///
/// `finished` controls whether the `--- result ---` footer lands, which is the
/// single bit that separates a `RUNNING` run from a terminal one and drives the
/// `complete` flag the frontend polls on.
fn plant_run_log(
    workspace: &Path,
    workflow_id: &str,
    run_id: &str,
    started_rfc3339: &str,
    finished: Option<(&str, u64, &str)>,
) -> PathBuf {
    let runs = workspace.join("skills").join(".runs");
    std::fs::create_dir_all(&runs).expect("create runs dir");
    let short = run_id.get(..8).unwrap_or(run_id);
    let path = runs.join(format!("{workflow_id}_20260907T101010Z_{short}.log"));

    let mut body = format!(
        "==== workflow_run: {workflow_id} ====\n\
         run_id : {run_id}\n\
         started: {started_rfc3339} UTC\n\
         inputs : {{}}\n\n\
         --- task prompt ---\nplanted prompt\n\n\
         --- steps ---\n"
    );
    if let Some((status, duration_ms, output)) = finished {
        body.push_str(&format!(
            "\n--- result ---\n\
             status  : {status}\n\
             duration: {duration_ms} ms\n\
             finished: 2026-09-07T10:20:10+00:00 UTC\n\n{output}\n"
        ));
    }
    std::fs::write(&path, body).expect("write run log");
    path
}

/// A loopback stand-in for the skill catalog + a downloadable `SKILL.md`.
async fn serve_fixture_catalog() -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    async fn catalog() -> axum::Json<Value> {
        axum::Json(json!([
            {
                "name": "planted-git-helper",
                "description": "Automate git triage.",
                "category": "software-development",
                "source": "fixture",
                "tags": ["git"],
                "platforms": ["linux", "macos"],
                "envVars": [],
                "commands": []
            },
            {
                "name": "planted-notes-helper",
                "description": "Summarize notes.",
                "category": "productivity",
                "source": "fixture",
                "tags": ["notes"],
                "platforms": ["linux", "macos"],
                "envVars": [],
                "commands": []
            },
            {
                // Deliberately category-less: `list_categories` filters empty
                // strings out, so this row must NOT contribute a category.
                "name": "planted-uncategorised",
                "description": "No category at all.",
                "category": "",
                "source": "fixture",
                "tags": [],
                "platforms": ["linux", "macos"],
                "envVars": [],
                "commands": []
            }
        ]))
    }

    async fn skill_md() -> &'static str {
        "---\nname: url-installed-skill\ndescription: Installed over loopback HTTP by the e2e suite.\n---\n\n# URL Installed Skill\n\nBody.\n"
    }

    let app = Router::new()
        .route("/skills.json", get(catalog))
        .route("/skills/url-installed-skill/SKILL.md", get(skill_md));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture listener");
    let addr = listener.local_addr().expect("fixture addr");
    let join = tokio::spawn(async move { axum::serve(listener, app).await });
    (addr, join)
}

// ── skill_runtime ───────────────────────────────────────────────────────────

/// `skill_runtime_schemas` and `skill_runtime_resolve_runtimes` — the two
/// methods the production smoke-script generator reads.
///
/// `schemas` must return the namespace's own six controllers with the right
/// namespace stamped on each; a generator fed a truncated or mis-namespaced
/// list emits scripts that call methods that do not exist.
#[tokio::test]
async fn skill_runtime_schemas_and_runtime_resolution() {
    let _lock = env_lock();
    let harness = setup(Vec::new()).await;

    let schemas = rpc(
        &harness.rpc_base,
        100,
        "openhuman.skill_runtime_schemas",
        json!({}),
    )
    .await;
    let schemas = ok(&schemas, "skill_runtime_schemas");
    let listed = schemas
        .get("schemas")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("schemas payload has no `schemas` array: {schemas}"));

    let functions: Vec<&str> = listed
        .iter()
        .map(|entry| {
            assert_eq!(
                entry.get("namespace").and_then(Value::as_str),
                Some("skill_runtime"),
                "every returned schema must be stamped with its own namespace: {entry}"
            );
            entry
                .get("function")
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("schema entry without a `function`: {entry}"))
        })
        .collect();

    for expected in [
        "run",
        "cancel",
        "recent_runs",
        "read_run_log",
        "resolve_runtimes",
        "schemas",
    ] {
        assert!(
            functions.contains(&expected),
            "skill_runtime_schemas must advertise `{expected}`, got {functions:?}"
        );
    }
    assert_eq!(
        functions.len(),
        6,
        "the namespace has exactly six controllers; got {functions:?}"
    );

    // `resolve_runtimes` scoped to one runtime must return exactly that one,
    // with the documented status fields present. Availability depends on the
    // host toolchain, so the assertion is on shape and scoping, not on a
    // machine-specific `available: true`.
    let node_only = rpc(
        &harness.rpc_base,
        101,
        "openhuman.skill_runtime_resolve_runtimes",
        json!({ "runtime": "node" }),
    )
    .await;
    let node_only = ok(&node_only, "skill_runtime_resolve_runtimes node");
    let runtimes = node_only
        .get("runtimes")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("resolve_runtimes has no `runtimes` array: {node_only}"));
    assert_eq!(
        runtimes.len(),
        1,
        "scoping to `node` must resolve only node: {node_only}"
    );
    assert_eq!(
        runtimes[0].get("runtime").and_then(Value::as_str),
        Some("node"),
        "the single entry must be the node runtime: {node_only}"
    );
    for field in ["enabled", "available"] {
        assert!(
            runtimes[0].get(field).is_some_and(Value::is_boolean),
            "`{field}` must be present and boolean: {node_only}"
        );
    }

    // The default (`all`) resolves both slots.
    let all = rpc(
        &harness.rpc_base,
        102,
        "openhuman.skill_runtime_resolve_runtimes",
        json!({}),
    )
    .await;
    let all_names: Vec<&str> = ok(&all, "skill_runtime_resolve_runtimes all")
        .get("runtimes")
        .and_then(Value::as_array)
        .expect("runtimes array")
        .iter()
        .filter_map(|r| r.get("runtime").and_then(Value::as_str))
        .collect();
    assert!(
        all_names.contains(&"node") && all_names.contains(&"python"),
        "the default must resolve both runtimes, got {all_names:?}"
    );

    // Failure path: an unrecognised runtime is refused, and the error names the
    // three that are accepted rather than failing opaquely.
    let bogus = rpc(
        &harness.rpc_base,
        103,
        "openhuman.skill_runtime_resolve_runtimes",
        json!({ "runtime": "brainfuck" }),
    )
    .await;
    let message = error_message(&bogus, "skill_runtime_resolve_runtimes bogus");
    assert!(
        message.contains("brainfuck") && message.contains("node") && message.contains("python"),
        "the refusal must name the bad value and the accepted ones, got: {message}"
    );

    harness.join.abort();
}

/// `skill_runtime_run`'s two synchronous refusals, then `recent_runs`,
/// `read_run_log` and `cancel` against planted logs.
#[tokio::test]
async fn skill_runtime_run_validation_and_run_log_surface() {
    let _lock = env_lock();
    let harness = setup(Vec::new()).await;
    plant_skill(&harness.workspace, "planted-runtime-skill", "target_repo");

    // Refusal 1: an id no root resolves.
    let unknown = rpc(
        &harness.rpc_base,
        110,
        "openhuman.skill_runtime_run",
        json!({ "skill_id": "no-such-skill-anywhere" }),
    )
    .await;
    let message = error_message(&unknown, "skill_runtime_run unknown skill");
    assert!(
        message.contains("unknown skill") && message.contains("no-such-skill-anywhere"),
        "the refusal must name the unresolved skill, got: {message}"
    );

    // Refusal 2: the skill resolves, but a declared `required` input is absent.
    // This is the assertion that proves the planted skill's `skill.toml` was
    // actually parsed — a fallback SKILL.md-only definition declares no inputs
    // and would have started a run instead.
    let missing_input = rpc(
        &harness.rpc_base,
        111,
        "openhuman.skill_runtime_run",
        json!({ "skill_id": "planted-runtime-skill" }),
    )
    .await;
    let message = error_message(&missing_input, "skill_runtime_run missing input");
    assert!(
        message.contains("missing required inputs") && message.contains("target_repo"),
        "the refusal must name the missing input, got: {message}"
    );

    // An explicit null is treated as absent, not as a supplied value.
    let null_input = rpc(
        &harness.rpc_base,
        112,
        "openhuman.skill_runtime_run",
        json!({ "skill_id": "planted-runtime-skill", "inputs": { "target_repo": null } }),
    )
    .await;
    assert!(
        error_message(&null_input, "skill_runtime_run null input").contains("target_repo"),
        "a null input must count as missing: {null_input}"
    );

    // ── the run-log surface, against logs whose content is knowable ──────────
    let running_id = "aaaaaaaa-1111-2222-3333-444444444444";
    let done_id = "bbbbbbbb-1111-2222-3333-444444444444";
    plant_run_log(
        &harness.workspace,
        "planted-runtime-skill",
        running_id,
        "2026-09-07T10:10:10+00:00",
        None,
    );
    plant_run_log(
        &harness.workspace,
        "planted-runtime-skill",
        done_id,
        "2026-09-07T11:10:10+00:00",
        Some(("DONE", 4242, "PLANTED_RUN_OUTPUT")),
    );
    // A third run, under a different skill, to prove the filter bites.
    plant_run_log(
        &harness.workspace,
        "some-other-skill",
        "cccccccc-1111-2222-3333-444444444444",
        "2026-09-07T12:10:10+00:00",
        Some(("FAILED", 7, "nope")),
    );

    let recent = rpc(
        &harness.rpc_base,
        113,
        "openhuman.skill_runtime_recent_runs",
        json!({}),
    )
    .await;
    let recent = ok(&recent, "skill_runtime_recent_runs");
    let runs = recent
        .get("runs")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("recent_runs has no `runs` array: {recent}"));
    assert_eq!(
        runs.len(),
        3,
        "all three planted runs must be seen: {recent}"
    );
    // Sorted most-recent-first by `started`.
    assert_eq!(
        runs.iter()
            .filter_map(|r| r.get("workflow_id").and_then(Value::as_str))
            .collect::<Vec<_>>(),
        vec![
            "some-other-skill",
            "planted-runtime-skill",
            "planted-runtime-skill"
        ],
        "runs must come back newest-first by `started`: {recent}"
    );

    // The unfinished log has no footer, so its status is the RUNNING default
    // and it carries no duration; the finished one carries both.
    let by_id = |id: &str| -> Value {
        runs.iter()
            .find(|r| r.get("run_id").and_then(Value::as_str) == Some(id))
            .unwrap_or_else(|| panic!("run {id} missing from {recent}"))
            .clone()
    };
    let running = by_id(running_id);
    assert_eq!(
        running.get("status").and_then(Value::as_str),
        Some("RUNNING"),
        "a log with no `--- result ---` footer must read as RUNNING: {running}"
    );
    assert!(
        running.get("duration_ms").is_none_or(Value::is_null),
        "an unfinished run must not report a duration: {running}"
    );
    let done = by_id(done_id);
    assert_eq!(
        done.get("status").and_then(Value::as_str),
        Some("DONE"),
        "the footer's status word must be parsed out: {done}"
    );
    assert_eq!(
        done.get("duration_ms").and_then(Value::as_u64),
        Some(4242),
        "the footer's `duration: <n> ms` must be parsed to a number: {done}"
    );

    // Filtering scopes to one skill.
    let filtered = rpc(
        &harness.rpc_base,
        114,
        "openhuman.skill_runtime_recent_runs",
        json!({ "skill_id": "some-other-skill" }),
    )
    .await;
    let filtered = ok(&filtered, "skill_runtime_recent_runs filtered");
    let filtered_runs = filtered
        .get("runs")
        .and_then(Value::as_array)
        .expect("runs");
    assert_eq!(
        filtered_runs.len(),
        1,
        "the skill_id filter must exclude the other two: {filtered}"
    );
    assert_eq!(
        filtered_runs[0].get("workflow_id").and_then(Value::as_str),
        Some("some-other-skill")
    );

    // `limit` is honoured and clamped.
    let limited = rpc(
        &harness.rpc_base,
        115,
        "openhuman.skill_runtime_recent_runs",
        json!({ "limit": 1 }),
    )
    .await;
    assert_eq!(
        ok(&limited, "skill_runtime_recent_runs limit")
            .get("runs")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1),
        "limit must cap the result: {limited}"
    );

    // ── read_run_log ────────────────────────────────────────────────────────
    let slice = rpc(
        &harness.rpc_base,
        116,
        "openhuman.skill_runtime_read_run_log",
        json!({ "run_id": done_id }),
    )
    .await;
    let slice = ok(&slice, "skill_runtime_read_run_log");
    let content = slice
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("read_run_log has no `content`: {slice}"));
    assert!(
        content.contains("PLANTED_RUN_OUTPUT") && content.contains(done_id),
        "the slice must contain the log's own body and run id: {content}"
    );
    assert_eq!(
        slice.get("eof").and_then(Value::as_bool),
        Some(true),
        "a default 64 KiB read of a tiny log reaches EOF: {slice}"
    );
    assert_eq!(
        slice.get("complete").and_then(Value::as_bool),
        Some(true),
        "a log carrying the footer must report complete: {slice}"
    );
    let end_offset = slice
        .get("offset")
        .and_then(Value::as_u64)
        .expect("offset cursor");
    assert_eq!(
        slice.get("bytes_read").and_then(Value::as_u64),
        Some(end_offset),
        "reading from offset 0, the cursor must equal the bytes returned: {slice}"
    );

    // Tail-mode: re-issuing at the returned cursor yields nothing new but keeps
    // reporting `complete`, which is what stops the frontend polling.
    let tail = rpc(
        &harness.rpc_base,
        117,
        "openhuman.skill_runtime_read_run_log",
        json!({ "run_id": done_id, "offset": end_offset }),
    )
    .await;
    let tail = ok(&tail, "skill_runtime_read_run_log tail");
    assert_eq!(tail.get("bytes_read").and_then(Value::as_u64), Some(0));
    assert_eq!(tail.get("content").and_then(Value::as_str), Some(""));
    assert_eq!(tail.get("complete").and_then(Value::as_bool), Some(true));

    // A still-running log reports `complete: false` — the polling frontend
    // depends on that bit exactly.
    let running_slice = rpc(
        &harness.rpc_base,
        118,
        "openhuman.skill_runtime_read_run_log",
        json!({ "run_id": running_id }),
    )
    .await;
    assert_eq!(
        ok(&running_slice, "read_run_log running")
            .get("complete")
            .and_then(Value::as_bool),
        Some(false),
        "a footer-less log must not claim completion: {running_slice}"
    );

    // Failure path: an unknown run id resolves to no path and is refused.
    let unknown_run = rpc(
        &harness.rpc_base,
        119,
        "openhuman.skill_runtime_read_run_log",
        json!({ "run_id": "ffffffff-0000-0000-0000-000000000000" }),
    )
    .await;
    let message = error_message(&unknown_run, "read_run_log unknown run");
    assert!(
        message.contains("unknown run_id"),
        "the refusal must say the run id is unknown, got: {message}"
    );

    // ── cancel ──────────────────────────────────────────────────────────────
    // No live run holds a cancellation token, so this is the documented
    // "already finished or never existed" answer: reported, not thrown.
    let cancel = rpc(
        &harness.rpc_base,
        120,
        "openhuman.skill_runtime_cancel",
        json!({ "run_id": done_id }),
    )
    .await;
    let cancel = ok(&cancel, "skill_runtime_cancel");
    assert_eq!(
        cancel.get("run_id").and_then(Value::as_str),
        Some(done_id),
        "cancel must echo the requested run id: {cancel}"
    );
    assert_eq!(
        cancel.get("cancelled").and_then(Value::as_bool),
        Some(false),
        "cancelling a run with no live token must report false, not error: {cancel}"
    );

    harness.join.abort();
}

// ── skills (the uncovered half) ─────────────────────────────────────────────

/// `skills_run` / `skills_recent_runs` / `skills_read_run_log` / `skills_cancel`
/// — the older namespace over the same machinery, asserted at the points where
/// its wire shape differs from `skill_runtime_*`.
#[tokio::test]
async fn skills_run_and_log_surface_keeps_its_own_wire_shape() {
    let _lock = env_lock();
    let harness = setup(Vec::new()).await;
    plant_skill(&harness.workspace, "planted-legacy-skill", "ticket");

    let unknown = rpc(
        &harness.rpc_base,
        130,
        "openhuman.skills_run",
        json!({ "workflow_id": "not-a-skill" }),
    )
    .await;
    assert!(
        error_message(&unknown, "skills_run unknown").contains("unknown skill"),
        "skills_run must refuse an unresolved id: {unknown}"
    );

    // `skills_run` takes `workflow_id` where `skill_runtime_run` takes
    // `skill_id`; sending the sibling's spelling is a params error, which is
    // worth pinning because the two namespaces front the same machinery.
    let wrong_param = rpc(
        &harness.rpc_base,
        131,
        "openhuman.skills_run",
        json!({ "skill_id": "planted-legacy-skill" }),
    )
    .await;
    assert!(
        error_message(&wrong_param, "skills_run wrong param").contains("workflow_id"),
        "skills_run must name the param it wanted: {wrong_param}"
    );

    let missing_input = rpc(
        &harness.rpc_base,
        132,
        "openhuman.skills_run",
        json!({ "workflow_id": "planted-legacy-skill" }),
    )
    .await;
    assert!(
        error_message(&missing_input, "skills_run missing input").contains("ticket"),
        "skills_run must name the missing required input: {missing_input}"
    );

    let run_id = "dddddddd-1111-2222-3333-444444444444";
    plant_run_log(
        &harness.workspace,
        "planted-legacy-skill",
        run_id,
        "2026-09-07T13:10:10+00:00",
        Some(("DEGENERATE", 99, "PLANTED_LEGACY_OUTPUT")),
    );

    let recent = rpc(
        &harness.rpc_base,
        133,
        "openhuman.skills_recent_runs",
        json!({ "workflow_id": "planted-legacy-skill" }),
    )
    .await;
    let recent = ok(&recent, "skills_recent_runs");
    let runs = recent
        .get("runs")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("skills_recent_runs has no `runs`: {recent}"));
    assert_eq!(runs.len(), 1, "one planted run for this skill: {recent}");
    assert_eq!(
        runs[0].get("status").and_then(Value::as_str),
        Some("DEGENERATE"),
        "a DEGENERATE footer must survive the scan verbatim: {recent}"
    );
    assert_eq!(runs[0].get("run_id").and_then(Value::as_str), Some(run_id));

    let slice = rpc(
        &harness.rpc_base,
        134,
        "openhuman.skills_read_run_log",
        json!({ "run_id": run_id, "max_bytes": 32 }),
    )
    .await;
    let slice = ok(&slice, "skills_read_run_log capped");
    assert_eq!(
        slice.get("bytes_read").and_then(Value::as_u64),
        Some(32),
        "an explicit max_bytes must cap the slice at exactly that many bytes: {slice}"
    );
    assert_eq!(
        slice.get("eof").and_then(Value::as_bool),
        Some(false),
        "a capped read short of the file end must not claim EOF: {slice}"
    );
    assert!(
        slice
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(|c| c.starts_with("==== workflow_run:")),
        "a read from offset 0 must start at the header banner: {slice}"
    );

    let cancel = rpc(
        &harness.rpc_base,
        135,
        "openhuman.skills_cancel",
        json!({ "run_id": "not-a-live-run" }),
    )
    .await;
    let cancel = ok(&cancel, "skills_cancel");
    assert_eq!(
        cancel.get("cancelled").and_then(Value::as_bool),
        Some(false),
        "an unknown run id is reported as not-cancelled, not as an error: {cancel}"
    );
    assert_eq!(
        cancel.get("run_id").and_then(Value::as_str),
        Some("not-a-live-run")
    );

    harness.join.abort();
}

/// `skills_read_resource` — the happy read plus the traversal and symlink
/// guards, which are the reason this controller has a hand-written path jail
/// rather than a plain `fs::read`.
#[tokio::test]
async fn skills_read_resource_reads_a_bundle_file_and_refuses_escapes() {
    let _lock = env_lock();
    let harness = setup(Vec::new()).await;
    plant_skill(&harness.workspace, "planted-resource-skill", "unused");

    let read = rpc(
        &harness.rpc_base,
        140,
        "openhuman.skills_read_resource",
        json!({ "workflow_id": "planted-resource-skill", "relative_path": "references/note.md" }),
    )
    .await;
    let read = ok(&read, "skills_read_resource");
    assert_eq!(
        read.get("content").and_then(Value::as_str),
        Some("PLANTED_RESOURCE_BODY\n"),
        "the resource body must come back verbatim: {read}"
    );
    assert_eq!(
        read.get("bytes").and_then(Value::as_u64),
        Some("PLANTED_RESOURCE_BODY\n".len() as u64),
        "`bytes` must be the on-disk size, not a rounded or char count: {read}"
    );
    assert_eq!(
        read.get("relative_path").and_then(Value::as_str),
        Some("references/note.md"),
        "the request path must be echoed back: {read}"
    );
    assert_eq!(
        read.get("workflow_id").and_then(Value::as_str),
        Some("planted-resource-skill")
    );

    // Traversal: `..` is rejected on the component walk, before any filesystem
    // call — so this must fail even though the target exists.
    let traversal = rpc(
        &harness.rpc_base,
        141,
        "openhuman.skills_read_resource",
        json!({ "workflow_id": "planted-resource-skill", "relative_path": "../../../etc/passwd" }),
    )
    .await;
    let message = error_message(&traversal, "skills_read_resource traversal");
    assert!(
        message.contains(".."),
        "the traversal refusal must name the offending component, got: {message}"
    );

    // An absolute path is a separate rejection from `..`.
    let absolute = rpc(
        &harness.rpc_base,
        142,
        "openhuman.skills_read_resource",
        json!({ "workflow_id": "planted-resource-skill", "relative_path": "/etc/passwd" }),
    )
    .await;
    let message = error_message(&absolute, "skills_read_resource absolute");
    assert!(
        message.contains("relative"),
        "an absolute path must be refused for being absolute, got: {message}"
    );

    // A symlinked leaf pointing outside the bundle is caught by the
    // `symlink_metadata` pre-check, not followed.
    #[cfg(unix)]
    {
        let outside = harness.workspace.join("outside-secret.txt");
        std::fs::write(&outside, "SHOULD_NOT_BE_READABLE\n").expect("write outside file");
        let link = harness
            .workspace
            .join("skills")
            .join("planted-resource-skill")
            .join("references")
            .join("escape.md");
        std::os::unix::fs::symlink(&outside, &link).expect("create symlink");

        let symlinked = rpc(
            &harness.rpc_base,
            143,
            "openhuman.skills_read_resource",
            json!({ "workflow_id": "planted-resource-skill", "relative_path": "references/escape.md" }),
        )
        .await;
        let message = error_message(&symlinked, "skills_read_resource symlink");
        assert!(
            message.contains("symlink"),
            "a symlinked leaf must be refused as a symlink, got: {message}"
        );
        assert!(
            !message.contains("SHOULD_NOT_BE_READABLE"),
            "the refusal must not leak the target's contents: {message}"
        );
    }

    // An unknown skill id is refused before any path work.
    let unknown = rpc(
        &harness.rpc_base,
        144,
        "openhuman.skills_read_resource",
        json!({ "workflow_id": "no-such-skill", "relative_path": "references/note.md" }),
    )
    .await;
    assert!(
        unknown.get("error").is_some(),
        "reading a resource of an unknown skill must error: {unknown}"
    );

    harness.join.abort();
}

/// `skills_install_from_url` — the SSRF guards it exists to enforce, then the
/// loopback happy path behind the documented escape hatch.
#[tokio::test]
async fn skills_install_from_url_enforces_its_url_policy_then_installs() {
    let _lock = env_lock();
    let (fixture_addr, fixture_join) = serve_fixture_catalog().await;
    let harness = setup(vec![EnvVarGuard::set(
        "OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP",
        "1",
    )])
    .await;

    // Guard 1: plain HTTP to a non-loopback host is refused even with the
    // local-HTTP hatch open — the hatch is loopback-only.
    let plain_http = rpc(
        &harness.rpc_base,
        150,
        "openhuman.skills_install_from_url",
        json!({ "url": "http://example.com/skill.md" }),
    )
    .await;
    let message = error_message(&plain_http, "install_from_url plain http");
    assert!(
        message.contains("https"),
        "a non-loopback http URL must be refused for its scheme, got: {message}"
    );

    // Guard 2: an https URL whose host is a private/loopback literal is the
    // SSRF vector this controller's host check exists for.
    for host in ["127.0.0.1", "10.0.0.1", "169.254.169.254"] {
        let private = rpc(
            &harness.rpc_base,
            151,
            "openhuman.skills_install_from_url",
            json!({ "url": format!("https://{host}/skill.md") }),
        )
        .await;
        let message = error_message(&private, "install_from_url private host");
        assert!(
            message.contains(host) && message.contains("not allowed"),
            "https://{host} must be refused by name, got: {message}"
        );
    }

    // Guard 3: an https URL that is not a single `.md` file.
    let not_md = rpc(
        &harness.rpc_base,
        152,
        "openhuman.skills_install_from_url",
        json!({ "url": "https://example.com/some/repo" }),
    )
    .await;
    let message = error_message(&not_md, "install_from_url non-md");
    assert!(
        message.contains("unsupported url form"),
        "a non-`.md` target must be refused as an unsupported form, got: {message}"
    );

    // Happy path: loopback HTTP with the hatch open, against the fixture.
    let installed = rpc(
        &harness.rpc_base,
        153,
        "openhuman.skills_install_from_url",
        json!({
            "url": format!("http://127.0.0.1:{}/skills/url-installed-skill/SKILL.md", fixture_addr.port()),
            "timeout_secs": 30
        }),
    )
    .await;
    let installed = ok(&installed, "skills_install_from_url happy path");
    assert!(
        installed
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(|u| u.contains("url-installed-skill")),
        "the installed URL must be echoed: {installed}"
    );
    // NOTE the field name. The controller schema declares this output as
    // `new_skills`; the handler emits `new_workflows`. See
    // `~/tinyhuman/bugs/e2e-wave-skills-schema-output-name-drift.md`. Asserted
    // against what the code does, and against what the frontend reads
    // (`app/src/services/api/skillsApi.ts:400`).
    let new_workflows = installed
        .get("new_workflows")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("install result has no `new_workflows`: {installed}"));
    assert!(
        new_workflows
            .iter()
            .any(|s| s.as_str() == Some("url-installed-skill")),
        "the freshly installed slug must appear in `new_workflows`: {installed}"
    );
    assert!(
        installed.get("new_skills").is_none(),
        "the schema's declared `new_skills` is NOT what the handler emits; if this \
         starts passing the schema and the handler have been reconciled and this \
         suite plus the bug note need updating: {installed}"
    );

    // Installing the same URL again is an idempotent success reporting no new
    // slugs — not a duplicate-directory error.
    let again = rpc(
        &harness.rpc_base,
        154,
        "openhuman.skills_install_from_url",
        json!({
            "url": format!("http://127.0.0.1:{}/skills/url-installed-skill/SKILL.md", fixture_addr.port())
        }),
    )
    .await;
    let again = ok(&again, "skills_install_from_url idempotent");
    assert_eq!(
        again
            .get("new_workflows")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0),
        "a repeat install must report zero new slugs: {again}"
    );

    fixture_join.abort();
    harness.join.abort();
}

// ── skill_registry ──────────────────────────────────────────────────────────

/// `skill_registry_categories` — the distinct, sorted, non-empty categories of
/// the catalog.
#[tokio::test]
async fn skill_registry_categories_are_distinct_sorted_and_drop_the_blank() {
    let _lock = env_lock();
    let (fixture_addr, fixture_join) = serve_fixture_catalog().await;
    let fixture_base = format!("http://{fixture_addr}");
    let harness = setup(vec![
        EnvVarGuard::set(
            "OPENHUMAN_SKILL_REGISTRY_CATALOG_URL",
            &format!("{fixture_base}/skills.json"),
        ),
        EnvVarGuard::set(
            "OPENHUMAN_SKILL_REGISTRY_DOWNLOAD_BASE_URL",
            &format!("{fixture_base}/skills"),
        ),
    ])
    .await;

    let response = rpc(
        &harness.rpc_base,
        160,
        "openhuman.skill_registry_categories",
        json!({}),
    )
    .await;
    let response = ok(&response, "skill_registry_categories");
    let categories: Vec<&str> = response
        .get("categories")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no `categories` array: {response}"))
        .iter()
        .map(|c| {
            c.as_str()
                .unwrap_or_else(|| panic!("a category must be a string: {c}"))
        })
        .collect();

    assert!(
        categories.contains(&"software-development") && categories.contains(&"productivity"),
        "both fixture categories must be present, got {categories:?}"
    );
    assert!(
        !categories.iter().any(|c| c.is_empty()),
        "the blank category must be filtered out, got {categories:?}"
    );
    let mut sorted = categories.clone();
    sorted.sort_unstable();
    assert_eq!(
        categories, sorted,
        "categories must come back sorted, got {categories:?}"
    );
    let mut deduped = sorted.clone();
    deduped.dedup();
    assert_eq!(
        categories.len(),
        deduped.len(),
        "categories must be distinct, got {categories:?}"
    );

    fixture_join.abort();
    harness.join.abort();
}

// ── javascript ──────────────────────────────────────────────────────────────

/// `javascript_list_tools` and `javascript_execute_tool` — the runtime bridge
/// the agent reaches its tool registry through.
///
/// The happy path executes a real tool (`file_read`) against a file this test
/// wrote into the action sandbox, so it proves the whole chain: build the
/// registry, resolve by name, execute, and wrap the result in the MCP-style
/// envelope.
#[tokio::test]
async fn javascript_bridge_lists_and_executes_a_real_tool() {
    let _lock = env_lock();
    let harness = setup(Vec::new()).await;

    let listed = rpc(
        &harness.rpc_base,
        170,
        "openhuman.javascript_list_tools",
        json!({}),
    )
    .await;
    // Assert the envelope explicitly before unwrapping: this handler always
    // logs, so it is always wrapped, and that is the shape a caller must parse.
    let listed_raw = ok(&listed, "javascript_list_tools");
    assert!(
        listed_raw.get("logs").is_some_and(Value::is_array),
        "a handler that logs must return the `{{result, logs}}` envelope: {listed_raw}"
    );
    let listed = payload(&listed, "javascript_list_tools");
    let tools = listed
        .get("tools")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no `tools` array: {listed}"));
    assert!(
        !tools.is_empty(),
        "the bridge must expose a non-empty tool registry: {listed}"
    );

    let file_read = tools
        .iter()
        .find(|t| t.get("name").and_then(Value::as_str) == Some("file_read"))
        .unwrap_or_else(|| {
            panic!(
                "`file_read` must be in the registry; got {:?}",
                tools
                    .iter()
                    .filter_map(|t| t.get("name").and_then(Value::as_str))
                    .collect::<Vec<_>>()
            )
        });
    // The schema documents each summary's fields; a caller renders from them.
    for field in ["description", "permission_level", "parameters"] {
        assert!(
            file_read.get(field).is_some(),
            "a tool summary must carry `{field}`: {file_read}"
        );
    }
    // Sorted by name, per `ops::list_tools`.
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t.get("name").and_then(Value::as_str))
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted, "the tool list must be sorted by name");

    // Happy path: read a file the action sandbox actually holds.
    let action_dir = std::env::var("OPENHUMAN_ACTION_DIR").expect("action dir set by setup()");
    std::fs::write(
        Path::new(&action_dir).join("bridge-fixture.txt"),
        "BRIDGE_FIXTURE_BODY\n",
    )
    .expect("write action-dir fixture");

    let executed = rpc(
        &harness.rpc_base,
        171,
        "openhuman.javascript_execute_tool",
        json!({ "tool_name": "file_read", "args": { "path": "bridge-fixture.txt" } }),
    )
    .await;
    let executed = payload(&executed, "javascript_execute_tool file_read");
    assert_eq!(
        executed.get("tool_name").and_then(Value::as_str),
        Some("file_read"),
        "the executed tool name must be echoed: {executed}"
    );
    assert!(
        executed.get("elapsed_ms").is_some_and(Value::is_number),
        "`elapsed_ms` must be a number: {executed}"
    );
    let result = executed
        .get("result")
        .unwrap_or_else(|| panic!("no `result` envelope: {executed}"));
    assert_eq!(
        result.get("is_error").and_then(Value::as_bool),
        Some(false),
        "reading a file that exists must not be an error result: {result}"
    );
    assert!(
        result.to_string().contains("BRIDGE_FIXTURE_BODY"),
        "the tool's output must carry the file's contents: {result}"
    );

    // Failure path 1: a registered tool given bad args fails *inside* the
    // envelope — `is_error: true`, not a JSON-RPC error.
    let bad_args = rpc(
        &harness.rpc_base,
        172,
        "openhuman.javascript_execute_tool",
        json!({ "tool_name": "file_read", "args": { "path": "definitely-not-here.txt" } }),
    )
    .await;
    let bad_args = payload(&bad_args, "javascript_execute_tool bad args");
    assert_eq!(
        bad_args.get("tool_name").and_then(Value::as_str),
        Some("file_read"),
        "a failing execution still echoes the tool name: {bad_args}"
    );
    assert_eq!(
        bad_args
            .get("result")
            .and_then(|r| r.get("is_error"))
            .and_then(Value::as_bool),
        Some(true),
        "a tool-level failure must surface as `is_error` in the envelope, not as \
         a transport error: {bad_args}"
    );

    // Failure path 2: a name the registry does not hold is a dispatch error,
    // which is a different outcome from the above and must stay different.
    let unknown = rpc(
        &harness.rpc_base,
        173,
        "openhuman.javascript_execute_tool",
        json!({ "tool_name": "no_such_tool_anywhere", "args": {} }),
    )
    .await;
    let message = error_message(&unknown, "javascript_execute_tool unknown tool");
    assert!(
        message.contains("unknown tool") && message.contains("no_such_tool_anywhere"),
        "an unregistered name must be refused by name, got: {message}"
    );

    // Failure path 3: the required param is missing entirely.
    let no_name = rpc(
        &harness.rpc_base,
        174,
        "openhuman.javascript_execute_tool",
        json!({ "args": {} }),
    )
    .await;
    assert!(
        error_message(&no_name, "javascript_execute_tool no name").contains("tool_name"),
        "an absent `tool_name` must be named in the error: {no_name}"
    );

    harness.join.abort();
}
