//! JSON-RPC E2E coverage for the `learning` namespace — the eleven controllers
//! behind the user-profile facet cache.
//!
//! The arc these cases walk is the production one: real observations are pushed
//! into `learning::candidate::global()` (the same buffer
//! `learning::extract::heuristics`, `extract::signature` and
//! `learning::reflection` push into), `learning.rebuild_cache` runs the
//! stability detector over them, and every read/mutate controller is then driven
//! over the actual axum JSON-RPC router. Nothing is written to the facet store
//! behind the RPC surface's back.
//!
//! This file is a **module** of the aggregated `raw_coverage_all` target, not a
//! target of its own — `build.rs` globs `tests/raw_coverage/` and generates the
//! `mod` list. Run with:
//!   `~/tinyhuman/ci-slot.sh cargo test --test raw_coverage_all \
//!      --features "$(bash scripts/ci/product-features.sh)" -- learning_facets`
//!
//! ## Two hazards this file is written around
//!
//! 1. **One process, ~77 suites.** The env lock is the crate-wide
//!    `SHARED_ENV_LOCK`; a lock private to this module would isolate nothing.
//!    The RPC bearer is read back from `core::auth::get_rpc_token()` rather than
//!    assumed, because that token is a process-global `OnceLock` and a sibling
//!    suite may have seeded it first.
//!
//! 2. **The facet store is shared.** The memory module host captures one
//!    workspace per process, so these cases key everything on a unique
//!    `E2E_KEY_*` slug and assert on *their own* rows — never on a global count.
//!    `cache_stats` is asserted as "at least mine", and `reset_cache` is checked
//!    by what happened to this suite's two facets, not by the totals.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::http::header::AUTHORIZATION;
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::TempDir;

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;
use openhuman_core::openhuman::agent::learning::candidate::{
    self, CueFamily, EvidenceRef, FacetClass, LearningCandidate,
};
use openhuman_core::openhuman::config::Config;

/// Preferred bearer. Only the real one if this module wins the process-global
/// `OnceLock` race — send [`rpc_bearer`], never this.
const PREFERRED_RPC_TOKEN: &str = "learning-facets-e2e-token";

/// Facet keys this suite owns. Distinctive enough that a sibling suite (or a
/// leftover row in the shared store) can never be mistaken for one of them.
const PINNED_KEY: &str = "e2e_learning_pinned_slug";
const VOLATILE_KEY: &str = "e2e_learning_volatile_slug";

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
            .name("learning-facets-e2e-seams".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                let config = Arc::new(shared_config_at(learning_workspace()));
                #[cfg(feature = "modules")]
                openhuman_core::openhuman::modules::memory::set_modules_policy(config);
            })
            .expect("spawn learning facets seam installer")
            .join()
            .expect("learning facets seam installer panicked");
    });
}

/// The one memory workspace every case in this module shares.
///
/// The module host captures a workspace **once per process** — the loaded
/// artifact takes its `workspace_dir` at load time and later policy calls are
/// ignored — while a per-case `TempDir` would be deleted the moment its case
/// returned, leaving every later read answering from a dead store. So this is
/// one leaked directory, published once, and every case points both its config
/// and `OPENHUMAN_WORKSPACE` at it. The path ends in `workspace` so
/// `resolve_config_dir_for_workspace` treats it as the workspace itself rather
/// than appending another segment.
fn learning_workspace() -> &'static Path {
    static WORKSPACE: OnceLock<PathBuf> = OnceLock::new();
    WORKSPACE.get_or_init(|| {
        let dir = TempDir::new().expect("learning workspace tempdir");
        let path = dir.path().join("workspace");
        std::fs::create_dir_all(&path).expect("create learning workspace");
        // Leaked on purpose: the module keeps this path for the process lifetime.
        std::mem::forget(dir);
        path
    })
}

/// A config whose workspace **and** config path are the shared ones.
///
/// `config_path` matters as much as `workspace_dir`: `Config::default()` names
/// the developer's real `~/.openhuman/config.toml`, and pointing it beside the
/// shared workspace is where `Config::load_or_init` resolves it from
/// `OPENHUMAN_WORKSPACE` too — so env-driven and config-driven paths agree.
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
"#;
    std::fs::write(config_path, cfg).expect("write config.toml");
    let _: Config = toml::from_str(cfg).expect("test config must match schema");
}

// ── Harness ───────────────────────────────────────────────────────────────

fn ensure_rpc_auth() {
    AUTH_INIT.get_or_init(|| {
        std::env::set_var(CORE_TOKEN_ENV_VAR, PREFERRED_RPC_TOKEN);
        let token_dir = std::env::temp_dir().join("openhuman-learning-facets-e2e-auth");
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

async fn setup() -> Harness {
    ensure_memory_seams();
    let workspace = learning_workspace().to_path_buf();
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
        workspace,
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

// ── Candidate seeding ─────────────────────────────────────────────────────

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Push `repeats` observations of one (class, key, value) into the global
/// candidate buffer — the same buffer the production extractors push into.
///
/// `Explicit` and a recent `observed_at` are chosen deliberately: the stability
/// formula is `Σ(weight × exp(-Δt/half_life) × ln(1 + evidence_count))` with a
/// ×2.0 multiplier for an explicit cue, so several fresh explicit observations
/// clear `TAU_PROMOTE` (1.5) and enter as `Active`. Anything weaker would enter
/// as `Provisional` or `Candidate` and the assertions below would be about the
/// thresholds rather than about the controllers.
fn seed_candidate(class: FacetClass, key: &str, value: &str, repeats: i64) {
    let buffer = candidate::global();
    for i in 0..repeats {
        buffer.push(LearningCandidate {
            class,
            key: key.to_string(),
            value: value.to_string(),
            cue_family: CueFamily::Explicit,
            evidence: EvidenceRef::Episodic { episodic_id: i + 1 },
            initial_confidence: 0.9,
            observed_at: now_secs(),
        });
    }
}

/// Find one facet by its full `class/key` in a `list_facets` / `cache_stats`
/// style array.
fn find_facet<'a>(facets: &'a [Value], full_key: &str) -> Option<&'a Value> {
    facets
        .iter()
        .find(|f| f.get("key").and_then(Value::as_str) == Some(full_key))
}

fn facets_of(payload: &Value, context: &str) -> Vec<Value> {
    payload
        .get("facets")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{context}: expected a facets array: {payload}"))
        .clone()
}

// ── Tests ─────────────────────────────────────────────────────────────────

/// The full facet lifecycle over RPC: rebuild from observations → list → get →
/// update → pin → unpin → cache_stats → forget → reset.
///
/// One case rather than nine, because these controllers share one durable row
/// and splitting them would either re-seed nine times or make each case depend
/// on the previous one's leftovers — the state-leak the wave brief forbids.
#[tokio::test]
async fn learning_facet_lifecycle_from_rebuild_to_reset() {
    let _lock = env_lock();
    let harness = setup().await;

    // Drain anything a sibling suite left, so `rebuild` scores only these.
    let _ = candidate::global().drain();
    seed_candidate(FacetClass::Style, PINNED_KEY, "terse", 6);
    seed_candidate(FacetClass::Tooling, VOLATILE_KEY, "pnpm", 6);

    let pinned_full = format!("style/{PINNED_KEY}");
    let volatile_full = format!("tooling/{VOLATILE_KEY}");

    // ── rebuild_cache: the detector promotes both observations ──
    let rebuilt = rpc(
        &harness.rpc_base,
        40_001,
        "openhuman.learning_rebuild_cache",
        json!({}),
    )
    .await;
    let rebuilt = payload(&rebuilt, "learning_rebuild_cache");
    for field in ["added", "evicted", "kept", "total_size"] {
        assert!(
            rebuilt.get(field).and_then(Value::as_u64).is_some(),
            "rebuild_cache must report {field}: {rebuilt}"
        );
    }
    assert!(
        rebuilt.get("added").and_then(Value::as_u64).unwrap_or(0) >= 2,
        "both seeded observations must be added as facets: {rebuilt}"
    );
    assert!(
        rebuilt
            .get("total_size")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            >= 2,
        "the cache must hold at least the two seeded facets: {rebuilt}"
    );

    // ── list_facets: both are visible, with the value they were seeded with ──
    let listed = rpc(
        &harness.rpc_base,
        40_002,
        "openhuman.learning_list_facets",
        json!({}),
    )
    .await;
    let listed = payload(&listed, "learning_list_facets");
    let facets = facets_of(&listed, "learning_list_facets");
    assert_eq!(
        listed.get("count").and_then(Value::as_u64),
        Some(facets.len() as u64),
        "count must agree with the array it summarises: {listed}"
    );
    let seeded = find_facet(&facets, &pinned_full)
        .unwrap_or_else(|| panic!("the seeded style facet must be listed: {listed}"));
    assert_eq!(seeded.get("value").and_then(Value::as_str), Some("terse"));
    assert_eq!(
        seeded.get("state").and_then(Value::as_str),
        Some("active"),
        "six fresh explicit cues clear TAU_PROMOTE: {seeded}"
    );
    assert_eq!(seeded.get("class").and_then(Value::as_str), Some("style"));
    assert!(
        seeded.get("stability").and_then(Value::as_f64).unwrap_or(0.0) > 0.0,
        "a promoted facet carries a positive stability: {seeded}"
    );
    assert!(
        find_facet(&facets, &volatile_full).is_some(),
        "the seeded tooling facet must be listed too: {listed}"
    );

    // ── list_facets(class=…): the filter narrows to one class ──
    let styled = rpc(
        &harness.rpc_base,
        40_003,
        "openhuman.learning_list_facets",
        json!({ "class": "style" }),
    )
    .await;
    let styled = payload(&styled, "learning_list_facets style");
    let styled_facets = facets_of(&styled, "learning_list_facets style");
    assert!(
        find_facet(&styled_facets, &pinned_full).is_some(),
        "the style facet survives its own class filter: {styled}"
    );
    assert!(
        find_facet(&styled_facets, &volatile_full).is_none(),
        "a tooling facet must not appear under class=style: {styled}"
    );

    // ── get_facet: found, with the same value list reported ──
    let got = rpc(
        &harness.rpc_base,
        40_004,
        "openhuman.learning_get_facet",
        json!({ "class": "style", "key": PINNED_KEY }),
    )
    .await;
    let got = payload(&got, "learning_get_facet");
    assert_eq!(got.get("found").and_then(Value::as_bool), Some(true));
    assert_eq!(
        got.get("facet")
            .and_then(|f| f.get("value"))
            .and_then(Value::as_str),
        Some("terse")
    );

    // ── update_facet: writes the new value AND pins, so it survives rebuilds ──
    let updated = rpc(
        &harness.rpc_base,
        40_005,
        "openhuman.learning_update_facet",
        json!({ "class": "style", "key": PINNED_KEY, "value": "exhaustive" }),
    )
    .await;
    let updated = payload(&updated, "learning_update_facet");
    let updated_facet = updated
        .get("facet")
        .unwrap_or_else(|| panic!("update_facet returns the facet: {updated}"));
    assert_eq!(
        updated_facet.get("value").and_then(Value::as_str),
        Some("exhaustive"),
        "the new value must be persisted, not echoed: {updated}"
    );
    assert_eq!(
        updated_facet.get("user_state").and_then(Value::as_str),
        Some("pinned"),
        "an edit pins the facet so a later rebuild cannot revert it: {updated}"
    );

    // Re-read through a different controller to prove it was written, not echoed.
    let reread = rpc(
        &harness.rpc_base,
        40_006,
        "openhuman.learning_get_facet",
        json!({ "class": "style", "key": PINNED_KEY }),
    )
    .await;
    assert_eq!(
        payload(&reread, "learning_get_facet after update")
            .get("facet")
            .and_then(|f| f.get("value"))
            .and_then(Value::as_str),
        Some("exhaustive")
    );

    // ── unpin then pin: user_state round-trips on the volatile facet ──
    let pinned = rpc(
        &harness.rpc_base,
        40_007,
        "openhuman.learning_pin_facet",
        json!({ "class": "tooling", "key": VOLATILE_KEY }),
    )
    .await;
    assert_eq!(
        payload(&pinned, "learning_pin_facet")
            .get("facet")
            .and_then(|f| f.get("user_state"))
            .and_then(Value::as_str),
        Some("pinned")
    );

    let unpinned = rpc(
        &harness.rpc_base,
        40_008,
        "openhuman.learning_unpin_facet",
        json!({ "class": "tooling", "key": VOLATILE_KEY }),
    )
    .await;
    assert_eq!(
        payload(&unpinned, "learning_unpin_facet")
            .get("facet")
            .and_then(|f| f.get("user_state"))
            .and_then(Value::as_str),
        Some("auto"),
        "unpin returns the facet to automatic lifecycle management"
    );

    // ── cache_stats: the aggregate sees this suite's rows ──
    let stats = rpc(
        &harness.rpc_base,
        40_009,
        "openhuman.learning_cache_stats",
        json!({}),
    )
    .await;
    let stats = payload(&stats, "learning_cache_stats");
    let total = stats
        .get("total")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("cache_stats must report a total: {stats}"));
    let by_state: u64 = ["active", "provisional", "candidate", "dropped"]
        .iter()
        .map(|k| stats.get(*k).and_then(Value::as_u64).unwrap_or(0))
        .sum();
    assert_eq!(
        total, by_state,
        "the per-state counts must partition the total: {stats}"
    );
    assert!(total >= 2, "both seeded facets are counted: {stats}");
    assert!(
        stats
            .get("by_class")
            .and_then(|c| c.get("style"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
            >= 1,
        "by_class must attribute the style facet: {stats}"
    );

    // ── forget_facet: drops the volatile one out of the visible list ──
    let forgotten = rpc(
        &harness.rpc_base,
        40_010,
        "openhuman.learning_forget_facet",
        json!({ "class": "tooling", "key": VOLATILE_KEY }),
    )
    .await;
    let forgotten = payload(&forgotten, "learning_forget_facet");
    let forgotten_facet = forgotten
        .get("facet")
        .unwrap_or_else(|| panic!("forget_facet returns the facet: {forgotten}"));
    assert_eq!(
        forgotten_facet.get("state").and_then(Value::as_str),
        Some("dropped")
    );
    assert_eq!(
        forgotten_facet.get("user_state").and_then(Value::as_str),
        Some("forgotten"),
        "forget must record the user's intent so a rebuild cannot resurface it"
    );

    let after_forget = rpc(
        &harness.rpc_base,
        40_011,
        "openhuman.learning_list_facets",
        json!({}),
    )
    .await;
    let after_forget = facets_of(
        &payload(&after_forget, "learning_list_facets after forget"),
        "list after forget",
    );
    assert!(
        find_facet(&after_forget, &volatile_full).is_none(),
        "a dropped facet must leave the user-visible list"
    );
    assert!(
        find_facet(&after_forget, &pinned_full).is_some(),
        "forgetting one facet must not touch the other"
    );

    // ── reset_cache: clears non-pinned rows, preserves pinned ones ──
    let reset = rpc(
        &harness.rpc_base,
        40_012,
        "openhuman.learning_reset_cache",
        json!({}),
    )
    .await;
    let reset = payload(&reset, "learning_reset_cache");
    assert!(
        reset.get("deleted").and_then(Value::as_u64).is_some(),
        "reset_cache must report a delete count: {reset}"
    );
    assert!(
        reset
            .get("pinned_preserved")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            >= 1,
        "the pinned facet must be counted as preserved: {reset}"
    );

    let survivor = rpc(
        &harness.rpc_base,
        40_013,
        "openhuman.learning_get_facet",
        json!({ "class": "style", "key": PINNED_KEY }),
    )
    .await;
    let survivor = payload(&survivor, "learning_get_facet after reset");
    assert_eq!(
        survivor.get("found").and_then(Value::as_bool),
        Some(true),
        "reset_cache must not delete a pinned facet: {survivor}"
    );
    assert_eq!(
        survivor
            .get("facet")
            .and_then(|f| f.get("value"))
            .and_then(Value::as_str),
        Some("exhaustive"),
        "and it must survive with the value the user set"
    );

    harness.join.abort();
}

/// Failure paths across the facet controllers: missing params are refused by
/// name, and a key with no row is refused rather than silently created.
///
/// The wording asserted here is **`core::all::validate_params`'**, not the
/// handlers'. Every dispatch is schema-validated for required-presence, unknown
/// params and declared types *before* the handler body runs
/// (`src/core/all.rs:1334`), so the handlers' own
/// ``missing required `class` `` strings are unreachable over RPC. Asserting the
/// handler's wording would pass only if that uniform gate were removed.
#[tokio::test]
async fn learning_facet_controllers_refuse_bad_input_and_absent_keys() {
    let _lock = env_lock();
    let harness = setup().await;

    // `get_facet` needs both halves of the key.
    let no_class = rpc(
        &harness.rpc_base,
        40_101,
        "openhuman.learning_get_facet",
        json!({ "key": "verbosity" }),
    )
    .await;
    assert!(
        error_message(&no_class, "get_facet without class")
            .contains("missing required param 'class'"),
        "the refusal must name the param: {no_class}"
    );

    let no_key = rpc(
        &harness.rpc_base,
        40_102,
        "openhuman.learning_get_facet",
        json!({ "class": "style" }),
    )
    .await;
    assert!(
        error_message(&no_key, "get_facet without key").contains("missing required param 'key'"),
        "the refusal must name the param: {no_key}"
    );

    // `update_facet` needs a value on top of the key.
    let no_value = rpc(
        &harness.rpc_base,
        40_103,
        "openhuman.learning_update_facet",
        json!({ "class": "style", "key": "verbosity" }),
    )
    .await;
    assert!(
        error_message(&no_value, "update_facet without value")
            .contains("missing required param 'value'"),
        "the refusal must name the param: {no_value}"
    );

    // A key with no row: `get` reports absence, the mutators refuse.
    let absent = "e2e_learning_key_that_was_never_observed";
    let missing = rpc(
        &harness.rpc_base,
        40_104,
        "openhuman.learning_get_facet",
        json!({ "class": "style", "key": absent }),
    )
    .await;
    let missing = payload(&missing, "get_facet absent");
    assert_eq!(
        missing.get("found").and_then(Value::as_bool),
        Some(false),
        "absence is a clean `found: false` on the read path: {missing}"
    );
    assert_eq!(
        missing.get("facet"),
        Some(&Value::Null),
        "and it carries no facet: {missing}"
    );

    for (id, method, context) in [
        (40_105_i64, "openhuman.learning_update_facet", "update"),
        (40_106, "openhuman.learning_pin_facet", "pin"),
        (40_107, "openhuman.learning_unpin_facet", "unpin"),
    ] {
        let mut params = json!({ "class": "style", "key": absent });
        if method.ends_with("update_facet") {
            params
                .as_object_mut()
                .expect("params object")
                .insert("value".into(), json!("anything"));
        }
        let response = rpc(&harness.rpc_base, id, method, params).await;
        let message = error_message(&response, context);
        assert!(
            message.contains("facet not found") && message.contains(absent),
            "{context} on an absent key must refuse and name it, got: {message}"
        );
    }

    // `forget_facet` is the odd one out and this pins the behaviour as it is,
    // not as it should be: it answers `Ok` with a null facet for a key that was
    // never observed, while its three sibling mutators refuse. The log line it
    // emits says `state=dropped user_state=forgotten` for a row that does not
    // exist, so a caller reading the log cannot tell a real forget from a typo.
    // See `~/tinyhuman/bugs/e2e-wave-learning-forget-facet-succeeds-on-absent-key.md`.
    let forgotten_nothing = rpc(
        &harness.rpc_base,
        40_108,
        "openhuman.learning_forget_facet",
        json!({ "class": "style", "key": absent }),
    )
    .await;
    let forgotten_nothing = payload(&forgotten_nothing, "forget_facet absent");
    assert_eq!(
        forgotten_nothing.get("facet"),
        Some(&Value::Null),
        "forget on an absent key reports success with no facet: {forgotten_nothing}"
    );

    harness.join.abort();
}

/// `learning.save_profile` writes `PROFILE.md` into the active workspace, and
/// refuses a call with no body.
///
/// `summarize` is left at its default `false`: with it on the handler routes the
/// body through the LLM compressor, which needs a provider this hermetic suite
/// deliberately does not have.
#[tokio::test]
async fn learning_save_profile_writes_the_workspace_file() {
    let _lock = env_lock();
    let harness = setup().await;

    let body = "# E2E Profile\n\n- Prefers terse answers\n- Uses pnpm\n";
    let saved = rpc(
        &harness.rpc_base,
        40_201,
        "openhuman.learning_save_profile",
        json!({ "markdown": body }),
    )
    .await;
    let saved = payload(&saved, "learning_save_profile");
    assert_eq!(
        saved.get("bytes").and_then(Value::as_u64),
        Some(body.len() as u64),
        "the reported byte count is the body's, not a rounded estimate: {saved}"
    );
    let reported = saved
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("save_profile must report the path it wrote: {saved}"));

    let expected = harness.workspace.join("PROFILE.md");
    assert_eq!(
        std::fs::read_to_string(&expected).expect("PROFILE.md must exist on disk"),
        body,
        "the file content must be the markdown handed in"
    );
    assert!(
        reported.ends_with("PROFILE.md"),
        "the reported path must name the file: {reported}"
    );

    // A second save replaces rather than appends.
    let shorter = "# Replaced\n";
    let replaced = rpc(
        &harness.rpc_base,
        40_202,
        "openhuman.learning_save_profile",
        json!({ "markdown": shorter }),
    )
    .await;
    assert_eq!(
        payload(&replaced, "learning_save_profile replace")
            .get("bytes")
            .and_then(Value::as_u64),
        Some(shorter.len() as u64)
    );
    assert_eq!(
        std::fs::read_to_string(&expected).expect("PROFILE.md must still exist"),
        shorter,
        "save_profile truncates — a stale tail would corrupt the profile"
    );

    // Failure path: no body at all.
    let empty = rpc(
        &harness.rpc_base,
        40_203,
        "openhuman.learning_save_profile",
        json!({}),
    )
    .await;
    assert!(
        error_message(&empty, "save_profile without markdown")
            .contains("missing required param 'markdown'"),
        "the refusal must name the param: {empty}"
    );

    harness.join.abort();
}

/// `learning.linkedin_enrichment` runs a network pipeline (Gmail search → Apify
/// scrape → memory write). With `api_url` pointed at a closed port it must
/// surface a *pipeline* failure — not panic, not hang, and not report success
/// with an empty result.
#[tokio::test]
async fn learning_linkedin_enrichment_fails_loudly_without_a_backend() {
    let _lock = env_lock();
    let harness = setup().await;

    let response = rpc(
        &harness.rpc_base,
        40_301,
        "openhuman.learning_linkedin_enrichment",
        json!({ "profile_url": "https://www.linkedin.com/in/e2e-nobody" }),
    )
    .await;

    // Either shape is a correct contract here, and both are asserted rather
    // than one being assumed: the pipeline may refuse outright (no backend
    // reachable) or complete with an empty, honestly-reported log. What it must
    // never do is claim it scraped a profile it could not reach.
    match response.get("error") {
        Some(_) => {
            let message = error_message(&response, "learning_linkedin_enrichment");
            assert!(
                !message.is_empty(),
                "a refusal must carry a message: {response}"
            );
        }
        None => {
            let body = payload(&response, "learning_linkedin_enrichment");
            assert!(
                body.get("log").and_then(Value::as_array).is_some(),
                "the pipeline always reports its stages: {body}"
            );
            assert_eq!(
                body.get("profile_data"),
                Some(&Value::Null),
                "no backend means no scraped profile: {body}"
            );
        }
    }

    harness.join.abort();
}
