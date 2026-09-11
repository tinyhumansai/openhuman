//! JSON-RPC E2E coverage for three memory-family namespaces that had none:
//! `memory_goals` (5 controllers), `people` (4), and the four uncovered
//! `tree_summarizer` reads/passes (`query`, `status`, `run`, `rebuild`).
//!
//! All three sit behind the bound memory driver rather than a path this host
//! owns, so every case drives them over the real axum JSON-RPC router and
//! asserts on what came back through the driver — never on a file the test
//! wrote itself.
//!
//! This file is a **module** of the aggregated `raw_coverage_all` target, not a
//! target of its own — `build.rs` globs `tests/raw_coverage/` and generates the
//! `mod` list. Run with:
//!   `~/tinyhuman/ci-slot.sh cargo test --test raw_coverage_all \
//!      --features "$(bash scripts/ci/product-features.sh)" -- memory_goals_people`
//!
//! ## Written around a shared process and a shared store
//!
//! ~77 suites share one process here, so the env lock is the crate-wide
//! `SHARED_ENV_LOCK` and the RPC bearer is read back from
//! `core::auth::get_rpc_token()` rather than assumed — a private lock would
//! isolate nothing and a hard-coded token would 401 whenever a sibling suite
//! seeded the process-global one first.
//!
//! The goals document is workspace-wide and the people store is shared, so
//! every case keys on a distinctive `e2e-…` marker and asserts on **its own**
//! rows: a goal is added, found by the id the handler assigned, edited, and
//! deleted. No case asserts a global count, because a sibling suite's row would
//! make that assertion fail for a reason unrelated to the controller.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::http::header::AUTHORIZATION;
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::TempDir;

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;
use openhuman_core::openhuman::config::Config;

/// Preferred bearer. Only the real one if this module wins the process-global
/// `OnceLock` race — send [`rpc_bearer`], never this.
const PREFERRED_RPC_TOKEN: &str = "memory-goals-people-e2e-token";

static AUTH_INIT: OnceLock<()> = OnceLock::new();
static MEMORY_SEAMS_INIT: OnceLock<()> = OnceLock::new();

/// The crate-wide env lock — see the module docs.
static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ── Env isolation ─────────────────────────────────────────────────────────

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

// ── Shared memory workspace ───────────────────────────────────────────────

/// The transport-only JSON-RPC router builds no core runtime context, so a
/// memory-backed route has no seams unless they are installed explicitly.
fn ensure_memory_seams() {
    MEMORY_SEAMS_INIT.get_or_init(|| {
        std::thread::Builder::new()
            .name("memory-goals-people-e2e-seams".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                let config = Arc::new(shared_config_at(memory_workspace()));
                #[cfg(feature = "modules")]
                openhuman_core::openhuman::modules::memory::set_modules_policy(config);
            })
            .expect("spawn memory goals/people seam installer")
            .join()
            .expect("memory goals/people seam installer panicked");
    });
}

/// The one memory workspace every case in this module shares.
///
/// The module host captures a workspace **once per process**: the loaded
/// artifact takes its `workspace_dir` at load time and later policy calls are
/// ignored. A per-case `TempDir` would be deleted when its case returned,
/// leaving every later read answering from a dead store — 0 rows where a case
/// had just written one. One leaked directory, published once, is the same
/// arrangement `tests/json_rpc_e2e.rs` uses. The path ends in `workspace` so
/// `resolve_config_dir_for_workspace` treats it as the workspace itself rather
/// than appending another segment.
fn memory_workspace() -> &'static Path {
    static WORKSPACE: OnceLock<PathBuf> = OnceLock::new();
    WORKSPACE.get_or_init(|| {
        let dir = TempDir::new().expect("memory workspace tempdir");
        let path = dir.path().join("workspace");
        std::fs::create_dir_all(&path).expect("create memory workspace");
        // Leaked on purpose: the module keeps this path for the process lifetime.
        std::mem::forget(dir);
        path
    })
}

/// A config whose workspace **and** config path are the shared ones.
fn shared_config_at(workspace: &Path) -> Config {
    let mut config = Config::default();
    config.workspace_dir = workspace.to_path_buf();
    config.config_path = workspace
        .parent()
        .expect("shared workspace has a parent")
        .join("config.toml");
    config.embeddings_provider = Some("none".into());
    config
}

/// The config on disk. `local_ai.enabled = false` and no
/// `memory_tree.cloud_summarization_opt_in` are the *defaults* and are written
/// out explicitly, because the tree-summarizer consent gate below asserts on
/// the refusal they produce.
fn write_min_config(config_path: &Path) {
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).expect("create config dir");
    }
    let cfg = r#"api_url = "http://127.0.0.1:9"
default_model = "e2e-model"
default_temperature = 0.2

[secrets]
encrypt = false

[local_ai]
enabled = false

[memory_tree]
embedding_strict = false
cloud_summarization_opt_in = false
"#;
    std::fs::write(config_path, cfg).expect("write config.toml");
    let _: Config = toml::from_str(cfg).expect("test config must match schema");
}

// ── Harness ───────────────────────────────────────────────────────────────

fn ensure_rpc_auth() {
    AUTH_INIT.get_or_init(|| {
        std::env::set_var(CORE_TOKEN_ENV_VAR, PREFERRED_RPC_TOKEN);
        let token_dir = std::env::temp_dir().join("openhuman-memory-goals-people-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
}

/// The bearer the running process actually validates — see the module docs.
fn rpc_bearer() -> &'static str {
    ensure_rpc_auth();
    openhuman_core::core::auth::get_rpc_token()
        .expect("the RPC token must be initialised before a request is signed")
}

struct Harness {
    _guards: Vec<EnvVarGuard>,
    rpc_base: String,
    join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

async fn serve_rpc() -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    ensure_rpc_auth();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc listener");
    let addr = listener.local_addr().expect("rpc listener addr");
    let join =
        tokio::spawn(async move { axum::serve(listener, build_core_http_router(false)).await });
    (addr, join)
}

async fn setup() -> Harness {
    ensure_memory_seams();
    let workspace = memory_workspace().to_path_buf();
    let config_path = workspace
        .parent()
        .expect("shared workspace has a parent")
        .join("config.toml");
    write_min_config(&config_path);

    let guards = vec![
        EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
    ];

    let (addr, join) = serve_rpc().await;
    Harness {
        _guards: guards,
        rpc_base: format!("http://{addr}"),
        join,
    }
}

async fn rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("client");
    let url = format!("{}/rpc", rpc_base.trim_end_matches('/'));
    let response = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {}", rpc_bearer()))
        .json(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
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

/// The payload of a successful dispatch, unwrapping the `RpcOutcome`
/// `{ result, logs }` envelope when the handler produced one.
fn payload(value: &Value, context: &str) -> Value {
    if let Some(error) = value.get("error") {
        panic!("{context}: unexpected JSON-RPC error: {error}");
    }
    let outer = value
        .get("result")
        .unwrap_or_else(|| panic!("{context}: missing result: {value}"));
    match outer.get("result") {
        Some(inner) => inner.clone(),
        None => outer.clone(),
    }
}

fn error_message(value: &Value, context: &str) -> String {
    value
        .get("error")
        .unwrap_or_else(|| panic!("{context}: expected a JSON-RPC error, got: {value}"))
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: error object has no message: {value}"))
        .to_string()
}

/// The `items` array of a goals document, wherever the handler put it.
///
/// `list` answers the bare `GoalsDoc`; `add` answers `{ id, goals }`. Both
/// shapes are a published compatibility surface, so this reads either rather
/// than normalising one into the other.
fn goal_items(payload: &Value, context: &str) -> Vec<Value> {
    let doc = payload.get("goals").unwrap_or(payload);
    doc.get("items")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{context}: expected a goals items array: {payload}"))
        .clone()
}

fn find_goal<'a>(items: &'a [Value], id: &str) -> Option<&'a Value> {
    items
        .iter()
        .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
}

// ── memory_goals ──────────────────────────────────────────────────────────

/// The goals CRUD arc over RPC: add mints an id, list sees it, edit rewrites
/// the text in place, delete removes it, and list no longer sees it.
///
/// One case rather than four: the document is a single workspace-wide object,
/// so separate cases would either depend on each other's leftovers or leak a
/// goal for the next suite to trip over.
#[tokio::test]
async fn memory_goals_add_list_edit_delete_round_trip() {
    let _lock = env_lock();
    let harness = setup().await;

    let original = "e2e-goals-marker: keep the RPC surface honest";
    let rewritten = "e2e-goals-marker: keep the RPC surface honest and small";

    // ── add: the handler assigns the id, the caller does not ──
    let added = rpc(
        &harness.rpc_base,
        41_001,
        "openhuman.memory_goals_add",
        json!({ "text": original }),
    )
    .await;
    let added = payload(&added, "memory_goals_add");
    let goal_id = added
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("add must return the assigned id: {added}"))
        .to_string();
    assert!(
        goal_id.starts_with('g'),
        "ids come from the contract's `g<N>` allocator, got {goal_id}"
    );
    let after_add = goal_items(&added, "memory_goals_add");
    assert_eq!(
        find_goal(&after_add, &goal_id)
            .and_then(|g| g.get("text"))
            .and_then(Value::as_str),
        Some(original),
        "add returns the updated list read back through the driver: {added}"
    );

    // ── list: an independent read sees the same row ──
    let listed = rpc(
        &harness.rpc_base,
        41_002,
        "openhuman.memory_goals_list",
        json!({}),
    )
    .await;
    let listed_items = goal_items(&payload(&listed, "memory_goals_list"), "memory_goals_list");
    assert_eq!(
        find_goal(&listed_items, &goal_id)
            .and_then(|g| g.get("text"))
            .and_then(Value::as_str),
        Some(original),
        "the goal must be durable, not a value echoed by add"
    );

    // ── edit: rewrites in place, keeping the id ──
    let edited = rpc(
        &harness.rpc_base,
        41_003,
        "openhuman.memory_goals_edit",
        json!({ "id": goal_id, "text": rewritten }),
    )
    .await;
    let edited_items = goal_items(&payload(&edited, "memory_goals_edit"), "memory_goals_edit");
    assert_eq!(
        find_goal(&edited_items, &goal_id)
            .and_then(|g| g.get("text"))
            .and_then(Value::as_str),
        Some(rewritten),
        "edit must replace the text under the same id: {edited}"
    );
    assert_eq!(
        edited_items
            .iter()
            .filter(|g| g.get("id").and_then(Value::as_str) == Some(goal_id.as_str()))
            .count(),
        1,
        "edit must not duplicate the row it rewrote"
    );

    // ── delete: removes it, and a following list confirms ──
    let deleted = rpc(
        &harness.rpc_base,
        41_004,
        "openhuman.memory_goals_delete",
        json!({ "id": goal_id }),
    )
    .await;
    let deleted_items = goal_items(
        &payload(&deleted, "memory_goals_delete"),
        "memory_goals_delete",
    );
    assert!(
        find_goal(&deleted_items, &goal_id).is_none(),
        "delete must return the list without the deleted goal: {deleted}"
    );

    let final_list = rpc(
        &harness.rpc_base,
        41_005,
        "openhuman.memory_goals_list",
        json!({}),
    )
    .await;
    let final_items = goal_items(
        &payload(&final_list, "memory_goals_list final"),
        "memory_goals_list final",
    );
    assert!(
        find_goal(&final_items, &goal_id).is_none(),
        "the delete must be durable, not just reflected in its own reply"
    );

    harness.join.abort();
}

/// The goals validation boundary, which is deliberately **host-side**: a goal
/// that carries a secret or an email address must be refused before it reaches
/// the driver, and an unknown id must be a `NotFound` rather than a silent
/// no-op.
#[tokio::test]
async fn memory_goals_refuse_pii_secrets_blank_text_and_unknown_ids() {
    let _lock = env_lock();
    let harness = setup().await;

    // Empty / whitespace-only text.
    let blank = rpc(
        &harness.rpc_base,
        41_101,
        "openhuman.memory_goals_add",
        json!({ "text": "   " }),
    )
    .await;
    assert!(
        !error_message(&blank, "memory_goals_add blank").is_empty(),
        "a blank goal must be refused: {blank}"
    );

    // A multi-line goal — the contract is one concise sentence.
    let multiline = rpc(
        &harness.rpc_base,
        41_102,
        "openhuman.memory_goals_add",
        json!({ "text": "first line\nsecond line" }),
    )
    .await;
    assert!(
        !error_message(&multiline, "memory_goals_add multiline").is_empty(),
        "a multi-line goal must be refused: {multiline}"
    );

    // An email address is PII by the host's own predicate.
    let pii = rpc(
        &harness.rpc_base,
        41_103,
        "openhuman.memory_goals_add",
        json!({ "text": "email the report to someone@example.com every Friday" }),
    )
    .await;
    let pii_message = error_message(&pii, "memory_goals_add pii");
    assert!(
        pii_message.contains("secrets or PII"),
        "the refusal must say why, got: {pii_message}"
    );

    // The type contract. This is `core::all::validate_params`' wording, not the
    // handler's: every dispatch is schema-validated for required-presence and
    // declared types before the handler body runs (`src/core/all.rs:1334`), so
    // `parse_value`'s own "invalid params: …" is unreachable over RPC.
    let missing_text = rpc(
        &harness.rpc_base,
        41_104,
        "openhuman.memory_goals_add",
        json!({}),
    )
    .await;
    assert!(
        error_message(&missing_text, "memory_goals_add without text")
            .contains("missing required param 'text'"),
        "a missing required field is refused by name before the handler runs: {missing_text}"
    );

    // Unknown ids on both mutators.
    for (id, method, context) in [
        (41_105_i64, "openhuman.memory_goals_edit", "edit"),
        (41_106, "openhuman.memory_goals_delete", "delete"),
    ] {
        let mut params = json!({ "id": "g-e2e-never-allocated" });
        if method.ends_with("edit") {
            params
                .as_object_mut()
                .expect("params object")
                .insert("text".into(), json!("anything at all"));
        }
        let response = rpc(&harness.rpc_base, id, method, params).await;
        let message = error_message(&response, context);
        assert!(
            message.contains("g-e2e-never-allocated"),
            "{context} on an unknown id must name it, got: {message}"
        );
    }

    harness.join.abort();
}

/// `memory_goals.reflect` runs the enrichment agent on demand. With no
/// reachable provider it must report the failure **in band** — `ran: false`
/// plus a summary — and still hand back the current list, because the caller
/// asked for the list and a second failure reading it back must not replace the
/// report of the first.
#[tokio::test]
async fn memory_goals_reflect_reports_a_failed_run_without_losing_the_list() {
    let _lock = env_lock();
    let harness = setup().await;

    let reflected = rpc(
        &harness.rpc_base,
        41_201,
        "openhuman.memory_goals_reflect",
        json!({ "context": "e2e-goals-reflect: review nothing in particular" }),
    )
    .await;
    let reflected = payload(&reflected, "memory_goals_reflect");

    assert!(
        reflected.get("ran").and_then(Value::as_bool).is_some(),
        "reflect must always report whether the agent ran: {reflected}"
    );
    assert!(
        reflected
            .get("summary")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty()),
        "reflect must always carry a human-readable summary: {reflected}"
    );
    // The list comes back either way — that is the whole point of the shape.
    assert!(
        reflected
            .get("goals")
            .and_then(|g| g.get("items"))
            .and_then(Value::as_array)
            .is_some(),
        "reflect must return the goals list alongside its outcome: {reflected}"
    );

    if reflected.get("ran").and_then(Value::as_bool) == Some(false) {
        assert!(
            reflected
                .get("summary")
                .and_then(Value::as_str)
                .is_some_and(|s| s.contains("enrichment failed")),
            "a failed run must say so rather than reporting a bland summary: {reflected}"
        );
    }

    harness.join.abort();
}

// ── people ────────────────────────────────────────────────────────────────

/// The people surface over RPC: minting a person from a handle, resolving that
/// handle again to the same id, scoring them, and finding them in the ranked
/// list.
#[tokio::test]
async fn people_resolve_mints_then_score_and_list_agree_on_the_person() {
    let _lock = env_lock();
    let harness = setup().await;

    let handle = "e2e-people@example.test";

    // ── resolve without create_if_missing: an unknown handle is not minted ──
    let unknown = rpc(
        &harness.rpc_base,
        42_001,
        "openhuman.people_resolve",
        json!({ "kind": "email", "value": handle }),
    )
    .await;
    let unknown = payload(&unknown, "people_resolve without create");
    assert_eq!(
        unknown.get("created").and_then(Value::as_bool),
        Some(false),
        "resolve must not mint unless asked: {unknown}"
    );
    assert_eq!(
        unknown.get("person_id"),
        Some(&Value::Null),
        "an unresolved handle answers a null id, not an error: {unknown}"
    );

    // ── resolve with create_if_missing: mints once ──
    let minted = rpc(
        &harness.rpc_base,
        42_002,
        "openhuman.people_resolve",
        json!({ "kind": "email", "value": handle, "create_if_missing": true }),
    )
    .await;
    let minted = payload(&minted, "people_resolve create");
    let person_id = minted
        .get("person_id")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("minting must return an id: {minted}"))
        .to_string();
    assert!(
        uuid_shaped(&person_id),
        "a PersonId is a UUID, got {person_id}"
    );
    assert_eq!(
        minted.get("created").and_then(Value::as_bool),
        Some(true),
        "the first resolve-with-create reports the mint: {minted}"
    );

    // ── resolving again is idempotent: same id, not created twice ──
    let again = rpc(
        &harness.rpc_base,
        42_003,
        "openhuman.people_resolve",
        json!({ "kind": "email", "value": handle, "create_if_missing": true }),
    )
    .await;
    let again = payload(&again, "people_resolve idempotent");
    assert_eq!(
        again.get("person_id").and_then(Value::as_str),
        Some(person_id.as_str()),
        "the same handle must always resolve to the same person: {again}"
    );
    assert_eq!(
        again.get("created").and_then(Value::as_bool),
        Some(false),
        "a second resolve must not report a mint: {again}"
    );

    // ── score: the composite and its four components are in range ──
    let scored = rpc(
        &harness.rpc_base,
        42_004,
        "openhuman.people_score",
        json!({ "person_id": person_id }),
    )
    .await;
    let scored = payload(&scored, "people_score");
    assert_eq!(
        scored.get("person_id").and_then(Value::as_str),
        Some(person_id.as_str()),
        "score echoes the id it scored: {scored}"
    );
    let composite = scored
        .get("score")
        .and_then(Value::as_f64)
        .unwrap_or_else(|| panic!("score must report a composite: {scored}"));
    assert!(
        (0.0..=1.0).contains(&composite),
        "the composite score is documented as [0,1], got {composite}"
    );
    let components = scored
        .get("components")
        .unwrap_or_else(|| panic!("score must break down by component: {scored}"));
    for component in ["recency", "frequency", "reciprocity", "depth"] {
        let value = components
            .get(component)
            .and_then(Value::as_f64)
            .unwrap_or_else(|| panic!("missing component {component}: {scored}"));
        assert!(
            (0.0..=1.0).contains(&value),
            "component {component} is documented as [0,1], got {value}"
        );
    }
    assert_eq!(
        scored.get("interaction_count").and_then(Value::as_u64),
        Some(0),
        "a person minted from a bare handle has no observed interactions: {scored}"
    );

    // ── list: the minted person appears, carrying the handle it was minted from ──
    let listed = rpc(
        &harness.rpc_base,
        42_005,
        "openhuman.people_list",
        json!({ "limit": 200 }),
    )
    .await;
    let listed = payload(&listed, "people_list");
    let people = listed
        .get("people")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("people_list returns a people array: {listed}"));
    let row = people
        .iter()
        .find(|p| p.get("person_id").and_then(Value::as_str) == Some(person_id.as_str()))
        .unwrap_or_else(|| panic!("the minted person must be listed: {listed}"));
    let handles = row
        .get("handles")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("a listed person carries its handles: {row}"));
    assert!(
        handles.iter().any(|h| {
            h.get("kind").and_then(Value::as_str) == Some("email")
                && h.get("value")
                    .and_then(Value::as_str)
                    .is_some_and(|v| v.eq_ignore_ascii_case(handle))
        }),
        "the handle the person was minted from must be on the row: {row}"
    );
    assert!(
        row.get("components").is_some(),
        "the ranked list carries the same component breakdown as `score`: {row}"
    );

    harness.join.abort();
}

fn uuid_shaped(value: &str) -> bool {
    value.len() == 36 && value.split('-').map(str::len).eq([8, 4, 4, 4, 12])
}

/// The people validation boundary: the handle kind is a closed set, a
/// malformed `person_id` is refused by name, and `list`'s `limit` is typed.
#[tokio::test]
async fn people_reject_unknown_handle_kinds_and_malformed_ids() {
    let _lock = env_lock();
    let harness = setup().await;

    let bad_kind = rpc(
        &harness.rpc_base,
        42_101,
        "openhuman.people_resolve",
        json!({ "kind": "carrier_pigeon", "value": "someone" }),
    )
    .await;
    let message = error_message(&bad_kind, "people_resolve bad kind");
    assert!(
        message.contains("carrier_pigeon")
            && message.contains("imessage")
            && message.contains("email")
            && message.contains("display_name"),
        "the refusal must name the offending kind and the accepted set, got: {message}"
    );

    let no_value = rpc(
        &harness.rpc_base,
        42_102,
        "openhuman.people_resolve",
        json!({ "kind": "email" }),
    )
    .await;
    assert!(
        error_message(&no_value, "people_resolve without value")
            .contains("missing required param 'value'"),
        "the refusal must name the param: {no_value}"
    );

    let wrong_type = rpc(
        &harness.rpc_base,
        42_103,
        "openhuman.people_resolve",
        json!({ "kind": "email", "value": 42 }),
    )
    .await;
    let wrong_type_message = error_message(&wrong_type, "people_resolve numeric value");
    assert!(
        wrong_type_message.contains("expected string") && wrong_type_message.contains("number"),
        "the refusal must name both the expected and the actual type, got: {wrong_type_message}"
    );

    // `person_id` is parsed as a UUID host-side so a typo fails with the param
    // name rather than as an opaque driver error.
    let bad_id = rpc(
        &harness.rpc_base,
        42_104,
        "openhuman.people_score",
        json!({ "person_id": "not-a-uuid" }),
    )
    .await;
    let bad_id_message = error_message(&bad_id, "people_score bad id");
    assert!(
        bad_id_message.contains("person_id") && bad_id_message.contains("not-a-uuid"),
        "the refusal must name the param and the value, got: {bad_id_message}"
    );

    // A well-formed UUID that names nobody: the id parses, so the refusal has
    // to come from the driver — and it must still identify what was not found.
    let no_such_person = rpc(
        &harness.rpc_base,
        42_105,
        "openhuman.people_score",
        json!({ "person_id": "00000000-0000-4000-8000-000000000000" }),
    )
    .await;
    let no_such_message = error_message(&no_such_person, "people_score unknown uuid");
    assert!(
        no_such_message.contains("00000000-0000-4000-8000-000000000000"),
        "the refusal must name the person that was not found, got: {no_such_message}"
    );

    let bad_limit = rpc(
        &harness.rpc_base,
        42_106,
        "openhuman.people_list",
        json!({ "limit": "lots" }),
    )
    .await;
    assert!(
        error_message(&bad_limit, "people_list string limit").contains("unsigned integer"),
        "a non-numeric limit must be refused: {bad_limit}"
    );

    harness.join.abort();
}

/// `people.refresh_address_book` seeds from the system address book. On a CI
/// host with no address book — or no Contacts permission — the contract is
/// `seeded: 0`, deliberately *not* a distinct error, and `permission_denied`
/// can no longer become true.
#[tokio::test]
async fn people_refresh_address_book_reports_a_seed_count_not_a_permission_error() {
    let _lock = env_lock();
    let harness = setup().await;

    let refreshed = rpc(
        &harness.rpc_base,
        42_201,
        "openhuman.people_refresh_address_book",
        json!({}),
    )
    .await;

    // A host that cannot read contacts at all may refuse at the driver; what it
    // must never do is report a permission problem through the retained field.
    if refreshed.get("error").is_some() {
        let message = error_message(&refreshed, "people_refresh_address_book");
        assert!(
            message.contains("address_book"),
            "a driver refusal must be attributed to the address book, got: {message}"
        );
    } else {
        let body = payload(&refreshed, "people_refresh_address_book");
        assert!(
            body.get("seeded").and_then(Value::as_u64).is_some(),
            "the outcome must carry a seed count: {body}"
        );
        assert!(
            body.get("skipped").and_then(Value::as_u64).is_some(),
            "the outcome must carry a skip count: {body}"
        );
        assert_eq!(
            body.get("permission_denied").and_then(Value::as_bool),
            Some(false),
            "the field is retained for wire compatibility and can no longer \
             become true — a host without permission reports seeded: 0: {body}"
        );
    }

    harness.join.abort();
}

// ── tree_summarizer ───────────────────────────────────────────────────────

/// `tree_summarizer.status` and `.query` over a namespace with no tree.
///
/// `status` answers a well-formed empty status; `query` refuses, because a node
/// that is not there is the caller asking for something specific that does not
/// exist — the asymmetry is the contract, and it is what these assertions pin.
#[tokio::test]
async fn tree_summarizer_status_is_empty_and_query_refuses_an_absent_node() {
    let _lock = env_lock();
    let harness = setup().await;

    let namespace = "e2e_tree_summarizer_empty";

    let status = rpc(
        &harness.rpc_base,
        43_001,
        "openhuman.tree_summarizer_status",
        json!({ "namespace": namespace }),
    )
    .await;
    let status = payload(&status, "tree_summarizer_status");
    assert_eq!(
        status.get("namespace").and_then(Value::as_str),
        Some(namespace),
        "status echoes the namespace it describes: {status}"
    );
    assert_eq!(
        status.get("total_nodes").and_then(Value::as_u64),
        Some(0),
        "a namespace with no tree has no nodes: {status}"
    );
    assert_eq!(
        status.get("depth").and_then(Value::as_u64),
        Some(0),
        "and therefore no depth: {status}"
    );
    assert_eq!(
        status.get("newest_entry"),
        Some(&Value::Null),
        "with no entries the timestamps are null, not epoch: {status}"
    );

    // `query` defaults to the root node, which does not exist yet.
    let query = rpc(
        &harness.rpc_base,
        43_002,
        "openhuman.tree_summarizer_query",
        json!({ "namespace": namespace }),
    )
    .await;
    let message = error_message(&query, "tree_summarizer_query root");
    assert!(
        message.contains("node 'root' not found") && message.contains(namespace),
        "the refusal must name both the node and the namespace, got: {message}"
    );

    // An explicit node id is reported by that id, not silently rewritten to root.
    let dated = rpc(
        &harness.rpc_base,
        43_003,
        "openhuman.tree_summarizer_query",
        json!({ "namespace": namespace, "node_id": "2026/09/07/12" }),
    )
    .await;
    let dated_message = error_message(&dated, "tree_summarizer_query node");
    assert!(
        dated_message.contains("2026/09/07/12"),
        "the refusal must name the node that was asked for, got: {dated_message}"
    );

    harness.join.abort();
}

/// The consent gate on the two passes that spend on a model.
///
/// `run` and `rebuild` resolve a summarization provider **before** any driver
/// work begins, purely so an opted-out user's memory summaries cannot be sent
/// to a cloud provider by a route that knows nothing about the opt-in. With
/// local AI off and `memory_tree.cloud_summarization_opt_in = false` — the
/// shipped default — both must refuse with an error that names the setting.
#[tokio::test]
async fn tree_summarizer_run_and_rebuild_refuse_without_summarization_consent() {
    let _lock = env_lock();
    let harness = setup().await;

    let namespace = "e2e_tree_summarizer_consent";

    for (id, method, context) in [
        (43_101_i64, "openhuman.tree_summarizer_run", "run"),
        (43_102, "openhuman.tree_summarizer_rebuild", "rebuild"),
    ] {
        let response = rpc(&harness.rpc_base, id, method, json!({ "namespace": namespace })).await;
        let message = error_message(&response, context);
        assert!(
            message.contains("no summarization provider"),
            "{context} must refuse before doing any work, got: {message}"
        );
        assert!(
            message.contains("local AI")
                && message.contains("memory_tree.cloud_summarization_opt_in"),
            "the refusal must name both ways out, so it is actionable — got: {message}"
        );
    }

    // Missing `namespace` is refused by name on every controller in the family.
    for (id, method, context) in [
        (43_103_i64, "openhuman.tree_summarizer_status", "status"),
        (43_104, "openhuman.tree_summarizer_query", "query"),
        (43_105, "openhuman.tree_summarizer_run", "run"),
        (43_106, "openhuman.tree_summarizer_rebuild", "rebuild"),
    ] {
        let response = rpc(&harness.rpc_base, id, method, json!({})).await;
        assert!(
            error_message(&response, context).contains("missing required param 'namespace'"),
            "{context} must name the missing param: {response}"
        );
    }

    // A wrong-typed namespace is a type error, not a stringified number.
    let wrong_type = rpc(
        &harness.rpc_base,
        43_107,
        "openhuman.tree_summarizer_status",
        json!({ "namespace": 7 }),
    )
    .await;
    assert!(
        error_message(&wrong_type, "tree_summarizer_status numeric namespace")
            .contains("invalid type for param 'namespace' in tree_summarizer.status"),
        "the refusal must name the param, the controller, and both types: {wrong_type}"
    );

    harness.join.abort();
}
