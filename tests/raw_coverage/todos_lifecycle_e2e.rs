//! JSON-RPC E2E coverage for the `openhuman.todos_*` board lifecycle.
//!
//! Run:
//! `cargo test --test raw_coverage_all --features "$(bash scripts/ci/product-features.sh)" todos_`
//!
//! Everything here is hermetic: the todo board is a workspace-local store, so a
//! temp `HOME` + `OPENHUMAN_WORKSPACE` is the whole fixture. No network.
//!
//! ## Why the `test` namespace is NOT covered here
//!
//! The wave brief flagged the single-method `test` namespace as possibly
//! deliberately untestable. It is, and more decisively than "nobody got round
//! to it": the whole `test_support` module — its one `test`-namespaced reset
//! method plus the five read-only `test_support_*` introspection methods — is
//! registered behind `#[cfg(feature = "e2e-test-support")]` at
//! `src/core/all.rs:875`. That gate is in neither `[features] default` nor
//! `scripts/ci/product-features.txt`; only `app/scripts/e2e-build.sh` turns it
//! on. Dispatching the reset under the product feature string this wave
//! mandates returns `unknown method`, which is the core's uniform "suppressed"
//! answer (see `docs/openhuman-core.md` — absence, not failure).
//!
//! So no `tests/**/*_e2e.rs` target can reach it without a bespoke feature
//! string, which would thrash the shared target dir for every worker. The
//! honest reading is 0%, and it is left at 0% deliberately. Note also that the
//! coverage gate's own denominator counts it: the namespace is discovered from
//! a `ControllerSchema` literal, and schema literals are not `#[cfg]`-gated
//! even though their registration is — so the gate asks for coverage of a
//! method that does not exist in the build it measures.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::http::header::AUTHORIZATION;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;

/// The bearer this suite *proposes*. It is only used if this suite happens to
/// be the first in the aggregated binary to initialise the token subsystem —
/// see `rpc_token()` below, which is what actually gets sent.
const PROPOSED_RPC_TOKEN: &str = "todos-lifecycle-e2e-token";

static AUTH_INIT: OnceLock<()> = OnceLock::new();

/// The crate-wide env lock, not a private one.
///
/// Every suite under `tests/raw_coverage/` compiles into the single
/// `raw_coverage_all` binary, so libtest runs them concurrently in one process
/// and a lock local to this file would isolate nothing — it would pass alone
/// and race another suite under load.
static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

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

/// Serializes every env mutation across the whole aggregated binary: `HOME` /
/// `OPENHUMAN_WORKSPACE` are process-global, so two cases running at once would
/// read one another's workspace. Poison is recovered so a panicking case cannot
/// wedge the other seventy-odd suites.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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
        let token_dir = std::env::temp_dir().join("openhuman-todos-lifecycle-e2e-auth");
        std::fs::create_dir_all(&token_dir).expect("token dir");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
    openhuman_core::core::auth::get_rpc_token()
        .expect("the token subsystem is initialised by the line above")
}

// ── Harness ─────────────────────────────────────────────────────────────────

const MIN_CONFIG: &str = r#"api_url = "http://127.0.0.1:9"
default_model = "todos-e2e-model"

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
    // Parsed here so a schema drift fails as a config error rather than as a
    // baffling handler error three calls later.
    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(MIN_CONFIG).expect("test config must match the config schema");

    let workspace = home.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");

    let mut guards = vec![
        EnvVarGuard::set_to_path("HOME", home),
        EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace),
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
        rpc_base: format!("http://{addr}"),
        join,
    }
}

async fn rpc(base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
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

/// The `/schema` controller catalog, as the frontend and the CLI/RPC
/// smoke-test generator consume it.
async fn schema_catalog(base: &str) -> Value {
    let url = format!("{}/schema", base.trim_end_matches('/'));
    reqwest::get(&url)
        .await
        .unwrap_or_else(|err| panic!("GET {url}: {err}"))
        .json::<Value>()
        .await
        .unwrap_or_else(|err| panic!("schema json: {err}"))
}

/// One method's declared entry from the catalog.
fn catalog_entry<'a>(catalog: &'a Value, method: &str) -> &'a Value {
    catalog
        .get("methods")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("catalog has no methods array: {catalog}"))
        .iter()
        .find(|m| m.get("method").and_then(Value::as_str) == Some(method))
        .unwrap_or_else(|| panic!("catalog does not advertise {method}"))
}

fn ok<'a>(value: &'a Value, context: &str) -> &'a Value {
    if let Some(error) = value.get("error") {
        panic!("{context}: unexpected JSON-RPC error: {error}");
    }
    value
        .get("result")
        .unwrap_or_else(|| panic!("{context}: missing result: {value}"))
}

fn error_message<'a>(value: &'a Value, context: &str) -> &'a str {
    value
        .get("error")
        .unwrap_or_else(|| panic!("{context}: expected a JSON-RPC error, got: {value}"))
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: error carries no message: {value}"))
}

/// The todos handlers return the snapshot unwrapped (see `snapshot_output` —
/// the catalog names the payload, the handler returns it flat), so cards live
/// at the top level of `result`.
fn cards<'a>(result: &'a Value, context: &str) -> &'a Vec<Value> {
    result
        .get("cards")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{context}: snapshot carries no `cards` array: {result}"))
}

/// Card titles, in board order.
///
/// The wire field is `title` — note that the `todos_add` / `todos_edit` *input*
/// params spell the same thing `content`, and `todos_replace`'s input spells it
/// `title` again. See `~/tinyhuman/bugs/e2e-wave-todos-replace-undocumented-card-shape.md`.
fn card_titles(result: &Value, context: &str) -> Vec<String> {
    cards(result, context)
        .iter()
        .map(|card| {
            card.get("title")
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("{context}: card without `title`: {card}"))
                .to_string()
        })
        .collect()
}

// ── Cases ───────────────────────────────────────────────────────────────────

/// `replace` → `list` → `set_session_thread` → `clear`: the write half of the
/// board that had no e2e coverage at all.
///
/// Asserts on content, not on `Ok`: the exact card titles after a wholesale
/// replace, the session link landing on the right card, and `clear` actually
/// emptying the list rather than returning a stale snapshot.
#[tokio::test]
async fn todos_replace_set_session_thread_and_clear_round_trip() {
    let _lock = env_lock();
    let harness = setup(Vec::new()).await;
    let thread = "todos-e2e-thread-a";

    // A wholesale replace on an empty board is the create path.
    let replaced = rpc(
        &harness.rpc_base,
        1,
        "openhuman.todos_replace",
        json!({
            "thread_id": thread,
            // `id`, `title` and `status` must all be PRESENT — none of the
            // three carries `#[serde(default)]`. The schema's "id may be empty
            // — server generates" means the string may be empty, not the key
            // may be absent, and the schema names no field at all.
            "cards": [
                { "id": "", "title": "draft the migration", "status": "todo" },
                { "id": "", "title": "review the migration", "status": "in_progress" }
            ]
        }),
    )
    .await;
    let replaced = ok(&replaced, "todos_replace");
    assert_eq!(
        card_titles(replaced, "todos_replace"),
        vec![
            "draft the migration".to_string(),
            "review the migration".to_string()
        ],
        "replace must persist exactly the cards it was given, in order"
    );
    assert!(
        replaced
            .get("markdown")
            .and_then(Value::as_str)
            .is_some_and(|md| md.contains("draft the migration")),
        "the snapshot's markdown rendering must include the card titles: {replaced}"
    );

    // The server mints ids for the blank ones; `list` must read back the same
    // board from disk rather than an in-memory echo of the request.
    let listed = rpc(
        &harness.rpc_base,
        2,
        "openhuman.todos_list",
        json!({ "thread_id": thread }),
    )
    .await;
    let listed = ok(&listed, "todos_list");
    assert_eq!(
        card_titles(listed, "todos_list"),
        card_titles(replaced, "todos_replace"),
        "list must return the board replace just wrote"
    );
    let first_id = cards(listed, "todos_list")[0]
        .get("id")
        .and_then(Value::as_str)
        .expect("server-minted card id")
        .to_string();
    assert!(!first_id.is_empty(), "server must mint a non-empty card id");

    // Link the card to an agent session's thread, then clear the link.
    let linked = rpc(
        &harness.rpc_base,
        3,
        "openhuman.todos_set_session_thread",
        json!({ "thread_id": thread, "id": first_id, "sessionThreadId": "agent-session-77" }),
    )
    .await;
    let linked = ok(&linked, "todos_set_session_thread");
    let linked_card = cards(linked, "todos_set_session_thread")
        .iter()
        .find(|card| card.get("id").and_then(Value::as_str) == Some(first_id.as_str()))
        .expect("the linked card survives the write");
    assert_eq!(
        linked_card
            .get("sessionThreadId")
            .or_else(|| linked_card.get("session_thread_id"))
            .and_then(Value::as_str),
        Some("agent-session-77"),
        "set_session_thread must stamp the session link onto the named card: {linked_card}"
    );

    let unlinked = rpc(
        &harness.rpc_base,
        4,
        "openhuman.todos_set_session_thread",
        json!({ "thread_id": thread, "id": first_id }),
    )
    .await;
    let unlinked = ok(&unlinked, "todos_set_session_thread clear");
    let unlinked_card = cards(unlinked, "todos_set_session_thread clear")
        .iter()
        .find(|card| card.get("id").and_then(Value::as_str) == Some(first_id.as_str()))
        .expect("the card survives the clear");
    assert!(
        unlinked_card
            .get("sessionThreadId")
            .or_else(|| unlinked_card.get("session_thread_id"))
            .and_then(Value::as_str)
            .is_none(),
        "an omitted sessionThreadId must clear the link, not leave it: {unlinked_card}"
    );

    // Clear empties the board, and the emptiness survives a re-read.
    let cleared = rpc(
        &harness.rpc_base,
        5,
        "openhuman.todos_clear",
        json!({ "thread_id": thread }),
    )
    .await;
    let cleared = ok(&cleared, "todos_clear");
    assert!(
        cards(cleared, "todos_clear").is_empty(),
        "clear must return an empty board: {cleared}"
    );

    let after_clear = rpc(
        &harness.rpc_base,
        6,
        "openhuman.todos_list",
        json!({ "thread_id": thread }),
    )
    .await;
    assert!(
        cards(
            ok(&after_clear, "todos_list after clear"),
            "todos_list after clear"
        )
        .is_empty(),
        "the cleared board must still be empty when re-read from disk"
    );

    harness.join.abort();
}

/// `decide_plan` on a card awaiting approval, plus the two failure paths the
/// board's write handlers share: a blank `thread_id` and an unknown card id.
#[tokio::test]
async fn todos_decide_plan_approves_and_rejects_and_refuses_bad_input() {
    let _lock = env_lock();
    let harness = setup(Vec::new()).await;
    let thread = "todos-e2e-thread-b";

    let seeded = rpc(
        &harness.rpc_base,
        10,
        "openhuman.todos_replace",
        json!({
            "thread_id": thread,
            "cards": [
                { "id": "", "title": "ship the thing", "status": "awaiting_approval" },
                { "id": "", "title": "ship the other thing", "status": "awaiting_approval" }
            ]
        }),
    )
    .await;
    let seeded = ok(&seeded, "seed for decide_plan");
    let seeded_cards = cards(seeded, "seed for decide_plan");
    assert_eq!(seeded_cards.len(), 2, "two cards seeded: {seeded}");
    let approve_id = seeded_cards[0]["id"].as_str().expect("card id").to_string();
    let reject_id = seeded_cards[1]["id"].as_str().expect("card id").to_string();

    let approved = rpc(
        &harness.rpc_base,
        11,
        "openhuman.todos_decide_plan",
        json!({ "thread_id": thread, "id": approve_id, "approve": true }),
    )
    .await;
    let approved = ok(&approved, "todos_decide_plan approve");
    let approved_status = cards(approved, "todos_decide_plan approve")
        .iter()
        .find(|card| card["id"].as_str() == Some(approve_id.as_str()))
        .and_then(|card| card.get("status"))
        .and_then(Value::as_str)
        .expect("approved card keeps a status")
        .to_string();
    assert_ne!(
        approved_status, "awaiting_approval",
        "approving must move the card off awaiting_approval, got {approved_status}"
    );
    assert_ne!(
        approved_status, "rejected",
        "approving must not reject the card"
    );

    let rejected = rpc(
        &harness.rpc_base,
        12,
        "openhuman.todos_decide_plan",
        json!({ "thread_id": thread, "id": reject_id, "approve": false }),
    )
    .await;
    let rejected = ok(&rejected, "todos_decide_plan reject");
    assert_eq!(
        cards(rejected, "todos_decide_plan reject")
            .iter()
            .find(|card| card["id"].as_str() == Some(reject_id.as_str()))
            .and_then(|card| card.get("status"))
            .and_then(Value::as_str),
        Some("rejected"),
        "rejecting must set the card's status to rejected: {rejected}"
    );

    // Failure path 1: a blank thread_id is refused before any store is opened.
    let blank_thread = rpc(
        &harness.rpc_base,
        13,
        "openhuman.todos_decide_plan",
        json!({ "thread_id": "   ", "id": approve_id, "approve": true }),
    )
    .await;
    assert!(
        error_message(&blank_thread, "todos_decide_plan blank thread")
            .contains("thread_id must not be empty"),
        "a blank thread_id must be named in the error, got: {blank_thread}"
    );

    // Failure path 2: an unknown card id must error rather than silently
    // returning an unchanged board — a no-op success here would let the UI
    // report a decision that never landed.
    let unknown_card = rpc(
        &harness.rpc_base,
        14,
        "openhuman.todos_decide_plan",
        json!({ "thread_id": thread, "id": "no-such-card-id", "approve": true }),
    )
    .await;
    assert!(
        unknown_card.get("error").is_some(),
        "deciding an unknown card must be an error, not a silent no-op: {unknown_card}"
    );

    harness.join.abort();
}

/// The run-record surface: `run_list`, `run_get`, and `reclaim_stale` on a
/// thread with no runs — the empty-state contract every caller hits first.
#[tokio::test]
async fn todos_run_records_and_reclaim_report_an_honest_empty_state() {
    let _lock = env_lock();
    let harness = setup(Vec::new()).await;
    let thread = "todos-e2e-thread-c";

    let runs = rpc(
        &harness.rpc_base,
        20,
        "openhuman.todos_run_list",
        json!({ "thread_id": thread }),
    )
    .await;
    let runs = ok(&runs, "todos_run_list");
    assert_eq!(
        runs.as_array().map(Vec::len),
        Some(0),
        "a thread with no runs must list an empty array, not null or an object: {runs}"
    );

    // Same, filtered by a card that has never run.
    let filtered = rpc(
        &harness.rpc_base,
        21,
        "openhuman.todos_run_list",
        json!({ "thread_id": thread, "cardId": "card-that-never-ran" }),
    )
    .await;
    assert_eq!(
        ok(&filtered, "todos_run_list filtered")
            .as_array()
            .map(Vec::len),
        Some(0),
        "filtering by an unknown card id must return an empty list: {filtered}"
    );

    // `run_get` for an unknown run is a documented `null`, not an error — the
    // Tasks board polls this and a thrown error would surface as a red toast.
    let missing = rpc(
        &harness.rpc_base,
        22,
        "openhuman.todos_run_get",
        json!({ "thread_id": thread, "runId": "no-such-run" }),
    )
    .await;
    assert_eq!(
        ok(&missing, "todos_run_get missing"),
        &Value::Null,
        "run_get for an unknown run id must be null: {missing}"
    );

    // `reclaim_stale` with nothing to reclaim must still report the counters,
    // so a caller can tell "scanned, found none" from "did not scan".
    let reclaimed = rpc(
        &harness.rpc_base,
        23,
        "openhuman.todos_reclaim_stale",
        json!({ "thread_id": thread, "heartbeatStaleSecs": 1, "claimTtlSecs": 1, "maxReclaimCount": 1 }),
    )
    .await;
    let reclaimed = ok(&reclaimed, "todos_reclaim_stale");
    assert_eq!(
        reclaimed.get("reclaimedCount").and_then(Value::as_u64),
        Some(0),
        "nothing to reclaim must be reported as 0, not omitted: {reclaimed}"
    );
    assert_eq!(
        reclaimed.get("blockedCount").and_then(Value::as_u64),
        Some(0),
        "nothing blocked must be reported as 0: {reclaimed}"
    );
    assert_eq!(
        reclaimed
            .get("details")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0),
        "an empty reclaim must carry an empty details array: {reclaimed}"
    );

    // The run surface refuses a blank thread just like the card surface does.
    let blank = rpc(
        &harness.rpc_base,
        24,
        "openhuman.todos_run_list",
        json!({ "thread_id": "" }),
    )
    .await;
    assert!(
        error_message(&blank, "todos_run_list blank thread")
            .contains("thread_id must not be empty"),
        "run_list must refuse a blank thread_id: {blank}"
    );

    harness.join.abort();
}

/// `todos_replace` refuses a card missing any of the mandatory trio.
///
/// These three refusals are the *handler's* behaviour and are unchanged by
/// #6087, which corrected the schema rather than the handler. What changed is
/// that they are now **documented**: the catalog spells `id`, `title` and
/// `status` as required (see `replace_cards_input`), so a caller can no longer
/// walk into them by reading the schema. They are pinned here because the
/// refusals themselves are the contract — `TaskBoardCard`'s `id`, `title` and
/// `status` carry no `#[serde(default)]`, so every one of them must be
/// **present**. Two consequences, both pinned below:
///
/// 1. The obvious spelling — `content`, which is what the sibling `todos_add`
///    and `todos_edit` inputs call the very same text — is rejected outright.
/// 2. "id may be empty" is about the string, not the key: omitting `status`
///    fails even with a well-formed title.
///
/// The assertions are written against what the code *does*. The companion
/// case `todos_replace_accepts_a_card_built_only_from_its_declared_schema`
/// proves the other half: that the catalog now carries enough to build a card
/// that these refusals let through.
#[tokio::test]
async fn todos_replace_rejects_the_card_shape_its_own_schema_implies() {
    let _lock = env_lock();
    let harness = setup(Vec::new()).await;
    let thread = "todos-e2e-thread-shape";

    // `content` — the name every other todos input uses for this field.
    let with_content = rpc(
        &harness.rpc_base,
        40,
        "openhuman.todos_replace",
        json!({
            "thread_id": thread,
            "cards": [{ "id": "", "content": "spelled the sibling way", "status": "todo" }]
        }),
    )
    .await;
    let message = error_message(&with_content, "todos_replace with `content`");
    assert!(
        message.contains("missing field") && message.contains("title"),
        "the `content` spelling must fail naming `title`, got: {message}"
    );

    // A well-formed title, but no status.
    let no_status = rpc(
        &harness.rpc_base,
        41,
        "openhuman.todos_replace",
        json!({ "thread_id": thread, "cards": [{ "id": "", "title": "no status here" }] }),
    )
    .await;
    let message = error_message(&no_status, "todos_replace without `status`");
    assert!(
        message.contains("missing field") && message.contains("status"),
        "an absent `status` must be rejected, got: {message}"
    );

    // And a well-formed title with no `id` key at all — "id may be empty" does
    // not extend to omitting it.
    let no_id = rpc(
        &harness.rpc_base,
        42,
        "openhuman.todos_replace",
        json!({ "thread_id": thread, "cards": [{ "title": "no id key", "status": "todo" }] }),
    )
    .await;
    let message = error_message(&no_id, "todos_replace without `id`");
    assert!(
        message.contains("missing field") && message.contains("id"),
        "an absent `id` key must be rejected even though an empty `id` is fine, got: {message}"
    );

    // Nothing partial was written on the way through the three refusals.
    let listed = rpc(
        &harness.rpc_base,
        43,
        "openhuman.todos_list",
        json!({ "thread_id": thread }),
    )
    .await;
    assert!(
        cards(
            ok(&listed, "board after refused replaces"),
            "board after refused replaces"
        )
        .is_empty(),
        "a rejected replace must leave the board untouched: {listed}"
    );

    harness.join.abort();
}

/// A card built **only** from what the catalog declares is accepted (#6087).
///
/// This is the regression test for the schema defect, and it is deliberately
/// written so that it cannot pass by accident. Rather than hard-coding a card
/// that happens to work, it *reads the declared schema* and constructs the card
/// from it: every field the catalog marks `required` is populated, and the
/// `status` value is taken from the declared enum's own variant list. If the
/// declaration goes back to a bare `TypeSchema::Json`, or names a field the
/// handler does not deserialize, or spells `title` as `content`, or offers a
/// `status` variant `TaskCardStatus` will not parse, this fails — because the
/// card it builds is only ever as correct as the schema it read.
#[tokio::test]
async fn todos_replace_accepts_a_card_built_only_from_its_declared_schema() {
    let _lock = env_lock();
    let harness = setup(Vec::new()).await;
    let thread = "todos-e2e-thread-from-schema";

    let catalog = schema_catalog(&harness.rpc_base).await;
    let entry = catalog_entry(&catalog, "openhuman.todos_replace");

    let cards_input = entry
        .get("inputs")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("todos_replace declares no inputs: {entry}"))
        .iter()
        .find(|i| i.get("name").and_then(Value::as_str) == Some("cards"))
        .unwrap_or_else(|| panic!("todos_replace declares no `cards` input: {entry}"));

    // The declaration must describe an array of objects, not an opaque blob.
    // `TypeSchema::Json` serialises as the bare string "Json", which is exactly
    // what this controller used to publish and what made it unconstructible.
    let card_fields = cards_input
        .get("ty")
        .and_then(|t| t.get("Array"))
        .and_then(|a| a.get("Object"))
        .and_then(|o| o.get("fields"))
        .and_then(Value::as_array)
        .unwrap_or_else(|| {
            panic!(
                "`cards` must declare an array of objects with named fields; \
                 an opaque type cannot be built from. Got ty = {}",
                cards_input.get("ty").unwrap_or(&Value::Null)
            )
        });

    // Build a card from the declaration alone.
    let mut card = serde_json::Map::new();
    let mut required_seen: Vec<String> = Vec::new();
    for field in card_fields {
        if field.get("required").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let name = field
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("declared field without a name: {field}"))
            .to_string();
        let ty = field.get("ty").unwrap_or(&Value::Null);
        let value = if let Some(variants) = ty.get("Enum").and_then(|e| e.get("variants")) {
            // Take the enum's own first variant — if the schema advertises a
            // value the wire type cannot parse, the dispatch below fails.
            variants
                .as_array()
                .and_then(|v| v.first())
                .cloned()
                .unwrap_or_else(|| panic!("declared enum with no variants: {field}"))
        } else if name == "id" {
            // The one documented special case: the key is required, the string
            // may be empty, and the server then generates the id.
            Value::String(String::new())
        } else {
            Value::String(format!("built from the schema: {name}"))
        };
        required_seen.push(name.clone());
        card.insert(name, value);
    }

    // EXACTLY the trio, not merely "includes" it. `TaskBoardCard` has exactly
    // three fields without a serde default; if a fourth is ever marked required
    // in the catalog, a caller obeying the schema would send a key the handler
    // does not need, and the loop above would happily paper over it.
    let mut sorted = required_seen.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        vec!["id".to_string(), "status".to_string(), "title".to_string()],
        "the catalog must mark exactly `id`, `title` and `status` required — \
         those are the only three `TaskBoardCard` fields with no serde default"
    );

    // The six `#[serde(default)]` fields must NOT be declared `Option(...)`.
    //
    // This is asserted against the *declaration*, not against a dispatch,
    // because a dispatch cannot see it: `core::all::check_type` short-circuits
    // on `Value::Null` for every declared type, so `null` reaches the handler
    // either way and is rejected by serde either way. The declaration is
    // therefore documentation-only at runtime — and documentation-only is
    // exactly the thing that rots unnoticed, so it gets a direct assertion.
    for name in [
        "plan",
        "allowedTools",
        "acceptanceCriteria",
        "evidence",
        "order",
        "updatedAt",
    ] {
        let field = card_fields
            .iter()
            .find(|f| f.get("name").and_then(Value::as_str) == Some(name))
            .unwrap_or_else(|| panic!("the card must declare `{name}`"));
        assert!(
            field.get("ty").and_then(|t| t.get("Option")).is_none(),
            "`{name}` is `#[serde(default)]` on a non-Option field upstream, so an \
             explicit null fails to deserialize. Declaring it `Option(...)` advertises \
             null as valid and puts the catalog back to describing a call the handler \
             rejects. Declared ty was: {}",
            field.get("ty").unwrap_or(&Value::Null)
        );
        assert_eq!(
            field.get("required").and_then(Value::as_bool),
            Some(false),
            "`{name}` has a serde default, so it must be optional-by-omission"
        );
    }

    // Every *optional* enum field must advertise exactly the variants its wire
    // type parses, and each must round-trip. The generated card above only
    // populates required fields, so without this an optional enum declared as a
    // free `Option(String)` — which `approvalMode` was — would let a
    // catalog-valid value like "sometimes" through the schema and straight into
    // an `invalid params` from the handler.
    for (name, expected) in [("approvalMode", ["not_required", "required"].as_slice())] {
        let field = card_fields
            .iter()
            .find(|f| f.get("name").and_then(Value::as_str) == Some(name))
            .unwrap_or_else(|| panic!("the card must declare `{name}`"));
        let variants = field
            .get("ty")
            .and_then(|t| t.get("Option"))
            .and_then(|inner| inner.get("Enum"))
            .and_then(|e| e.get("variants"))
            .and_then(Value::as_array)
            .unwrap_or_else(|| {
                panic!(
                    "`{name}` is backed by a closed enum upstream, so the catalog \
                     must declare its variants rather than a free string. \
                     Declared ty was: {}",
                    field.get("ty").unwrap_or(&Value::Null)
                )
            })
            .clone();

        // Probing only what is listed would let a *removed* variant through:
        // drop `not_required` and the surviving `required` probe still passes.
        // The complete set is the thing being pinned, so assert it before
        // probing. `expected` rides on the loop's own list so a second field
        // brings its own set rather than widening a shared literal.
        let mut declared = variants
            .iter()
            .map(|v| {
                v.as_str()
                    .unwrap_or_else(|| panic!("`{name}`'s declared variants must be strings"))
            })
            .collect::<Vec<_>>();
        declared.sort_unstable();
        assert_eq!(
            declared, expected,
            "`{name}` must declare exactly the variants its wire type parses"
        );

        for (i, variant) in variants.iter().enumerate() {
            let mut probe = card.clone();
            probe.insert("id".to_string(), Value::String(format!("enum-{name}-{i}")));
            probe.insert(name.to_string(), variant.clone());
            let response = rpc(
                &harness.rpc_base,
                90 + i as i64,
                "openhuman.todos_replace",
                json!({ "thread_id": thread, "cards": [Value::Object(probe)] }),
            )
            .await;
            ok(
                &response,
                &format!("todos_replace with the declared `{name}` variant {variant}"),
            );
        }
    }

    // Every advertised status variant must actually parse. Taking only the
    // first would let a misspelled later variant (`in-progress` for
    // `in_progress`, say) sit in the catalog undetected.
    let status_variants: Vec<String> = card_fields
        .iter()
        .find(|f| f.get("name").and_then(Value::as_str) == Some("status"))
        .and_then(|f| f.get("ty"))
        .and_then(|t| t.get("Enum"))
        .and_then(|e| e.get("variants"))
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("`status` must declare an enum of variants"))
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    assert!(
        status_variants.len() >= 2,
        "a lifecycle enum with fewer than two variants is a declaration bug: {status_variants:?}"
    );
    for (i, variant) in status_variants.iter().enumerate() {
        let mut probe = card.clone();
        probe.insert("status".to_string(), Value::String(variant.clone()));
        probe.insert("id".to_string(), Value::String(format!("probe-{i}")));
        let response = rpc(
            &harness.rpc_base,
            70 + i as i64,
            "openhuman.todos_replace",
            json!({ "thread_id": thread, "cards": [Value::Object(probe)] }),
        )
        .await;
        ok(
            &response,
            &format!("todos_replace with the declared status variant {variant:?}"),
        );
    }

    let replaced = rpc(
        &harness.rpc_base,
        60,
        "openhuman.todos_replace",
        json!({ "thread_id": thread, "cards": [Value::Object(card)] }),
    )
    .await;
    let result = ok(
        &replaced,
        "todos_replace with a card built from its own schema",
    );

    // It was accepted AND stored — a schema that merely deserializes but drops
    // the card would be no better than the old one.
    let titles = card_titles(result, "replace built from schema");
    assert_eq!(
        titles,
        vec!["built from the schema: title".to_string()],
        "the card built from the declared schema must land on the board: {result:?}"
    );

    // The server generated an id for the empty one, as the comment promises.
    let stored = cards(result, "replace built from schema");
    let id = stored[0]
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("stored card carries no id: {stored:?}"));
    assert!(
        !id.is_empty(),
        "an empty `id` must be server-generated, got an empty string back"
    );

    harness.join.abort();
}

/// A `#[serde(default)]` collection may be **omitted**, but not sent as `null`
/// — and the catalog now says so (#6087).
///
/// `TaskBoardCard`'s `plan`, `allowedTools`, `acceptanceCriteria` and
/// `evidence` are `Vec<String>`; `order` is `u32`; `updatedAt` is `String`.
/// None is an `Option`, so `#[serde(default)]` covers an *absent* key and an
/// explicit `null` fails with `invalid type: null`.
///
/// The first draft of this schema declared all six as `Option(...)`, which
/// advertised `null` as valid and would have put the catalog straight back to
/// describing a call the handler rejects — the exact defect #6087 removes.
/// They are declared with their real types and `required: false` instead, and
/// this pins both halves of that contract.
#[tokio::test]
async fn todos_replace_defaulted_card_fields_may_be_omitted_but_not_null() {
    let _lock = env_lock();
    let harness = setup(Vec::new()).await;
    let thread = "todos-e2e-thread-defaults";

    // Omitting every defaulted field is accepted — that is what the serde
    // defaults are for, and what `required: false` advertises.
    let omitted = rpc(
        &harness.rpc_base,
        80,
        "openhuman.todos_replace",
        json!({
            "thread_id": thread,
            "cards": [{ "id": "", "title": "only the required trio", "status": "todo" }]
        }),
    )
    .await;
    let result = ok(&omitted, "todos_replace omitting every defaulted field");
    assert_eq!(
        card_titles(result, "omitted defaults"),
        vec!["only the required trio".to_string()],
        "a card carrying only the required trio must be accepted: {result:?}"
    );

    // An explicit null for a defaulted collection is refused. If the catalog
    // ever goes back to declaring these `Option(...)`, it will be promising a
    // shape this assertion proves the handler does not accept.
    for field in ["plan", "allowedTools", "acceptanceCriteria", "evidence"] {
        let mut card = serde_json::Map::new();
        card.insert("id".into(), json!(""));
        card.insert("title".into(), json!("null probe"));
        card.insert("status".into(), json!("todo"));
        card.insert(field.to_string(), Value::Null);

        let response = rpc(
            &harness.rpc_base,
            81,
            "openhuman.todos_replace",
            json!({ "thread_id": thread, "cards": [Value::Object(card)] }),
        )
        .await;
        let message = error_message(&response, &format!("todos_replace with {field}: null"));
        assert!(
            message.contains("invalid type: null") || message.contains(field),
            "an explicit null for the defaulted `{field}` must be refused, so the \
             catalog must not advertise it as nullable; got: {message}"
        );
    }

    harness.join.abort();
}
