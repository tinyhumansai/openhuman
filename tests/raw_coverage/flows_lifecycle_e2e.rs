//! JSON-RPC E2E coverage for the uncovered half of the `flows` namespace:
//! core-managed drafts, duplicate/enable, revision history + rollback, the
//! run-history read/prune surface, the save-time approval manifest and
//! connector requirements, the canvas tool-browser surface, and the copilot's
//! Stop button.
//!
//! Every case boots the real Axum JSON-RPC router over HTTP against an
//! isolated `HOME`, dispatches through `/rpc`, and asserts on the **content**
//! of the response. Paths that would reach Composio are asserted at the
//! credential boundary so the suite stays hermetic and offline.
//!
//! Aggregated into `tests/raw_coverage_all.rs` by `build.rs`. Run with:
//! `cargo test --test raw_coverage_all --features "$(bash scripts/ci/product-features.sh)" flows_lifecycle_e2e`

#![cfg(feature = "flows")]

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
const TEST_RPC_TOKEN: &str = "flows-lifecycle-e2e-token";

/// `tinyflows_sqlite::flows::MAX_FLOW_RUNS_PER_FLOW` — the retention cap
/// `flows_prune_runs` reports as `kept`. Hard-coded rather than imported so a
/// silent change to the cap fails this test instead of tracking it.
const EXPECTED_RUN_RETENTION_CAP: u64 = 100;

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

/// Serializes every case in this binary: `HOME` and the backend-URL overrides
/// are process-global, so two cases running in parallel would resolve each
/// other's `config.toml` and each other's flows database.
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
        let token_dir = std::env::temp_dir().join("openhuman-flows-lifecycle-e2e-auth");
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

/// `api_url` points at a closed port on purpose: no case here may reach the
/// network, and a refused connection is the fastest deterministic failure.
fn write_min_config(openhuman_dir: &Path) {
    let cfg = r#"api_url = "http://127.0.0.1:9"
default_model = "flows-e2e-model"
default_temperature = 0.2
chat_onboarding_completed = true

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
            .timeout(Duration::from_secs(60))
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

    /// Dispatch and unwrap the controller payload, panicking on a JSON-RPC error.
    async fn ok(&self, id: i64, method: &str, params: Value) -> Value {
        let response = self.rpc(id, method, params).await;
        if let Some(error) = response.get("error") {
            panic!("{method}: unexpected JSON-RPC error: {error}");
        }
        let result = response
            .get("result")
            .unwrap_or_else(|| panic!("{method}: missing result: {response}"));
        // Controllers return `{ result, logs }`; peel that envelope when present.
        match result.get("result") {
            Some(inner) if result.get("logs").is_some() => inner.clone(),
            _ => result.clone(),
        }
    }

    /// Dispatch and return the JSON-RPC error message, panicking on success.
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
    let home = tmp.path();
    write_min_config(&home.join(".openhuman"));

    // `flows_approval_manifest` builds a `SecurityPolicy` from `action_dir`,
    // which defaults to `~/OpenHuman/projects` — the developer's real
    // directory. Pin it inside the tempdir so no case reads host state.
    let action_dir = home.join("actions");
    std::fs::create_dir_all(&action_dir).expect("create action dir");

    let guards = vec![
        EnvVarGuard::set_to_path("HOME", home),
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
        _tmp: tmp,
        _guards: guards,
        rpc_base: format!("http://{addr}"),
        token,
        join,
    }
}

/// The smallest structurally valid graph: one manual trigger.
fn trigger_only_graph() -> Value {
    json!({
        "nodes": [{ "id": "t", "kind": "trigger", "name": "Manual" }],
        "edges": []
    })
}

/// A trigger plus a pass-through parser — runs to completion with no model.
fn two_node_graph() -> Value {
    json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "p", "kind": "output_parser", "name": "Passthrough" }
        ],
        "edges": [{ "from_node": "t", "to_node": "p" }]
    })
}

fn str_at<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("expected a string at {pointer} in {value}"))
}

// ── drafts ──────────────────────────────────────────────────────────────────

/// The whole core-managed draft surface: create → list → get → update →
/// promote, plus the three ways it refuses. `draft_promote` is the interesting
/// one: it must run the *same* create gates as a direct save and then delete
/// the draft, so the draft store is empty afterwards and the flow store is not.
#[tokio::test]
async fn flows_draft_surface_round_trips_and_promotes_into_a_saved_flow() {
    let _lock = env_lock();
    let h = setup().await;

    let draft = h
        .ok(
            1001,
            "openhuman.flows_draft_create",
            json!({ "name": "Draft One", "graph": trigger_only_graph(), "origin": "chat" }),
        )
        .await;
    let draft_id = str_at(&draft, "/id").to_string();
    assert!(!draft_id.is_empty(), "draft_create must mint an id");
    assert_eq!(draft.get("name").and_then(Value::as_str), Some("Draft One"));
    assert_eq!(draft.get("origin").and_then(Value::as_str), Some("chat"));
    assert!(
        draft.get("flow_id").is_none_or(Value::is_null),
        "an unlinked draft carries no flow_id: {draft}"
    );
    assert_eq!(
        draft.pointer("/graph/nodes").and_then(Value::as_array).map(Vec::len),
        Some(1),
        "draft_create stores the graph verbatim: {draft}"
    );

    let listed = h.ok(1002, "openhuman.flows_draft_list", json!({})).await;
    let drafts = listed.as_array().expect("draft_list returns an array");
    assert_eq!(drafts.len(), 1, "exactly the draft just created: {listed}");
    assert_eq!(str_at(&drafts[0], "/id"), draft_id);

    let fetched = h
        .ok(1003, "openhuman.flows_draft_get", json!({ "id": draft_id }))
        .await;
    assert_eq!(str_at(&fetched, "/name"), "Draft One");

    let missing = h
        .err(
            1004,
            "openhuman.flows_draft_get",
            json!({ "id": "no-such-draft" }),
        )
        .await;
    assert!(
        missing.contains("no-such-draft") && missing.contains("not found"),
        "draft_get names the absent id: {missing}"
    );

    let updated = h
        .ok(
            1005,
            "openhuman.flows_draft_update",
            json!({ "id": draft_id, "name": "Draft Renamed", "graph": two_node_graph() }),
        )
        .await;
    assert_eq!(str_at(&updated, "/name"), "Draft Renamed");
    assert_eq!(
        updated.pointer("/graph/nodes").and_then(Value::as_array).map(Vec::len),
        Some(2),
        "draft_update replaces the graph: {updated}"
    );

    // A non-string `flow_id` must be REJECTED, not coerced into an unlink —
    // silently unlinking would make the later promote create a second flow.
    let bad_link = h
        .err(
            1006,
            "openhuman.flows_draft_update",
            json!({ "id": draft_id, "flow_id": 42 }),
        )
        .await;
    assert!(
        bad_link.to_lowercase().contains("flow_id"),
        "a numeric flow_id is rejected by name: {bad_link}"
    );

    let promoted = h
        .ok(
            1007,
            "openhuman.flows_draft_promote",
            json!({ "id": draft_id }),
        )
        .await;
    let flow_id = str_at(&promoted, "/id").to_string();
    assert_eq!(
        str_at(&promoted, "/name"),
        "Draft Renamed",
        "promote carries the draft's name onto the flow"
    );
    assert_ne!(flow_id, draft_id, "the flow gets its own id");

    let flows = h.ok(1008, "openhuman.flows_list", json!({})).await;
    let flows = flows.as_array().expect("flows_list returns an array");
    assert_eq!(flows.len(), 1, "promote saved exactly one flow: {flows:?}");
    assert_eq!(str_at(&flows[0], "/id"), flow_id);

    let after = h.ok(1009, "openhuman.flows_draft_list", json!({})).await;
    assert_eq!(
        after.as_array().map(Vec::len),
        Some(0),
        "promote deletes the draft it consumed: {after}"
    );

    // Delete is idempotent and says so rather than erroring.
    let deleted = h
        .ok(
            1010,
            "openhuman.flows_draft_delete",
            json!({ "id": draft_id }),
        )
        .await;
    assert_eq!(deleted.get("id").and_then(Value::as_str), Some(draft_id.as_str()));
    assert_eq!(
        deleted.get("deleted").and_then(Value::as_bool),
        Some(false),
        "deleting an already-absent draft reports deleted=false: {deleted}"
    );

    let promote_missing = h
        .err(
            1011,
            "openhuman.flows_draft_promote",
            json!({ "id": "no-such-draft" }),
        )
        .await;
    assert!(
        promote_missing.contains("no-such-draft"),
        "promote names the absent draft: {promote_missing}"
    );

    h.join.abort();
}

/// A draft that names a `flow_id` must UPDATE that flow on promote rather than
/// create a second one — the behaviour `parse_draft_update_flow_id` exists to
/// protect.
#[tokio::test]
async fn flows_draft_promote_updates_the_linked_flow_instead_of_creating_a_second() {
    let _lock = env_lock();
    let h = setup().await;

    let flow = h
        .ok(
            1101,
            "openhuman.flows_create",
            json!({ "name": "Original", "graph": trigger_only_graph() }),
        )
        .await;
    let flow_id = str_at(&flow, "/id").to_string();

    let draft = h
        .ok(
            1102,
            "openhuman.flows_draft_create",
            json!({
                "name": "Edited In Canvas",
                "graph": two_node_graph(),
                "flow_id": flow_id,
            }),
        )
        .await;
    assert_eq!(
        draft.get("flow_id").and_then(Value::as_str),
        Some(flow_id.as_str()),
        "the draft records the flow it edits: {draft}"
    );

    let promoted = h
        .ok(
            1103,
            "openhuman.flows_draft_promote",
            json!({ "id": str_at(&draft, "/id") }),
        )
        .await;
    assert_eq!(
        str_at(&promoted, "/id"),
        flow_id,
        "promoting a linked draft updates the SAME flow: {promoted}"
    );
    assert_eq!(str_at(&promoted, "/name"), "Edited In Canvas");

    let flows = h.ok(1104, "openhuman.flows_list", json!({})).await;
    assert_eq!(
        flows.as_array().map(Vec::len),
        Some(1),
        "no second flow was created: {flows}"
    );

    h.join.abort();
}

// ── duplicate / set_enabled ─────────────────────────────────────────────────

/// A duplicate is a copy that is deliberately born DISABLED and unbound, so it
/// can never fire on its own schedule before the user has reviewed it.
#[tokio::test]
async fn flows_duplicate_copies_the_graph_and_lands_disabled() {
    let _lock = env_lock();
    let h = setup().await;

    let source = h
        .ok(
            1201,
            "openhuman.flows_create",
            json!({ "name": "Nightly Report", "graph": two_node_graph() }),
        )
        .await;
    let source_id = str_at(&source, "/id").to_string();
    assert_eq!(
        source.get("enabled").and_then(Value::as_bool),
        Some(true),
        "a manually triggered flow is created enabled"
    );

    let copy = h
        .ok(
            1202,
            "openhuman.flows_duplicate",
            json!({ "id": source_id }),
        )
        .await;
    assert_ne!(str_at(&copy, "/id"), source_id, "the copy gets a fresh id");
    assert_eq!(
        str_at(&copy, "/name"),
        "Nightly Report (copy)",
        "the copy is suffixed: {copy}"
    );
    assert_eq!(
        copy.get("enabled").and_then(Value::as_bool),
        Some(false),
        "a duplicate is born disabled: {copy}"
    );
    assert_eq!(
        copy.pointer("/graph/nodes").and_then(Value::as_array).map(Vec::len),
        Some(2),
        "the copy carries the source graph: {copy}"
    );

    let flows = h.ok(1203, "openhuman.flows_list", json!({})).await;
    assert_eq!(flows.as_array().map(Vec::len), Some(2));

    let missing = h
        .err(
            1204,
            "openhuman.flows_duplicate",
            json!({ "id": "no-such-flow" }),
        )
        .await;
    assert!(
        missing.contains("no-such-flow") && missing.contains("not found"),
        "duplicate names the absent flow: {missing}"
    );

    h.join.abort();
}

/// `set_enabled` is the toggle behind the flow list's switch. Both directions
/// must be reflected in the stored flow, and a re-read must agree.
#[tokio::test]
async fn flows_set_enabled_toggles_both_ways_and_persists() {
    let _lock = env_lock();
    let h = setup().await;

    let flow = h
        .ok(
            1301,
            "openhuman.flows_create",
            json!({ "name": "Toggle Me", "graph": trigger_only_graph() }),
        )
        .await;
    let flow_id = str_at(&flow, "/id").to_string();

    let off = h
        .ok(
            1302,
            "openhuman.flows_set_enabled",
            json!({ "id": flow_id, "enabled": false }),
        )
        .await;
    assert_eq!(off.get("enabled").and_then(Value::as_bool), Some(false));

    let reread = h
        .ok(1303, "openhuman.flows_get", json!({ "id": flow_id }))
        .await;
    assert_eq!(
        reread.get("enabled").and_then(Value::as_bool),
        Some(false),
        "the disable is persisted, not just echoed: {reread}"
    );

    let on = h
        .ok(
            1304,
            "openhuman.flows_set_enabled",
            json!({ "id": flow_id, "enabled": true }),
        )
        .await;
    assert_eq!(on.get("enabled").and_then(Value::as_bool), Some(true));

    let missing = h
        .err(
            1305,
            "openhuman.flows_set_enabled",
            json!({ "id": "no-such-flow", "enabled": true }),
        )
        .await;
    assert!(
        missing.to_lowercase().contains("not found") || missing.contains("no-such-flow"),
        "set_enabled on an absent flow is an error: {missing}"
    );

    let no_flag = h
        .err(
            1306,
            "openhuman.flows_set_enabled",
            json!({ "id": flow_id }),
        )
        .await;
    assert!(
        no_flag.contains("enabled"),
        "the required `enabled` param is named: {no_flag}"
    );

    h.join.abort();
}

// ── history / rollback ──────────────────────────────────────────────────────

/// Every update snapshots the *prior* graph, and rollback restores one through
/// the normal update path — so the rollback is itself snapshotted and undoable.
#[tokio::test]
async fn flows_history_records_prior_graphs_and_rollback_restores_them() {
    let _lock = env_lock();
    let h = setup().await;

    let flow = h
        .ok(
            1401,
            "openhuman.flows_create",
            json!({ "name": "Versioned", "graph": trigger_only_graph() }),
        )
        .await;
    let flow_id = str_at(&flow, "/id").to_string();

    let empty = h
        .ok(
            1402,
            "openhuman.flows_get_history",
            json!({ "id": flow_id }),
        )
        .await;
    assert_eq!(
        empty.as_array().map(Vec::len),
        Some(0),
        "a freshly created flow has no prior revisions: {empty}"
    );

    h.ok(
        1403,
        "openhuman.flows_update",
        json!({ "id": flow_id, "graph": two_node_graph() }),
    )
    .await;

    let history = h
        .ok(
            1404,
            "openhuman.flows_get_history",
            json!({ "id": flow_id }),
        )
        .await;
    let revisions = history.as_array().expect("get_history returns an array");
    assert_eq!(revisions.len(), 1, "one prior snapshot: {history}");
    let revision = &revisions[0];
    let revision_id = str_at(revision, "/id").to_string();
    assert_eq!(
        revision.get("flow_id").and_then(Value::as_str),
        Some(flow_id.as_str())
    );
    assert_eq!(
        revision.pointer("/graph/nodes").and_then(Value::as_array).map(Vec::len),
        Some(1),
        "the snapshot holds the ORIGINAL one-node graph, not the new one: {revision}"
    );

    let rolled = h
        .ok(
            1405,
            "openhuman.flows_rollback",
            json!({ "id": flow_id, "revision_id": revision_id }),
        )
        .await;
    assert_eq!(
        rolled.pointer("/graph/nodes").and_then(Value::as_array).map(Vec::len),
        Some(1),
        "rollback restored the one-node graph: {rolled}"
    );

    let after = h
        .ok(
            1406,
            "openhuman.flows_get_history",
            json!({ "id": flow_id }),
        )
        .await;
    assert_eq!(
        after.as_array().map(Vec::len),
        Some(2),
        "the rollback snapshotted the graph it replaced, so it is undoable: {after}"
    );

    let bad_revision = h
        .err(
            1407,
            "openhuman.flows_rollback",
            json!({ "id": flow_id, "revision_id": "no-such-revision" }),
        )
        .await;
    assert!(
        bad_revision.contains("no-such-revision") && bad_revision.contains(&flow_id),
        "rollback names both the revision and the flow: {bad_revision}"
    );

    // `limit` is honoured, not ignored.
    let capped = h
        .ok(
            1408,
            "openhuman.flows_get_history",
            json!({ "id": flow_id, "limit": 1 }),
        )
        .await;
    assert_eq!(
        capped.as_array().map(Vec::len),
        Some(1),
        "get_history honours limit: {capped}"
    );

    h.join.abort();
}

// ── run history: list_all_runs / prune_runs ─────────────────────────────────

/// `flows_list_runs` is per-flow; `flows_list_all_runs` is the cross-flow feed
/// behind the global run list. It must see runs from *both* flows, newest
/// first, and honour its limit.
#[tokio::test]
async fn flows_list_all_runs_spans_flows_and_prune_reports_the_retention_cap() {
    let _lock = env_lock();
    let h = setup().await;

    let first = h
        .ok(
            1501,
            "openhuman.flows_create",
            json!({ "name": "Flow A", "graph": two_node_graph() }),
        )
        .await;
    let first_id = str_at(&first, "/id").to_string();
    let second = h
        .ok(
            1502,
            "openhuman.flows_create",
            json!({ "name": "Flow B", "graph": two_node_graph() }),
        )
        .await;
    let second_id = str_at(&second, "/id").to_string();

    h.ok(1503, "openhuman.flows_run", json!({ "id": first_id })).await;
    h.ok(1504, "openhuman.flows_run", json!({ "id": second_id })).await;

    let all = h.ok(1505, "openhuman.flows_list_all_runs", json!({})).await;
    let runs = all.as_array().expect("list_all_runs returns an array");
    assert_eq!(runs.len(), 2, "one run recorded per flow: {all}");
    let flow_ids: Vec<&str> = runs.iter().map(|run| str_at(run, "/flow_id")).collect();
    assert!(
        flow_ids.contains(&first_id.as_str()) && flow_ids.contains(&second_id.as_str()),
        "list_all_runs spans both flows, got {flow_ids:?}"
    );
    assert_eq!(
        str_at(&runs[0], "/flow_id"),
        second_id,
        "newest first — Flow B ran last: {all}"
    );

    let one = h
        .ok(1506, "openhuman.flows_list_all_runs", json!({ "limit": 1 }))
        .await;
    assert_eq!(
        one.as_array().map(Vec::len),
        Some(1),
        "list_all_runs honours limit: {one}"
    );

    // Two runs are far inside the retention window, so an explicit sweep must
    // remove nothing while still reporting the cap it swept against.
    let pruned = h
        .ok(
            1507,
            "openhuman.flows_prune_runs",
            json!({ "id": first_id }),
        )
        .await;
    assert_eq!(
        pruned.get("flow_id").and_then(Value::as_str),
        Some(first_id.as_str())
    );
    assert_eq!(
        pruned.get("pruned").and_then(Value::as_u64),
        Some(0),
        "nothing to prune inside the window: {pruned}"
    );
    assert_eq!(
        pruned.get("kept").and_then(Value::as_u64),
        Some(EXPECTED_RUN_RETENTION_CAP),
        "prune reports the retention cap it applied: {pruned}"
    );

    let still_there = h.ok(1508, "openhuman.flows_list_all_runs", json!({})).await;
    assert_eq!(
        still_there.as_array().map(Vec::len),
        Some(2),
        "a no-op prune must not delete live history: {still_there}"
    );

    let missing_id = h
        .err(1509, "openhuman.flows_prune_runs", json!({}))
        .await;
    assert!(
        missing_id.contains("id"),
        "prune_runs names its required param: {missing_id}"
    );

    h.join.abort();
}

// ── approval manifest / required connections ────────────────────────────────

/// The save+enable pre-authorization card: one row per permission a run will
/// prompt for, classified, deduped on the trust key, and joined against the
/// grants the flow already holds.
#[tokio::test]
async fn flows_approval_manifest_classifies_every_gated_node_kind() {
    let _lock = env_lock();
    let h = setup().await;

    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "http", "kind": "http_request", "name": "Call",
              "config": { "url": "https://example.invalid/hook", "method": "POST" } },
            { "id": "code", "kind": "code", "name": "Transform",
              "config": { "language": "javascript", "code": "return items;" } },
            { "id": "dyn", "kind": "tool_call", "name": "Chosen at run time",
              "config": { "slug": "=nodes.t.output.slug", "args": {} } },
            { "id": "ai", "kind": "agent", "name": "Summarize",
              "config": { "agent_ref": "summarizer", "prompt": "summarize" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "http" },
            { "from_node": "http", "to_node": "code" },
            { "from_node": "code", "to_node": "dyn" },
            { "from_node": "dyn", "to_node": "ai" }
        ]
    });

    let manifest = h
        .ok(
            1601,
            "openhuman.flows_approval_manifest",
            json!({ "graph": graph }),
        )
        .await;
    let entries = manifest
        .get("entries")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("manifest carries an entries array: {manifest}"));

    let find = |tool: &str| -> &Value {
        entries
            .iter()
            .find(|entry| entry.get("tool_name").and_then(Value::as_str) == Some(tool))
            .unwrap_or_else(|| panic!("manifest must list {tool}: {manifest}"))
    };

    let http = find("flows_http_request");
    assert_eq!(http.get("node_id").and_then(Value::as_str), Some("http"));
    assert_eq!(http.get("class").and_then(Value::as_str), Some("Network"));
    assert_eq!(
        str_at(http, "/label"),
        "Call https://example.invalid/hook",
        "the label names the destination so the card is readable: {http}"
    );

    let code = find("flows_code");
    assert_eq!(code.get("class").and_then(Value::as_str), Some("Write"));

    assert!(
        entries
            .iter()
            .any(|entry| entry.get("kind").and_then(Value::as_str) == Some("dynamic")
                && entry.get("node_id").and_then(Value::as_str) == Some("dyn")),
        "an `=` slug cannot be pre-approved and must be disclosed as dynamic: {manifest}"
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.get("kind").and_then(Value::as_str) == Some("agent")
                && entry.get("node_id").and_then(Value::as_str) == Some("ai")),
        "an agent node's inner tool calls are unknowable and must be disclosed: {manifest}"
    );

    // `missing` and `already_trusted` partition the APPROVABLE keys — the
    // dynamic and agent rows have no trust key to grant, so neither list may
    // name them.
    let gate_installed = manifest
        .get("gate_installed")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("gate_installed is reported as a boolean: {manifest}"));
    let missing: Vec<&str> = manifest
        .get("missing")
        .and_then(Value::as_array)
        .expect("missing array")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let already: Vec<&str> = manifest
        .get("already_trusted")
        .and_then(Value::as_array)
        .expect("already_trusted array")
        .iter()
        .filter_map(Value::as_str)
        .collect();

    if gate_installed {
        // No `id` was given, so no grants can be joined: every approvable key
        // is missing and nothing is already trusted.
        assert!(
            missing.contains(&"flows_http_request") && missing.contains(&"flows_code"),
            "un-granted approvable keys are listed as missing: {missing:?}"
        );
        assert!(
            already.is_empty(),
            "a candidate graph joined against no flow holds no grants: {already:?}"
        );
    } else {
        // Gate uninstalled: nothing ever parks, so nothing is `missing`; and no
        // grant was ever made, so nothing is `already_trusted` either.
        // `gate_installed: false` is the caller's only signal.
        //
        // This branch used to assert the opposite — the approvable keys landed
        // in `already_trusted`, claiming grants the flow did not hold. openhuman#6093
        // fixed that (`split_manifest_trust` returns two empty lists when the
        // gate is absent), so the assertion is inverted to the fixed behaviour.
        assert!(
            missing.is_empty(),
            "with no gate installed nothing can park, so nothing is missing: {missing:?}"
        );
        assert!(
            already.is_empty(),
            "with no gate installed no grant was ever made, so nothing is already \
             trusted either: {already:?}"
        );
    }
    for list in [&missing, &already] {
        assert!(
            !list.contains(&"dyn") && !list.contains(&"ai"),
            "only approvable rows carry a trust key: {list:?}"
        );
    }

    let neither = h
        .err(1602, "openhuman.flows_approval_manifest", json!({}))
        .await;
    assert!(
        neither.contains("'id'") && neither.contains("'graph'"),
        "the manifest says which of the two inputs it needs: {neither}"
    );

    let unknown = h
        .err(
            1603,
            "openhuman.flows_approval_manifest",
            json!({ "id": "no-such-flow" }),
        )
        .await;
    assert!(
        unknown.contains("no-such-flow"),
        "the manifest names the absent flow: {unknown}"
    );

    h.join.abort();
}

/// The "Connect <toolkit>" CTAs. Composio slugs contribute a toolkit; native
/// `oh:` tools and plain HTTP nodes deliberately do not.
#[tokio::test]
async fn flows_required_connections_lists_only_composio_toolkits() {
    let _lock = env_lock();
    let h = setup().await;

    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "send", "kind": "tool_call", "name": "Send",
              "config": { "slug": "GMAIL_SEND_EMAIL", "args": {} } },
            { "id": "native", "kind": "tool_call", "name": "Native",
              "config": { "slug": "oh:web_search", "args": {} } },
            { "id": "http", "kind": "http_request", "name": "Webhook",
              "config": { "url": "https://example.invalid/hook", "method": "POST" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "send" },
            { "from_node": "send", "to_node": "native" },
            { "from_node": "native", "to_node": "http" }
        ]
    });

    let out = h
        .ok(
            1701,
            "openhuman.flows_required_connections",
            json!({ "graph": graph }),
        )
        .await;
    let required = out
        .get("required_connections")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("required_connections array: {out}"));
    assert_eq!(
        required.len(),
        1,
        "only the Composio slug needs a connection — `oh:` and http_request do not: {out}"
    );
    assert_eq!(required[0].get("toolkit").and_then(Value::as_str), Some("gmail"));
    assert_eq!(
        required[0].get("status").and_then(Value::as_str),
        Some("missing"),
        "a fresh workspace has no Gmail connection: {out}"
    );

    let none = h
        .ok(
            1702,
            "openhuman.flows_required_connections",
            json!({ "graph": trigger_only_graph() }),
        )
        .await;
    assert_eq!(
        none.get("required_connections").and_then(Value::as_array).map(Vec::len),
        Some(0),
        "a trigger-only graph needs no connection: {none}"
    );

    let no_graph = h
        .err(1703, "openhuman.flows_required_connections", json!({}))
        .await;
    assert!(
        no_graph.contains("graph"),
        "the required `graph` param is named: {no_graph}"
    );

    h.join.abort();
}

// ── canvas tool browser ─────────────────────────────────────────────────────

/// The in-canvas tool browser reads the LIVE Composio catalog. With no
/// credentials configured there is nothing to read, and the two endpoints
/// degrade differently on purpose: search returns an empty list (a keyword miss
/// is not an error), while a contract fetch for a named action is.
#[tokio::test]
async fn flows_tool_catalog_surface_degrades_without_composio_credentials() {
    let _lock = env_lock();
    let h = setup().await;

    let search = h
        .ok(
            1801,
            "openhuman.flows_search_tool_catalog",
            json!({ "query": "send email", "toolkit": "gmail", "limit": 5 }),
        )
        .await;
    assert_eq!(
        search.get("tools").and_then(Value::as_array).map(Vec::len),
        Some(0),
        "an unreachable catalog contributes zero rows rather than erroring: {search}"
    );

    // The `<TOOLKIT>_<ACTION>` shape diagnostic, checked before any I/O.
    // A slug that trims to empty has no toolkit segment, so it is refused here
    // rather than at the catalog.
    let malformed = h
        .err(
            1802,
            "openhuman.flows_get_tool_contract",
            json!({ "slug": "   " }),
        )
        .await;
    assert!(
        malformed.contains("GMAIL_SEND_EMAIL"),
        "the shape diagnostic names the expected form: {malformed}"
    );

    // A single-token slug has no action segment, so it cannot name an action a
    // contract fetch could return — it is refused by the same shape guard.
    //
    // This used to assert the opposite: `toolkit_from_slug` falls back to the
    // whole string, so `nodashhere` was accepted as its own toolkit and failed
    // later at the catalog with an unrelated message. openhuman#6093 added
    // `toolkit_for_contract_slug`, which requires non-empty segments either
    // side of the first `_` before delegating, so the assertion is inverted to
    // the fixed behaviour.
    let single_token = h
        .err(
            1806,
            "openhuman.flows_get_tool_contract",
            json!({ "slug": "nodashhere" }),
        )
        .await;
    assert!(
        single_token.contains("nodashhere") && single_token.contains("GMAIL_SEND_EMAIL"),
        "a dashless slug is caught by the shape guard, quoting the caller's own slug \
         and the expected form: {single_token}"
    );
    assert!(
        !single_token.contains("catalog"),
        "and it is refused before any catalog round trip: {single_token}"
    );

    let unreachable = h
        .err(
            1803,
            "openhuman.flows_get_tool_contract",
            json!({ "slug": "GMAIL_SEND_EMAIL" }),
        )
        .await;
    assert!(
        unreachable.contains("gmail") && unreachable.contains("catalog"),
        "a well-formed slug fails at the catalog, naming the toolkit: {unreachable}"
    );

    let no_slug = h
        .err(1804, "openhuman.flows_get_tool_contract", json!({}))
        .await;
    assert!(
        no_slug.contains("slug"),
        "the required `slug` param is named: {no_slug}"
    );

    let no_query = h
        .err(1805, "openhuman.flows_search_tool_catalog", json!({}))
        .await;
    assert!(
        no_query.contains("query"),
        "the required `query` param is named: {no_query}"
    );

    h.join.abort();
}

// ── copilot Stop button ─────────────────────────────────────────────────────

/// `flows_build_cancel` is the real cancellation behind the Workflow Copilot's
/// Stop button. `cancelled: false` is a normal answer, not an error — nothing
/// was in flight — and that distinction is the whole contract, because a stale
/// Stop must never kill a newer turn.
#[tokio::test]
async fn flows_build_cancel_reports_no_turn_in_flight_without_erroring() {
    let _lock = env_lock();
    let h = setup().await;

    let unscoped = h
        .ok(
            1901,
            "openhuman.flows_build_cancel",
            json!({ "thread_id": "thread-with-no-build" }),
        )
        .await;
    assert_eq!(
        unscoped.get("cancelled").and_then(Value::as_bool),
        Some(false),
        "nothing in flight is reported, not raised: {unscoped}"
    );

    let scoped = h
        .ok(
            1902,
            "openhuman.flows_build_cancel",
            json!({ "thread_id": "thread-with-no-build", "request_id": "req-1" }),
        )
        .await;
    assert_eq!(
        scoped.get("cancelled").and_then(Value::as_bool),
        Some(false),
        "a scoped cancel for an unregistered turn is also a no-op: {scoped}"
    );

    let no_thread = h
        .err(1903, "openhuman.flows_build_cancel", json!({}))
        .await;
    assert!(
        no_thread.contains("thread_id"),
        "the required `thread_id` param is named: {no_thread}"
    );

    h.join.abort();
}
