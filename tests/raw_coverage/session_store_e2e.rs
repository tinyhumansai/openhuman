//! JSON-RPC E2E coverage for the session / artifact / compression store domains:
//! `session_import`, `tokenjuice` (compress + retrieve), `ai` (artifact CRUD)
//! and `test_support`.
//!
//! Every case boots the real axum JSON-RPC router (`build_core_http_router`)
//! against a per-test temp workspace, dispatches over HTTP, and
//! asserts on response *content*. Each namespace gets at least one failure
//! path, because that is where these handlers actually branch.
//!
//! This file is a **module** of the aggregated `raw_coverage_all` target, not a
//! target of its own — `build.rs` globs `tests/raw_coverage/` and generates the
//! `mod` list. Run with:
//!   `~/tinyhuman/ci-slot.sh cargo test --test raw_coverage_all \
//!      --features "$(bash scripts/ci/product-features.sh)" -- session_store`
//!
//! Because every suite in that binary shares one process, this file takes the
//! crate-wide `SHARED_ENV_LOCK` around each case and reads the live RPC bearer
//! back rather than assuming its own — see `env_lock` and `rpc_bearer`.
//!
//! ## One thing this file documents rather than asserts as correct
//!
//! **`test_support.*` is gated behind `e2e-test-support`,** which is not in
//! `scripts/ci/product-features.sh`. Under the product feature set those
//! five controllers must be *absent* — that gate is the reason the
//! destructive `test_reset` never ships — so the case here asserts the
//! absence, and the positive path is compiled in only when the feature is.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::http::header::AUTHORIZATION;
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;

/// Preferred bearer for this suite. It is only the *actual* bearer when this
/// module happens to be the first in the aggregated binary to initialise auth —
/// `core::auth::RPC_TOKEN` is a process-global `OnceLock` and first writer wins.
/// Never send this constant; send [`rpc_bearer`], which reads back whichever
/// token the process really validates.
const PREFERRED_RPC_TOKEN: &str = "session-store-e2e-token";

static AUTH_INIT: OnceLock<()> = OnceLock::new();

/// The crate-wide env lock. A lock private to this module would isolate
/// nothing: every `tests/raw_coverage/` suite is a module in the one
/// `raw_coverage_all` binary, so libtest runs them concurrently in one process
/// and only the shared mutex actually excludes another suite's `set_var`.
static ENV_LOCK: &OnceLock<Mutex<()>> = &crate::SHARED_ENV_LOCK;

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

/// `HOME` / `OPENHUMAN_WORKSPACE` are process-global, so every case in this
/// binary is serialised behind one lock.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn ensure_rpc_auth() {
    AUTH_INIT.get_or_init(|| {
        std::env::set_var(CORE_TOKEN_ENV_VAR, PREFERRED_RPC_TOKEN);
        let token_dir = std::env::temp_dir().join("openhuman-session-store-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
}

/// The bearer the running process actually validates.
///
/// `init_rpc_token` is idempotent on a process-global `OnceLock`, so in the
/// aggregated binary a sibling suite may have seeded a different token before
/// this module's first case ran. Hard-coding `PREFERRED_RPC_TOKEN` into the
/// header would then 401 every request here for a reason that has nothing to do
/// with the controller under test. Read the live value instead.
fn rpc_bearer() -> &'static str {
    ensure_rpc_auth();
    openhuman_core::core::auth::get_rpc_token()
        .expect("the RPC token must be initialised before a request is signed")
}

/// Write `config.toml` into `dir` — the workspace's parent, which is where
/// `resolve_config_dir_for_workspace` resolves it from `OPENHUMAN_WORKSPACE`.
fn write_min_config(dir: &Path) {
    std::fs::create_dir_all(dir).expect("create config dir");
    let cfg = r#"api_url = "http://127.0.0.1:9"
default_model = "e2e-model"
default_temperature = 0.2

[secrets]
encrypt = false

[local_ai]
enabled = false

[memory_tree]
embedding_strict = false
"#;
    std::fs::write(dir.join("config.toml"), cfg).expect("write config.toml");
    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(cfg).expect("test config must match schema");
}

struct Harness {
    _tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    workspace: PathBuf,
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

/// A fresh temp workspace per case, named through `OPENHUMAN_WORKSPACE` so the
/// test can seed files on disk (artifacts, `session_raw/` JSONL) at exactly the
/// path the handlers will read from. Each of the domains here keys on
/// `config.workspace_dir` alone, so a per-case workspace is safe — unlike the
/// memory-family suites, nothing in this file is bound once per process.
///
/// **`HOME` is deliberately left alone.** Loadable modules install under
/// `dirs::cache_dir()`, i.e. `$HOME/Library/Caches/openhuman/modules` on macOS
/// (`modules::ops::install_dir`), so repointing `HOME` at a tempdir would miss
/// the already-verified `tinyjuice` artifact and send the tokenjuice cases to
/// the network for a module the machine already has. The config is written
/// beside the workspace instead, which is where
/// `resolve_config_dir_for_workspace` looks when `OPENHUMAN_WORKSPACE` is set.
async fn setup() -> Harness {
    let tmp = tempdir().expect("tempdir");
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    write_min_config(tmp.path());

    let guards = vec![
        EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
    ];

    let (addr, join) = serve_rpc().await;
    Harness {
        _tmp: tmp,
        _guards: guards,
        workspace,
        rpc_base: format!("http://{addr}"),
        join,
    }
}

// ── RPC helpers ───────────────────────────────────────────────────────────

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

#[allow(dead_code)]
async fn schema_catalog(rpc_base: &str) -> Value {
    let url = format!("{}/schema", rpc_base.trim_end_matches('/'));
    reqwest::get(&url)
        .await
        .unwrap_or_else(|err| panic!("GET {url}: {err}"))
        .json::<Value>()
        .await
        .expect("schema json")
}

#[allow(dead_code)]
fn catalog_has(catalog: &Value, method: &str) -> bool {
    catalog
        .get("methods")
        .and_then(Value::as_array)
        .expect("schema methods array")
        .iter()
        .any(|entry| entry.get("method").and_then(Value::as_str) == Some(method))
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

// ── session_import ────────────────────────────────────────────────────────

/// `session_import.run` over a seeded legacy `session_raw/` directory:
/// dry-run plans without writing, the real run imports, and a second run is
/// idempotent because the item ledger says the work is done.
#[tokio::test]
async fn session_import_plans_imports_then_skips_on_rerun() {
    let _lock = env_lock();
    let harness = setup().await;

    let raw_dir = harness.workspace.join("session_raw");
    std::fs::create_dir_all(&raw_dir).expect("create session_raw");

    // The reader requires the first non-empty line to be the `_meta` header —
    // a transcript that starts with a message is rejected outright, so this
    // fixture is shaped like a real one rather than like a bare message log.
    let mut lines = vec![json!({
        "_meta": {
            "version": 1,
            "agent": "orchestrator",
            "dispatcher": "e2e",
            "created": "2026-09-07T10:00:00Z",
            "updated": "2026-09-07T10:05:00Z",
            "turn_count": 2,
            "input_tokens": 40,
            "output_tokens": 20,
            "cached_input_tokens": 0,
            "charged_amount_usd": 0.0,
            "thread_id": "e2e-session-import-thread",
        }
    })];
    for (role, content) in [
        ("user", "what is the capital of France?"),
        ("assistant", "Paris."),
        ("user", "and of Japan?"),
        ("assistant", "Tokyo."),
    ] {
        lines.push(json!({ "role": role, "content": content }));
    }
    let transcript = lines
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(raw_dir.join("20260907_capitals.jsonl"), transcript)
        .expect("write legacy transcript");

    // ── dry run: plans the work and writes nothing ──
    let planned = rpc(
        &harness.rpc_base,
        31_001,
        "openhuman.session_import_run",
        json!({ "dry_run": true }),
    )
    .await;
    let planned = payload(&planned, "session_import_run dry");
    assert_eq!(planned.get("dry_run").and_then(Value::as_bool), Some(true));
    assert_eq!(
        planned.get("scanned").and_then(Value::as_u64),
        Some(1),
        "the seeded transcript must be discovered: {planned}"
    );
    assert_eq!(
        planned.get("messages_written").and_then(Value::as_u64),
        Some(0),
        "a dry run writes no messages: {planned}"
    );
    let planned_item = planned
        .get("items")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .unwrap_or_else(|| panic!("dry run must report the item: {planned}"))
        .clone();
    assert_eq!(
        planned_item.get("action").and_then(Value::as_str),
        Some("would_import"),
        "dry-run action: {planned_item}"
    );
    // A dry run must not write the global marker either — proved below by the
    // real run still having work to do. Asserting on the store *directory* would
    // be wrong: `run_import` opens the stores before it decides to plan only.

    // ── real run: writes the messages into the TinyAgents store ──
    let imported = rpc(
        &harness.rpc_base,
        31_002,
        "openhuman.session_import_run",
        json!({}),
    )
    .await;
    let imported = payload(&imported, "session_import_run");
    assert_eq!(
        imported.get("dry_run").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        imported.get("imported").and_then(Value::as_u64),
        Some(1),
        "one source imported: {imported}"
    );
    assert_eq!(
        imported.get("failed").and_then(Value::as_u64),
        Some(0),
        "no failures: {imported}"
    );
    assert_eq!(
        imported.get("messages_written").and_then(Value::as_u64),
        Some(4),
        "the four message lines are written; the `_meta` header is not a message: {imported}"
    );
    let item = imported
        .get("items")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .unwrap_or_else(|| panic!("the real run must report the item: {imported}"))
        .clone();
    assert_eq!(item.get("action").and_then(Value::as_str), Some("imported"));
    assert_eq!(
        item.get("messages").and_then(Value::as_u64),
        Some(4),
        "the per-item count must agree with the run total: {item}"
    );
    assert_eq!(
        item.get("thread_id").and_then(Value::as_str),
        Some("e2e-session-import-thread"),
        "the thread id must come from the transcript's `_meta`, not be synthesized: {item}"
    );
    assert_eq!(
        item.get("stream").and_then(Value::as_str),
        Some("session.20260907_capitals.messages"),
        "messages go to a per-session journal stream named from the stem: {item}"
    );
    assert!(
        harness.workspace.join("tinyagents_store").exists(),
        "the real run must create the store"
    );

    // ── re-run: idempotent ──
    let again = rpc(
        &harness.rpc_base,
        31_003,
        "openhuman.session_import_run",
        json!({}),
    )
    .await;
    let again = payload(&again, "session_import_run rerun");
    assert_eq!(
        again.get("messages_written").and_then(Value::as_u64),
        Some(0),
        "a second run must write nothing: {again}"
    );
    assert!(
        again.get("already_done").and_then(Value::as_bool) == Some(true)
            || again.get("skipped").and_then(Value::as_u64) == Some(1),
        "the re-run must be short-circuited by the marker or the item ledger: {again}"
    );

    harness.join.abort();
}

/// Failure path: `dry_run` is declared `Option<Bool>` in the schema, so the
/// dispatcher's type check refuses a string before `run_import` opens a store or
/// scans a directory.
#[tokio::test]
async fn session_import_rejects_malformed_params() {
    let _lock = env_lock();
    let harness = setup().await;

    let bad = rpc(
        &harness.rpc_base,
        31_101,
        "openhuman.session_import_run",
        json!({ "dry_run": "yes-please" }),
    )
    .await;
    assert!(
        error_message(&bad, "session_import_run with a string dry_run")
            .contains("invalid type for param 'dry_run' in session_import.run"),
        "a wrong-typed param must be refused by name, with both types: {bad}"
    );

    harness.join.abort();
}

// ── tokenjuice ────────────────────────────────────────────────────────────

/// `tokenjuice.detect` → `.compress` → `.retrieve`: the full CCR round trip,
/// plus the invariants that must hold whichever branch the router takes.
///
/// A 400-row JSON array is the fixture because it is the one kind the existing
/// `json_rpc_e2e` coverage already pins `detect` on, so the cross-controller
/// assertion below rests on a classification asserted independently elsewhere.
/// In practice it compresses ~19× and takes the lossy path, so the `retrieve`
/// leg runs for real and the original is compared byte for byte.
///
/// **The lossy branch is guarded rather than assumed, and that is deliberate.**
/// Whether the router compresses is its own decision under a hint this RPC
/// cannot fully steer: `handle_compress` builds `ContentHint` from `tool_name`
/// alone (`inference/tokenjuice/schemas.rs:241-248`), leaving `explicit`,
/// `mime`, `extension` and `query` unreachable. Asserting `lossy == true`
/// unconditionally would pin a detector heuristic, not a contract — and the
/// same controller passes a 16 KB uniform *log* straight through untouched, so
/// the heuristic is not a stable thing to assert on. See
/// `~/tinyhuman/bugs/e2e-wave-tokenjuice-compress-drops-most-of-the-content-hint.md`.
///
/// What is asserted unconditionally is what must hold on every branch:
///
///  1. `compress` and `detect` agree on the content kind — two controllers, one
///     classifier, and nothing else checks they stay in step;
///  2. the reported byte counts describe the actual strings, not estimates;
///  3. **nothing is lost**: a lossy compaction is recoverable through the token
///     byte-for-byte, and a pass-through returns the input unchanged.
#[tokio::test]
async fn tokenjuice_compress_agrees_with_detect_and_never_loses_content() {
    let _lock = env_lock();
    let harness = setup().await;

    // A JSON array: the one kind the existing `json_rpc_e2e` coverage already
    // pins `detect` on, so the cross-controller assertion below rests on a
    // classification that is independently asserted elsewhere.
    let rows: Vec<Value> = (0..400)
        .map(|i| json!({ "id": i, "name": format!("row-{i}"), "status": "ok", "score": i * 3 }))
        .collect();
    let content = Value::Array(rows).to_string();
    assert!(
        content.len() > 8_000,
        "the fixture must clear the 2048-byte compression floor with margin"
    );

    let detected = rpc(
        &harness.rpc_base,
        32_001,
        "openhuman.tokenjuice_detect",
        json!({ "content": content }),
    )
    .await;
    let detected_kind = payload(&detected, "tokenjuice_detect")
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("detect must report a kind: {detected}"))
        .to_string();
    assert_eq!(
        detected_kind, "json",
        "a JSON array must classify as json: {detected}"
    );

    let compressed = rpc(
        &harness.rpc_base,
        32_002,
        "openhuman.tokenjuice_compress",
        json!({ "content": content, "tool_name": "session_store_e2e" }),
    )
    .await;
    let compressed = payload(&compressed, "tokenjuice_compress");

    // (1) One classifier, two controllers.
    assert_eq!(
        compressed.get("kind").and_then(Value::as_str),
        Some(detected_kind.as_str()),
        "compress must route on the same kind detect reports: {compressed}"
    );

    // (2) The byte counts describe the actual strings.
    let text = compressed
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("compress must return the routed text: {compressed}"));
    assert_eq!(
        compressed.get("originalBytes").and_then(Value::as_u64),
        Some(content.len() as u64),
        "originalBytes must be the byte length handed in: {compressed}"
    );
    assert_eq!(
        compressed.get("compactedBytes").and_then(Value::as_u64),
        Some(text.len() as u64),
        "compactedBytes is documented as the byte length of `text`: {compressed}"
    );

    // (3) Nothing is lost, on whichever branch the router took.
    let applied = compressed
        .get("applied")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            panic!("compress must report whether it changed the content: {compressed}")
        });
    let lossy = compressed
        .get("lossy")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("compress must report whether it dropped data: {compressed}"));

    if !applied {
        assert!(!lossy, "a pass-through cannot be lossy: {compressed}");
        assert_eq!(
            text, content,
            "a pass-through must return the input unchanged — byte for byte"
        );
        assert_eq!(
            compressed.get("compressor").and_then(Value::as_str),
            Some("none"),
            "a pass-through must name no compressor: {compressed}"
        );
        assert_eq!(
            compressed.get("ccrToken"),
            Some(&Value::Null),
            "nothing was offloaded, so there is no token to hand back: {compressed}"
        );
    } else {
        assert!(
            text.len() < content.len(),
            "an applied compaction must shrink the payload: {compressed}"
        );
        assert_ne!(
            compressed.get("compressor").and_then(Value::as_str),
            Some("none"),
            "an applied compaction must name the compressor that fired: {compressed}"
        );
    }

    if lossy {
        let token = compressed
            .get("ccrToken")
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                panic!(
                    "a lossy compaction over the CCR threshold must be recoverable: {compressed}"
                )
            })
            .to_string();

        let retrieved = rpc(
            &harness.rpc_base,
            32_003,
            "openhuman.tokenjuice_retrieve",
            json!({ "token": token }),
        )
        .await;
        let retrieved = payload(&retrieved, "tokenjuice_retrieve");
        assert_eq!(
            retrieved.get("found").and_then(Value::as_bool),
            Some(true),
            "the token minted by compress must resolve: {retrieved}"
        );
        assert_eq!(
            retrieved.get("content").and_then(Value::as_str),
            Some(content.as_str()),
            "retrieve must return the original verbatim — a compaction that \
             cannot be undone byte-for-byte has lost data"
        );
    }

    harness.join.abort();
}

/// Failure paths: `compress` needs content, and `retrieve` of a token that was
/// never minted is a clean `found: false` rather than an error.
#[tokio::test]
async fn tokenjuice_refuses_empty_input_and_misses_cleanly() {
    let _lock = env_lock();
    let harness = setup().await;

    let no_content = rpc(
        &harness.rpc_base,
        32_101,
        "openhuman.tokenjuice_compress",
        json!({ "tool_name": "session_store_e2e" }),
    )
    .await;
    assert!(
        error_message(&no_content, "tokenjuice_compress without content").contains("content"),
        "the refusal must name the missing field: {no_content}"
    );

    let miss = rpc(
        &harness.rpc_base,
        32_102,
        "openhuman.tokenjuice_retrieve",
        json!({ "token": "ccr:definitely-not-a-real-token" }),
    )
    .await;
    let miss = payload(&miss, "tokenjuice_retrieve miss");
    assert_eq!(
        miss.get("found").and_then(Value::as_bool),
        Some(false),
        "an unknown token is a miss, not an error: {miss}"
    );
    assert_eq!(
        miss.get("content"),
        Some(&Value::Null),
        "a miss carries no content: {miss}"
    );

    let no_token = rpc(
        &harness.rpc_base,
        32_103,
        "openhuman.tokenjuice_retrieve",
        json!({}),
    )
    .await;
    assert!(
        error_message(&no_token, "tokenjuice_retrieve without token").contains("token"),
        "the refusal must name the missing field: {no_token}"
    );

    harness.join.abort();
}

// ── ai (artifacts) ────────────────────────────────────────────────────────

/// Write a `meta.json` for one artifact directly into the workspace, the way
/// the producer tools do, and return its id.
fn seed_artifact(
    workspace: &Path,
    id: &str,
    kind: &str,
    title: &str,
    created_at: &str,
    thread_id: Option<&str>,
) {
    let dir = workspace.join("artifacts").join(id);
    std::fs::create_dir_all(&dir).expect("create artifact dir");
    let body = format!("{title} body");
    let filename = format!("{id}.md");
    std::fs::write(dir.join(&filename), &body).expect("write artifact file");

    let mut meta = json!({
        "id": id,
        "kind": kind,
        "title": title,
        "path": format!("{id}/{filename}"),
        "size_bytes": body.len(),
        "status": "ready",
        "created_at": created_at,
    });
    if let Some(tid) = thread_id {
        meta.as_object_mut()
            .expect("meta object")
            .insert("thread_id".into(), json!(tid));
    }
    std::fs::write(
        dir.join("meta.json"),
        serde_json::to_string_pretty(&meta).expect("serialize meta"),
    )
    .expect("write meta.json");
}

/// The artifact read/delete surface over three seeded artifacts: newest-first
/// ordering, per-thread filtering that also narrows `total`, pagination, `get`
/// resolving an absolute path, and `delete` actually removing the row.
#[tokio::test]
async fn ai_artifacts_list_filter_get_and_delete() {
    let _lock = env_lock();
    let harness = setup().await;

    seed_artifact(
        &harness.workspace,
        "artifact-oldest",
        "document",
        "Oldest note",
        "2026-01-01T00:00:00Z",
        Some("thread-alpha"),
    );
    seed_artifact(
        &harness.workspace,
        "artifact-middle",
        "presentation",
        "Q2 deck",
        "2026-05-01T00:00:00Z",
        None,
    );
    seed_artifact(
        &harness.workspace,
        "artifact-newest",
        "document",
        "Newest note",
        "2026-09-01T00:00:00Z",
        Some("thread-alpha"),
    );

    // ── list: newest first, total is the workspace count ──
    let list = rpc(
        &harness.rpc_base,
        33_001,
        "openhuman.ai_list_artifacts",
        json!({}),
    )
    .await;
    let list = payload(&list, "ai_list_artifacts");
    assert_eq!(list.get("total").and_then(Value::as_u64), Some(3));
    assert_eq!(list.get("limit").and_then(Value::as_u64), Some(50));
    let ids: Vec<&str> = list
        .get("artifacts")
        .and_then(Value::as_array)
        .expect("artifacts array")
        .iter()
        .filter_map(|a| a.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(
        ids,
        vec!["artifact-newest", "artifact-middle", "artifact-oldest"],
        "artifacts must be sorted by created_at descending"
    );

    // ── thread filter: narrows the page AND the total ──
    let filtered = rpc(
        &harness.rpc_base,
        33_002,
        "openhuman.ai_list_artifacts",
        json!({ "thread_id": "thread-alpha" }),
    )
    .await;
    let filtered = payload(&filtered, "ai_list_artifacts filtered");
    assert_eq!(
        filtered.get("total").and_then(Value::as_u64),
        Some(2),
        "total must reflect the filtered set, not the workspace: {filtered}"
    );
    let filtered_ids: Vec<&str> = filtered
        .get("artifacts")
        .and_then(Value::as_array)
        .expect("artifacts array")
        .iter()
        .filter_map(|a| a.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(filtered_ids, vec!["artifact-newest", "artifact-oldest"]);

    // ── pagination: offset skips into the sorted list ──
    let page = rpc(
        &harness.rpc_base,
        33_003,
        "openhuman.ai_list_artifacts",
        json!({ "offset": 1, "limit": 1 }),
    )
    .await;
    let page = payload(&page, "ai_list_artifacts paged");
    assert_eq!(page.get("total").and_then(Value::as_u64), Some(3));
    assert_eq!(page.get("offset").and_then(Value::as_u64), Some(1));
    let paged_ids: Vec<&str> = page
        .get("artifacts")
        .and_then(Value::as_array)
        .expect("artifacts array")
        .iter()
        .filter_map(|a| a.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(paged_ids, vec!["artifact-middle"]);

    // ── get: returns the meta plus a resolved absolute path ──
    let got = rpc(
        &harness.rpc_base,
        33_004,
        "openhuman.ai_get_artifact",
        json!({ "artifact_id": "artifact-middle" }),
    )
    .await;
    let got = payload(&got, "ai_get_artifact");
    assert_eq!(got.get("title").and_then(Value::as_str), Some("Q2 deck"));
    assert_eq!(
        got.get("kind").and_then(Value::as_str),
        Some("presentation")
    );
    let absolute = got
        .get("absolute_path")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("get must add absolute_path: {got}"));
    assert!(
        Path::new(absolute).exists(),
        "absolute_path must point at the file on disk: {absolute}"
    );

    // ── delete: removes it from the listing ──
    let deleted = rpc(
        &harness.rpc_base,
        33_005,
        "openhuman.ai_delete_artifact",
        json!({ "artifact_id": "artifact-oldest" }),
    )
    .await;
    let deleted = payload(&deleted, "ai_delete_artifact");
    assert_eq!(deleted.get("deleted").and_then(Value::as_bool), Some(true));
    assert_eq!(
        deleted.get("artifact_id").and_then(Value::as_str),
        Some("artifact-oldest")
    );

    let after = rpc(
        &harness.rpc_base,
        33_006,
        "openhuman.ai_list_artifacts",
        json!({}),
    )
    .await;
    let after = payload(&after, "ai_list_artifacts after delete");
    assert_eq!(
        after.get("total").and_then(Value::as_u64),
        Some(2),
        "the deleted artifact must be gone: {after}"
    );
    assert!(
        !harness
            .workspace
            .join("artifacts")
            .join("artifact-oldest")
            .exists(),
        "delete must remove the directory too"
    );

    harness.join.abort();
}

/// Failure paths for the artifact surface: a traversal id is refused before
/// any filesystem access, an absent id is an error, and `regenerate` refuses
/// both a non-presentation artifact and a call with no routing context.
#[tokio::test]
async fn ai_artifacts_reject_traversal_absence_and_unregenerable_kinds() {
    let _lock = env_lock();
    let harness = setup().await;

    seed_artifact(
        &harness.workspace,
        "artifact-doc",
        "document",
        "A document",
        "2026-03-01T00:00:00Z",
        None,
    );

    let traversal = rpc(
        &harness.rpc_base,
        33_101,
        "openhuman.ai_get_artifact",
        json!({ "artifact_id": "../../etc/passwd" }),
    )
    .await;
    assert!(
        error_message(&traversal, "ai_get_artifact traversal").contains("must not contain '/'"),
        "an id with a path separator must be refused by the validator: {traversal}"
    );

    let empty = rpc(
        &harness.rpc_base,
        33_102,
        "openhuman.ai_delete_artifact",
        json!({ "artifact_id": "   " }),
    )
    .await;
    assert!(
        error_message(&empty, "ai_delete_artifact blank id").contains("must not be empty"),
        "a whitespace-only id trims to empty and must be refused: {empty}"
    );

    let missing = rpc(
        &harness.rpc_base,
        33_103,
        "openhuman.ai_get_artifact",
        json!({ "artifact_id": "artifact-that-does-not-exist" }),
    )
    .await;
    assert!(
        error_message(&missing, "ai_get_artifact absent").contains("artifact-that-does-not-exist"),
        "the error must identify the artifact: {missing}"
    );

    // regenerate needs routing context for the socket events it triggers.
    let no_routing = rpc(
        &harness.rpc_base,
        33_104,
        "openhuman.ai_regenerate",
        json!({ "artifact_id": "artifact-doc", "thread_id": "", "client_id": "" }),
    )
    .await;
    assert!(
        error_message(&no_routing, "ai_regenerate without routing")
            .contains("thread_id + client_id"),
        "regenerate must refuse without event routing: {no_routing}"
    );

    // Only presentations persist the args a re-dispatch needs.
    let wrong_kind = rpc(
        &harness.rpc_base,
        33_105,
        "openhuman.ai_regenerate",
        json!({
            "artifact_id": "artifact-doc",
            "thread_id": "thread-alpha",
            "client_id": "client-1",
        }),
    )
    .await;
    let message = error_message(&wrong_kind, "ai_regenerate on a document");
    assert!(
        message.contains("only supported for presentations") && message.contains("document"),
        "the refusal must name the rule and the actual kind, got: {message}"
    );

    harness.join.abort();
}

// ── test_support ──────────────────────────────────────────────────────────

/// The `test_support.*` controllers are compiled in only under
/// `e2e-test-support`, which the product feature set deliberately omits — that
/// gate is what keeps the destructive `openhuman.test_reset` out of shipped
/// binaries. Under the product features the five methods must be absent from
/// the schema catalog *and* unroutable, and this case fails if either the
/// `#[cfg]` at `src/core/all.rs` or the feature list stops holding that line.
#[cfg(not(feature = "e2e-test-support"))]
#[tokio::test]
async fn test_support_controllers_are_absent_without_their_feature() {
    let _lock = env_lock();
    let harness = setup().await;

    let catalog = schema_catalog(&harness.rpc_base).await;
    for method in [
        "openhuman.test_support_workspace_root",
        "openhuman.test_support_list_workspace_files",
        "openhuman.test_support_read_workspace_file",
        "openhuman.test_support_in_flight_chats",
        "openhuman.test_support_wallet_prepared_quotes",
        "openhuman.test_reset",
    ] {
        assert!(
            !catalog_has(&catalog, method),
            "{method} must not be advertised in a product-feature build"
        );

        let response = rpc(&harness.rpc_base, 34_001, method, json!({})).await;
        let message = error_message(&response, method);
        assert!(
            message.contains("unknown method"),
            "{method} must be unroutable, got: {message}"
        );
    }

    // The catalog is not empty — proof the assertions above are about these
    // methods specifically and not about a router that advertises nothing.
    assert!(
        catalog_has(&catalog, "openhuman.ai_list_artifacts"),
        "the schema catalog must still advertise the ungated controllers"
    );

    harness.join.abort();
}

/// The positive side of the same gate: with `e2e-test-support` on, the
/// introspection controllers resolve the live workspace, list and read files
/// inside it, refuse a path that escapes it, and report the two in-process
/// snapshots as empty for a process that has run no chat and prepared no quote.
#[cfg(feature = "e2e-test-support")]
#[tokio::test]
async fn test_support_introspection_reads_the_live_workspace() {
    let _lock = env_lock();
    let harness = setup().await;

    std::fs::create_dir_all(harness.workspace.join("notes")).expect("create notes dir");
    std::fs::write(harness.workspace.join("notes/readme.md"), "hello workspace")
        .expect("write fixture file");

    let root = rpc(
        &harness.rpc_base,
        34_101,
        "openhuman.test_support_workspace_root",
        json!({}),
    )
    .await;
    let root = payload(&root, "test_support_workspace_root");
    assert_eq!(root.get("exists").and_then(Value::as_bool), Some(true));
    let reported = root
        .get("path")
        .and_then(Value::as_str)
        .expect("workspace_root returns a path");
    assert!(
        Path::new(reported).ends_with("workspace"),
        "workspace_root must name the configured workspace, got {reported}"
    );

    let listing = rpc(
        &harness.rpc_base,
        34_102,
        "openhuman.test_support_list_workspace_files",
        json!({ "rel_root": "notes", "max_depth": 1 }),
    )
    .await;
    let listing = payload(&listing, "test_support_list_workspace_files");
    assert_eq!(
        listing.get("truncated").and_then(Value::as_bool),
        Some(false)
    );
    let entries = listing
        .get("entries")
        .and_then(Value::as_array)
        .expect("entries array");
    assert!(
        entries
            .iter()
            .any(|e| e.get("rel_path").and_then(Value::as_str) == Some("readme.md")),
        "the seeded file must be listed: {listing}"
    );

    let read = rpc(
        &harness.rpc_base,
        34_103,
        "openhuman.test_support_read_workspace_file",
        json!({ "rel_path": "notes/readme.md" }),
    )
    .await;
    let read = payload(&read, "test_support_read_workspace_file");
    assert_eq!(
        read.get("content_utf8").and_then(Value::as_str),
        Some("hello workspace")
    );
    assert_eq!(read.get("truncated").and_then(Value::as_bool), Some(false));

    // max_bytes truncates on a byte boundary and says so.
    let clipped = rpc(
        &harness.rpc_base,
        34_104,
        "openhuman.test_support_read_workspace_file",
        json!({ "rel_path": "notes/readme.md", "max_bytes": 5 }),
    )
    .await;
    let clipped = payload(&clipped, "test_support_read_workspace_file clipped");
    assert_eq!(
        clipped.get("truncated").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        clipped.get("returned_bytes").and_then(Value::as_u64),
        Some(5)
    );
    assert_eq!(
        clipped.get("content_utf8").and_then(Value::as_str),
        Some("hello")
    );

    // Failure path: a relative path that resolves outside the workspace root.
    let escape = rpc(
        &harness.rpc_base,
        34_105,
        "openhuman.test_support_read_workspace_file",
        json!({ "rel_path": "../config.toml" }),
    )
    .await;
    assert!(
        error_message(&escape, "read_workspace_file escape").contains("escapes workspace root"),
        "a path resolving outside the workspace must be refused: {escape}"
    );

    // The two in-process snapshots: this process has run no chat turn and
    // prepared no wallet quote, so both must report an empty, well-formed view.
    let chats = rpc(
        &harness.rpc_base,
        34_106,
        "openhuman.test_support_in_flight_chats",
        json!({}),
    )
    .await;
    let chats = payload(&chats, "test_support_in_flight_chats");
    assert_eq!(
        chats.get("entries").and_then(Value::as_array).map(Vec::len),
        Some(0),
        "no chat is in flight in this process: {chats}"
    );

    let quotes = rpc(
        &harness.rpc_base,
        34_107,
        "openhuman.test_support_wallet_prepared_quotes",
        json!({}),
    )
    .await;
    let quotes = payload(&quotes, "test_support_wallet_prepared_quotes");
    assert_eq!(quotes.get("count").and_then(Value::as_u64), Some(0));
    assert_eq!(
        quotes.get("quotes").and_then(Value::as_array).map(Vec::len),
        Some(0),
        "no prepared quote has been minted: {quotes}"
    );

    harness.join.abort();
}
