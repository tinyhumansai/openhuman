//! Skill registry E2E: exercises browse, search, sources, detail, install
//! and uninstall JSON-RPC endpoints against a real core router backed by the
//! tinyskills `SkillRegistry`.
//!
//! Run: `cargo test -p openhuman-cli --test in_process_all skill_registry`
//!
//! The test uses a local fixture catalog and local SKILL.md download URL so CI
//! does not depend on the live Hermes API.

use crate::env_guard::env_lock_async as env_lock_async;
use crate::env_guard::EnvVarGuard;
use crate::rpc_auth::{ensure_rpc_auth, rpc_token};
use openhuman_core::core::auth::CORE_TOKEN_ENV_VAR;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_rpc::server::build_core_http_router;

const FILLER_ENTRIES: usize = 28;

// ── Server helpers ─────────────────────────────────────────────────────────

async fn serve_on_ephemeral(
    app: axum::Router,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    ensure_rpc_auth();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let handle = tokio::spawn(async move { axum::serve(listener, app).await });
    (addr, handle)
}

#[derive(Clone, Default)]
struct FixtureState {
    catalog_hits: Arc<AtomicUsize>,
    offline: Arc<AtomicBool>,
    scan_blocked: Arc<AtomicBool>,
    blocked_variant: Arc<AtomicUsize>,
    document_hits: Arc<AtomicUsize>,
}

fn fixture_catalog() -> Value {
    let mut items = vec![
        json!({
            "name": "git-helper",
            "description": "Automate git status and branch triage.",
            "overview": "Fixture skill for registry tests.",
            "category": "software-development",
            "categoryLabel": "Software Development",
            "source": "fixture",
            "tags": ["git", "workflow"],
            "platforms": ["linux", "macos"],
            "author": "OpenHuman Test",
            "version": "1.0.0",
            "license": "MIT",
            "envVars": [],
            "commands": ["git"],
            "docsPath": "fixture/software-development/software-development-git-helper"
        }),
        json!({
            "name": "notes-helper",
            "description": "Summarize notes.",
            "category": "productivity",
            "source": "fixture",
            "tags": ["notes"],
            "platforms": ["linux", "macos"],
            "envVars": [],
            "commands": []
        }),
    ];
    items.extend((0..FILLER_ENTRIES).map(|i| {
        json!({
            "name": format!("filler-{i:02}"),
            "description": "Padding so the catalog spans more than one page.",
            "category": "productivity",
            "source": "fixture-extra",
            "tags": [],
            "platforms": [],
            "envVars": [],
            "commands": []
        })
    }));
    Value::Array(items)
}

async fn catalog(State(state): State<FixtureState>) -> Response {
    state.catalog_hits.fetch_add(1, Ordering::SeqCst);
    if state.offline.load(Ordering::SeqCst) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    axum::Json(fixture_catalog()).into_response()
}

async fn skill_md(State(state): State<FixtureState>) -> String {
    state.document_hits.fetch_add(1, Ordering::SeqCst);
    if state.scan_blocked.load(Ordering::SeqCst) {
        let variant = state.blocked_variant.load(Ordering::SeqCst);
        return GIT_HELPER_SKILL_MD.replace(
            "report the result",
            &format!("report\u{200b} the result ({variant})"),
        );
    }
    GIT_HELPER_SKILL_MD.to_owned()
}

const GIT_HELPER_SKILL_MD: &str = r#"---
name: git-helper
description: Automate git status and branch triage.
version: 1.0.0
author: OpenHuman Test
license: MIT
metadata:
  id: git-helper
  hermes:
    tags: [git, workflow]
---

# Git Helper

## When to Use
Use when git state needs summarizing.

## Procedure
Run `git status --short` and report the result.
"#;

async fn serve_fixture_catalog() -> (
    SocketAddr,
    FixtureState,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    let state = FixtureState::default();
    let app = Router::new()
        .route("/skills.json", get(catalog))
        .route("/skills/git-helper/SKILL.md", get(skill_md))
        .with_state(state.clone());
    let (addr, join) = serve_on_ephemeral(app).await;
    (addr, state, join)
}

// ── JSON-RPC helpers ───────────────────────────────────────────────────────

async fn post_json_rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("build reqwest client");
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    let url = format!("{}/rpc", rpc_base.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {}", rpc_token()))
        .json(&body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {url}: {e}"));
    assert!(
        resp.status().is_success(),
        "HTTP error {} for {method}",
        resp.status()
    );
    resp.json::<Value>()
        .await
        .unwrap_or_else(|e| panic!("parse json for {method}: {e}"))
}

fn assert_no_jsonrpc_error<'a>(v: &'a Value, context: &str) -> &'a Value {
    if let Some(err) = v.get("error") {
        panic!("{context}: JSON-RPC error: {err}");
    }
    v.get("result")
        .unwrap_or_else(|| panic!("{context}: missing `result` field: {v}"))
}

fn jsonrpc_error_message(v: &Value, context: &str) -> String {
    let error = v
        .get("error")
        .unwrap_or_else(|| panic!("{context}: expected a JSON-RPC error, got {v}"));
    error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn entry_names(result: &Value) -> Vec<String> {
    result
        .get("entries")
        .and_then(Value::as_array)
        .expect("result must contain an `entries` array")
        .iter()
        .filter_map(|e| e.get("name").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

fn write_core_config(home: &Path) {
    let openhuman_home = home.join(".openhuman");
    let config = r#"api_url = "http://127.0.0.1:9"
default_model = "skill-e2e-model"

[secrets]
encrypt = false
"#;
    std::fs::create_dir_all(&openhuman_home).expect("create .openhuman dir");
    std::fs::write(openhuman_home.join("config.toml"), config).expect("write config.toml");
    let user_cfg_dir = openhuman_home.join("users").join("local");
    std::fs::create_dir_all(&user_cfg_dir).expect("create users/local dir");
    std::fs::write(user_cfg_dir.join("config.toml"), config)
        .expect("write users/local/config.toml");
}

struct Stack {
    rpc_base: String,
    fixture_base: String,
    fixture: FixtureState,
    _guards: Vec<EnvVarGuard>,
    joins: Vec<tokio::task::JoinHandle<Result<(), std::io::Error>>>,
}

impl Drop for Stack {
    fn drop(&mut self) {
        for join in &self.joins {
            join.abort();
        }
    }
}

async fn boot(home: &Path) -> Stack {
    write_core_config(home);
    let (fixture_addr, fixture, fixture_join) = serve_fixture_catalog().await;
    let fixture_base = format!("http://{fixture_addr}");
    let guards = vec![
        EnvVarGuard::set_to_path("HOME", home),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::set(CORE_TOKEN_ENV_VAR, rpc_token()),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::unset("OPENHUMAN_SKILL_REGISTRY_CACHE_DIR"),
        EnvVarGuard::set(
            "OPENHUMAN_SKILL_REGISTRY_CATALOG_URL",
            format!("{fixture_base}/skills.json"),
        ),
        EnvVarGuard::set(
            "OPENHUMAN_SKILL_REGISTRY_DOWNLOAD_BASE_URL",
            format!("{fixture_base}/skills"),
        ),
        EnvVarGuard::set("OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP", "1"),
    ];
    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    Stack {
        rpc_base: format!("http://{rpc_addr}"),
        fixture_base,
        fixture,
        _guards: guards,
        joins: vec![rpc_join, fixture_join],
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

/// End-to-end coverage for the `openhuman.skill_registry_*` endpoints:
/// concurrent cold reads share one upstream fetch, paging, multi-source
/// filtering, search, detail, schemas, install (happy, duplicate, unknown id)
/// and uninstall.
#[tokio::test]
async fn skill_registry_e2e_sources_browse_search_install() {
    let _env_lock = env_lock_async().await;
    let tmp = tempdir().expect("create tempdir");
    let home = tmp.path();
    let stack = boot(home).await;
    let rpc_base = stack.rpc_base.as_str();

    // ── Concurrent cold browses: one upstream fetch ──────────────────────
    let mut reads = Vec::new();
    for id in 0..5 {
        let rpc_base = rpc_base.to_owned();
        reads.push(tokio::spawn(async move {
            post_json_rpc(
                &rpc_base,
                9100 + id,
                "openhuman.skill_registry_browse",
                json!({ "page": 1, "page_size": 25 }),
            )
            .await
        }));
    }
    for read in reads {
        let response = read.await.expect("browse task");
        let result = assert_no_jsonrpc_error(&response, "concurrent browse");
        assert_eq!(result["total"], 2 + FILLER_ENTRIES as u64);
    }
    assert_eq!(
        stack.fixture.catalog_hits.load(Ordering::SeqCst),
        1,
        "concurrent cold reads must share one catalog fetch"
    );

    // ── sources ──────────────────────────────────────────────────────────
    let sources_resp = post_json_rpc(
        rpc_base,
        9001,
        "openhuman.skill_registry_sources",
        json!({}),
    )
    .await;
    let sources_result = assert_no_jsonrpc_error(&sources_resp, "skill_registry_sources");
    let sources: Vec<&str> = sources_result["sources"]
        .as_array()
        .expect("sources array")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(sources, ["fixture-extra", "fixture"], "most entries first");

    // ── browse: pages 1 and 2 ────────────────────────────────────────────
    let page_one = post_json_rpc(
        rpc_base,
        9002,
        "openhuman.skill_registry_browse",
        json!({ "page": 1, "page_size": 25 }),
    )
    .await;
    let page_one = assert_no_jsonrpc_error(&page_one, "browse page 1");
    assert_eq!(page_one["page"], 1);
    assert_eq!(page_one["total_pages"], 2);
    assert_eq!(page_one["freshness"], "live");
    assert!(page_one["last_error"].is_null());
    assert_eq!(entry_names(page_one).len(), 25);
    for entry in page_one["entries"].as_array().unwrap() {
        for field in [
            "id",
            "name",
            "description",
            "download_url",
            "source_url",
            "source",
            "category",
            "registry",
            "installable",
        ] {
            assert!(entry.get(field).is_some(), "entry missing '{field}': {entry}");
        }
    }

    let page_two = post_json_rpc(
        rpc_base,
        9003,
        "openhuman.skill_registry_browse",
        json!({ "page": 2, "page_size": 25 }),
    )
    .await;
    let page_two = assert_no_jsonrpc_error(&page_two, "browse page 2");
    assert_eq!(page_two["page"], 2);
    assert_eq!(entry_names(page_two).len(), 2 + FILLER_ENTRIES - 25);

    // ── browse: unpaged legacy read returns everything ───────────────────
    let unpaged = post_json_rpc(
        rpc_base,
        9004,
        "openhuman.skill_registry_browse",
        json!({ "force_refresh": true }),
    )
    .await;
    let unpaged = assert_no_jsonrpc_error(&unpaged, "browse unpaged");
    assert_eq!(entry_names(unpaged).len(), 2 + FILLER_ENTRIES);

    // ── search with sources[] ────────────────────────────────────────────
    let filtered = post_json_rpc(
        rpc_base,
        9005,
        "openhuman.skill_registry_search",
        json!({ "query": "", "sources": ["fixture"], "page": 1 }),
    )
    .await;
    let filtered = assert_no_jsonrpc_error(&filtered, "search sources[]");
    let mut names = entry_names(filtered);
    names.sort();
    assert_eq!(names, ["git-helper", "notes-helper"]);

    let search = post_json_rpc(
        rpc_base,
        9006,
        "openhuman.skill_registry_search",
        json!({ "query": "git", "page": 1 }),
    )
    .await;
    let search = assert_no_jsonrpc_error(&search, "search git");
    assert_eq!(entry_names(search), ["git-helper"]);
    let git_helper = &search["entries"][0];
    assert_eq!(
        git_helper["download_url"],
        format!("{}/skills/git-helper/SKILL.md", stack.fixture_base)
    );
    let entry_id = git_helper["id"].as_str().expect("entry id").to_owned();

    // ── detail ───────────────────────────────────────────────────────────
    let detail = post_json_rpc(
        rpc_base,
        9007,
        "openhuman.skill_registry_detail",
        json!({ "entry_id": entry_id }),
    )
    .await;
    let detail = assert_no_jsonrpc_error(&detail, "detail");
    assert_eq!(detail["overview"], "Fixture skill for registry tests.");
    assert_eq!(detail["license"], "MIT");

    // ── schemas ──────────────────────────────────────────────────────────
    let schemas_resp = post_json_rpc(
        rpc_base,
        9008,
        "openhuman.skill_registry_schemas",
        json!({}),
    )
    .await;
    let schemas_result = assert_no_jsonrpc_error(&schemas_resp, "skill_registry_schemas");
    let functions: Vec<&str> = schemas_result["schemas"]
        .as_array()
        .expect("schemas array")
        .iter()
        .filter_map(|schema| schema.get("function").and_then(Value::as_str))
        .collect();
    for function in ["browse", "search", "detail", "install", "uninstall"] {
        assert!(functions.contains(&function), "{function}: {functions:?}");
    }

    // ── install (happy path) ─────────────────────────────────────────────
    let install_resp = post_json_rpc(
        rpc_base,
        9009,
        "openhuman.skill_registry_install",
        json!({ "entry_id": entry_id }),
    )
    .await;
    let install_result = assert_no_jsonrpc_error(&install_resp, "install (happy)");
    assert!(
        !install_result["url"].as_str().unwrap_or_default().is_empty(),
        "install result `url` must not be empty"
    );
    let install_stdout = install_result["stdout"].as_str().unwrap_or_default();
    assert!(
        install_stdout.contains("Installed to"),
        "install stdout should mention 'Installed to', got: {install_stdout}"
    );
    assert!(install_result.get("stderr").is_some());
    let new_skills = install_result["new_skills"]
        .as_array()
        .expect("new_skills array");
    assert!(
        new_skills.iter().any(|s| s.as_str() == Some(entry_id.as_str())),
        "new_skills must contain '{entry_id}', got: {new_skills:?}"
    );
    let skill_file = home
        .join(".openhuman")
        .join("skills")
        .join(&entry_id)
        .join("SKILL.md");
    assert!(skill_file.exists(), "SKILL.md missing at {}", skill_file.display());

    // ── install (duplicate no-op success) ────────────────────────────────
    let dup_resp = post_json_rpc(
        rpc_base,
        9010,
        "openhuman.skill_registry_install",
        json!({ "entry_id": entry_id }),
    )
    .await;
    let dup_result = assert_no_jsonrpc_error(&dup_resp, "install (duplicate)");
    assert!(dup_result["stdout"]
        .as_str()
        .unwrap_or_default()
        .contains("already installed"));
    assert_eq!(dup_result["new_skills"], json!([]));

    // ── install (unknown id) ─────────────────────────────────────────────
    let missing = post_json_rpc(
        rpc_base,
        9011,
        "openhuman.skill_registry_install",
        json!({ "entry_id": "git-helpr" }),
    )
    .await;
    let message = jsonrpc_error_message(&missing, "install (unknown id)");
    assert!(
        message.contains("SKILL_REGISTRY_NOT_FOUND: "),
        "an unknown id is a typed not-found: {message}"
    );
    assert!(message.contains("git-helper"), "suggests real ids: {message}");

    // ── uninstall ────────────────────────────────────────────────────────
    let uninstall_resp = post_json_rpc(
        rpc_base,
        9012,
        "openhuman.skill_registry_uninstall",
        json!({ "name": entry_id }),
    )
    .await;
    let uninstall_result = assert_no_jsonrpc_error(&uninstall_resp, "skill_registry_uninstall");
    assert_eq!(uninstall_result["name"], json!(entry_id));
    assert!(!skill_file.exists(), "SKILL.md should be removed after uninstall");
}

/// A registry that has fetched once keeps serving its catalog when the
/// upstream goes down, flagging the failure; one that never fetched returns
/// the typed error.
#[tokio::test]
async fn skill_registry_e2e_serves_the_held_catalog_when_the_upstream_fails() {
    let _env_lock = env_lock_async().await;
    let tmp = tempdir().expect("create tempdir");
    let stack = boot(tmp.path()).await;
    let rpc_base = stack.rpc_base.as_str();

    let live = post_json_rpc(
        rpc_base,
        9201,
        "openhuman.skill_registry_browse",
        json!({ "page": 1 }),
    )
    .await;
    let live = assert_no_jsonrpc_error(&live, "browse live");
    assert_eq!(live["freshness"], "live");

    stack.fixture.offline.store(true, Ordering::SeqCst);
    let held = post_json_rpc(
        rpc_base,
        9202,
        "openhuman.skill_registry_browse",
        json!({ "page": 1, "force_refresh": true }),
    )
    .await;
    let held = assert_no_jsonrpc_error(&held, "browse after the upstream failed");
    assert_eq!(held["total"], 2 + FILLER_ENTRIES as u64);
    assert_eq!(held["last_error"]["kind"], "unavailable");

    let cold_tmp = tempdir().expect("create tempdir");
    let cold = boot(cold_tmp.path()).await;
    cold.fixture.offline.store(true, Ordering::SeqCst);
    let failed = post_json_rpc(
        &cold.rpc_base,
        9203,
        "openhuman.skill_registry_browse",
        json!({ "page": 1 }),
    )
    .await;
    let message = jsonrpc_error_message(&failed, "browse with nothing held");
    assert!(
        message.contains("SKILL_REGISTRY_UNAVAILABLE: "),
        "a cold registry with an unreachable upstream is a typed error: {message}"
    );
}

/// A `SKILL.md` the supply-chain scan blocks is fetched twice, refused with
/// `status: "scan_blocked"` and its findings, and installed only when the
/// call carries that document's digest as `acknowledged_digest`; a digest for
/// a document that has since changed is refused afresh. The pasted-URL install
/// takes the
/// same gate.
#[tokio::test]
async fn skill_registry_e2e_refuses_a_scan_blocked_install_until_acknowledged() {
    let _env_lock = env_lock_async().await;
    let tmp = tempdir().expect("create tempdir");
    let home = tmp.path();
    let stack = boot(home).await;
    let rpc_base = stack.rpc_base.as_str();
    stack.fixture.scan_blocked.store(true, Ordering::SeqCst);
    let skill_file = home
        .join(".openhuman")
        .join("skills")
        .join("git-helper")
        .join("SKILL.md");

    let blocked = post_json_rpc(
        rpc_base,
        9301,
        "openhuman.skill_registry_install",
        json!({ "entry_id": "git-helper" }),
    )
    .await;
    let blocked = assert_no_jsonrpc_error(&blocked, "install (scan blocked)");
    assert_eq!(blocked["status"], "scan_blocked", "{blocked}");
    assert_eq!(blocked["target"], "git-helper");
    assert_eq!(blocked["findings"][0]["check"], "invisible_code_points");
    assert_eq!(blocked["findings"][0]["verdict"], "block");
    assert!(blocked["message"]
        .as_str()
        .unwrap_or_default()
        .contains("not installed"));
    assert_eq!(
        stack.fixture.document_hits.load(Ordering::SeqCst),
        2,
        "a blocking scan is fetched and scanned once more"
    );
    assert!(!skill_file.exists(), "a blocked document is not written");

    let digest = blocked["digest"].as_str().expect("digest").to_owned();
    assert!(!digest.is_empty());

    let wrong = post_json_rpc(
        rpc_base,
        9302,
        "openhuman.skill_registry_install",
        json!({ "entry_id": "git-helper", "acknowledged_digest": "not-the-digest" }),
    )
    .await;
    let wrong = assert_no_jsonrpc_error(&wrong, "install (wrong digest)");
    assert_eq!(wrong["status"], "scan_blocked");
    assert_eq!(wrong["digest"], json!(digest));
    assert!(!skill_file.exists());

    stack.fixture.blocked_variant.store(1, Ordering::SeqCst);
    let changed = post_json_rpc(
        rpc_base,
        9303,
        "openhuman.skill_registry_install",
        json!({ "entry_id": "git-helper", "acknowledged_digest": digest }),
    )
    .await;
    let changed = assert_no_jsonrpc_error(&changed, "install (document changed)");
    assert_eq!(changed["status"], "scan_blocked", "{changed}");
    let fresh_digest = changed["digest"].as_str().expect("digest").to_owned();
    assert_ne!(fresh_digest, digest, "the refusal names the changed document");
    assert!(!skill_file.exists(), "a stale acknowledgement installs nothing");

    let acknowledged = post_json_rpc(
        rpc_base,
        9304,
        "openhuman.skill_registry_install",
        json!({ "entry_id": "git-helper", "acknowledged_digest": fresh_digest }),
    )
    .await;
    let acknowledged = assert_no_jsonrpc_error(&acknowledged, "install (acknowledged)");
    assert_eq!(acknowledged["status"], "installed", "{acknowledged}");
    assert_eq!(acknowledged["new_skills"], json!(["git-helper"]));
    assert!(skill_file.exists());

    let uninstall = post_json_rpc(
        rpc_base,
        9305,
        "openhuman.skill_registry_uninstall",
        json!({ "name": "git-helper" }),
    )
    .await;
    assert_no_jsonrpc_error(&uninstall, "uninstall");

    let url = format!("{}/skills/git-helper/SKILL.md", stack.fixture_base);
    let url_blocked = post_json_rpc(
        rpc_base,
        9306,
        "openhuman.skills_install_from_url",
        json!({ "url": url }),
    )
    .await;
    let url_blocked = assert_no_jsonrpc_error(&url_blocked, "install_from_url (scan blocked)");
    assert_eq!(url_blocked["status"], "scan_blocked", "{url_blocked}");
    assert!(!skill_file.exists());
    let url_digest = url_blocked["digest"].as_str().expect("digest").to_owned();

    let url_acknowledged = post_json_rpc(
        rpc_base,
        9307,
        "openhuman.skills_install_from_url",
        json!({ "url": url, "acknowledged_digest": url_digest }),
    )
    .await;
    let url_acknowledged =
        assert_no_jsonrpc_error(&url_acknowledged, "install_from_url (acknowledged)");
    assert_eq!(url_acknowledged["status"], "installed", "{url_acknowledged}");
    assert_eq!(url_acknowledged["new_workflows"], json!(["git-helper"]));
    assert!(skill_file.exists());
}
