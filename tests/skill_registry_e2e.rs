//! Skill registry E2E: exercises browse, search, sources, and install
//! JSON-RPC endpoints against a real core router.
//!
//! Run: `cargo test --test skill_registry_e2e`
//!
//! The test uses a local fixture catalog and local SKILL.md download URL so CI
//! does not depend on the live Hermes API.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::http::header::AUTHORIZATION;
use axum::routing::get;
use axum::Router;
use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;

// ── Constants ──────────────────────────────────────────────────────────────

const TEST_RPC_TOKEN: &str = "skill-registry-e2e-token";

// ── One-time auth init ─────────────────────────────────────────────────────

static SKILL_REGISTRY_AUTH_INIT: OnceLock<()> = OnceLock::new();

fn ensure_test_rpc_auth() {
    SKILL_REGISTRY_AUTH_INIT.get_or_init(|| {
        unsafe { std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN) };
        let token_dir = std::env::temp_dir().join("openhuman-skill-registry-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token for skill_registry_e2e");
    });
}

// ── Env lock (process-global env vars must not race) ──────────────────────

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static KEYRING_INIT: OnceLock<()> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    KEYRING_INIT.get_or_init(|| unsafe {
        std::env::set_var("OPENHUMAN_KEYRING_BACKEND", "file");
    });
    let mutex = ENV_LOCK.get_or_init(|| Mutex::new(()));
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ── EnvVarGuard ───────────────────────────────────────────────────────────

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::set_var(key, path.as_os_str()) };
        Self { key, old }
    }

    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::set_var(key, value) };
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::remove_var(key) };
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(v) => unsafe { std::env::set_var(self.key, v) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

// ── Server helpers ─────────────────────────────────────────────────────────

async fn serve_on_ephemeral(
    app: axum::Router,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    ensure_test_rpc_auth();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let handle = tokio::spawn(async move { axum::serve(listener, app).await });
    (addr, handle)
}

async fn serve_fixture_catalog() -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    async fn catalog() -> axum::Json<Value> {
        axum::Json(json!([
            {
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
            },
            {
                "name": "notes-helper",
                "description": "Summarize notes.",
                "category": "productivity",
                "source": "fixture",
                "tags": ["notes"],
                "platforms": ["linux", "macos"],
                "envVars": [],
                "commands": []
            }
        ]))
    }

    async fn skill_md() -> &'static str {
        r#"---
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
"#
    }

    let app = Router::new()
        .route("/skills.json", get(catalog))
        .route("/skills/git-helper/SKILL.md", get(skill_md));
    serve_on_ephemeral(app).await
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
        .header(AUTHORIZATION, format!("Bearer {TEST_RPC_TOKEN}"))
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

// ── Test ───────────────────────────────────────────────────────────────────

/// End-to-end coverage for the `openhuman.skill_registry_*` endpoints.
///
/// Steps:
/// 1. `sources`  — lists the distinct upstream sources from the Hermes catalog.
/// 2. `browse`   — fetches the live catalog (force_refresh = true).
/// 3. `search`   — queries for "git" and expects at least one match.
/// 4. `schemas`  — exposes CLI/RPC schemas for prod smoke scripts.
/// 5. `install`  — happy-path install of a skill.
/// 6. `install`  — duplicate install returns idempotent success with no new skills.
/// 7. `uninstall` — removes the installed skill.
#[tokio::test]
async fn skill_registry_e2e_sources_browse_search_install() {
    let _env_lock = env_lock();

    let tmp = tempdir().expect("create tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _token_guard = EnvVarGuard::set(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN);
    let _keyring_guard = EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file");

    let cfg_dir = openhuman_home.clone();
    std::fs::create_dir_all(&cfg_dir).expect("create .openhuman dir");
    std::fs::write(
        cfg_dir.join("config.toml"),
        r#"api_url = "http://127.0.0.1:9"
default_model = "skill-e2e-model"

[secrets]
encrypt = false
"#,
    )
    .expect("write config.toml");

    let user_cfg_dir = openhuman_home.join("users").join("local");
    std::fs::create_dir_all(&user_cfg_dir).expect("create users/local dir");
    std::fs::write(
        user_cfg_dir.join("config.toml"),
        r#"api_url = "http://127.0.0.1:9"
default_model = "skill-e2e-model"

[secrets]
encrypt = false
"#,
    )
    .expect("write users/local/config.toml");

    let (fixture_addr, fixture_join) = serve_fixture_catalog().await;
    let fixture_base = format!("http://{fixture_addr}");
    let _catalog_guard = EnvVarGuard::set(
        "OPENHUMAN_SKILL_REGISTRY_CATALOG_URL",
        &format!("{fixture_base}/skills.json"),
    );
    let _download_guard = EnvVarGuard::set(
        "OPENHUMAN_SKILL_REGISTRY_DOWNLOAD_BASE_URL",
        &format!("{fixture_base}/skills"),
    );
    let _local_http_guard = EnvVarGuard::set("OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP", "1");

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    // ── Step 1: sources ────────────────────────────────────────────────────

    let sources_resp = post_json_rpc(
        &rpc_base,
        9001,
        "openhuman.skill_registry_sources",
        json!({}),
    )
    .await;
    let sources_result = assert_no_jsonrpc_error(&sources_resp, "skill_registry_sources");

    let sources = sources_result
        .get("sources")
        .and_then(Value::as_array)
        .expect("sources result must contain a `sources` array");

    assert!(
        !sources.is_empty(),
        "expected at least one source from the Hermes catalog"
    );

    // Sources should be string values (e.g. "built-in", "ClawHub", "skills.sh").
    for source in sources {
        assert!(
            source.is_string(),
            "each source must be a string, got: {source}"
        );
    }

    // ── Step 2: browse (force_refresh = true) ─────────────────────────────

    let browse_resp = post_json_rpc(
        &rpc_base,
        9002,
        "openhuman.skill_registry_browse",
        json!({ "force_refresh": true }),
    )
    .await;
    let browse_result = assert_no_jsonrpc_error(&browse_resp, "skill_registry_browse");

    let entries = browse_result
        .get("entries")
        .and_then(Value::as_array)
        .expect("browse result must contain an `entries` array");

    assert!(
        !entries.is_empty(),
        "browse catalog must return at least one entry after force_refresh"
    );

    // Every entry must carry the required fields.
    let required_entry_fields = [
        "id",
        "name",
        "description",
        "download_url",
        "source",
        "category",
    ];
    for entry in entries.iter().take(10) {
        for field in &required_entry_fields {
            assert!(
                entry.get(field).is_some(),
                "catalog entry missing field '{field}': {entry}"
            );
        }
    }

    // ── Step 3: search ────────────────────────────────────────────────────

    let search_resp = post_json_rpc(
        &rpc_base,
        9003,
        "openhuman.skill_registry_search",
        json!({ "query": "git" }),
    )
    .await;
    let search_result = assert_no_jsonrpc_error(&search_resp, "skill_registry_search (git)");

    let search_entries = search_result
        .get("entries")
        .and_then(Value::as_array)
        .expect("search result must contain an `entries` array");

    assert!(
        !search_entries.is_empty(),
        "search for 'git' must return at least one match"
    );

    for entry in search_entries.iter().take(5) {
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        let desc = entry
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        let tags: Vec<String> = entry
            .get("tags")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t.as_str())
                    .map(str::to_lowercase)
                    .collect()
            })
            .unwrap_or_default();
        let category = entry
            .get("category")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        let author = entry
            .get("author")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();

        let matches = name.contains("git")
            || desc.contains("git")
            || tags.iter().any(|t| t.contains("git"))
            || category.contains("git")
            || author.contains("git");
        assert!(
            matches,
            "search result entry does not match query 'git': {entry}"
        );
    }

    // ── Step 4: schemas ───────────────────────────────────────────────────

    let schemas_resp = post_json_rpc(
        &rpc_base,
        9004,
        "openhuman.skill_registry_schemas",
        json!({}),
    )
    .await;
    let schemas_result = assert_no_jsonrpc_error(&schemas_resp, "skill_registry_schemas");
    let schemas = schemas_result
        .get("schemas")
        .and_then(Value::as_array)
        .expect("schemas result must contain a `schemas` array");
    assert!(
        schemas.iter().any(|schema| {
            schema.get("function").and_then(Value::as_str) == Some("install")
                && schema.get("namespace").and_then(Value::as_str) == Some("skill_registry")
        }),
        "schemas must include skill_registry install schema: {schemas:?}"
    );

    // ── Step 5: install (happy path) ──────────────────────────────────────

    // Find the fixture entry with a local download_url.
    let install_target = entries
        .iter()
        .find(|e| {
            e.get("download_url")
                .and_then(Value::as_str)
                .map(|u| u == format!("{fixture_base}/skills/git-helper/SKILL.md"))
                .unwrap_or(false)
        })
        .expect("expected the fixture git-helper download_url");

    let entry_id = install_target
        .get("id")
        .and_then(Value::as_str)
        .expect("install_target id");

    let install_resp = post_json_rpc(
        &rpc_base,
        9005,
        "openhuman.skill_registry_install",
        json!({ "entry_id": entry_id }),
    )
    .await;
    let install_result = assert_no_jsonrpc_error(&install_resp, "skill_registry_install (happy)");

    let install_url = install_result
        .get("url")
        .and_then(Value::as_str)
        .expect("install result must contain `url`");
    assert!(
        !install_url.is_empty(),
        "install result `url` must not be empty"
    );

    let install_stdout = install_result
        .get("stdout")
        .and_then(Value::as_str)
        .expect("install result must contain `stdout`");
    assert!(
        install_stdout.contains("Installed to"),
        "install stdout should mention 'Installed to', got: {install_stdout}"
    );

    let _install_stderr = install_result
        .get("stderr")
        .expect("install result must contain `stderr`");

    let new_skills = install_result
        .get("new_skills")
        .and_then(Value::as_array)
        .expect("install result must contain `new_skills` array");
    assert!(
        new_skills.iter().any(|s| s.as_str() == Some(entry_id)),
        "new_skills must contain '{entry_id}', got: {new_skills:?}"
    );

    // Verify the SKILL.md file actually landed on disk.
    let skill_file = home
        .join(".openhuman")
        .join("skills")
        .join(entry_id)
        .join("SKILL.md");
    assert!(
        skill_file.exists(),
        "SKILL.md should exist on disk at {}, but was not found",
        skill_file.display()
    );

    // ── Step 6: install (duplicate no-op success) ────────────────────────

    let dup_resp = post_json_rpc(
        &rpc_base,
        9006,
        "openhuman.skill_registry_install",
        json!({ "entry_id": entry_id }),
    )
    .await;
    let dup_result = assert_no_jsonrpc_error(&dup_resp, "skill_registry_install (duplicate)");
    let dup_stdout = dup_result
        .get("stdout")
        .and_then(Value::as_str)
        .expect("duplicate install result must contain `stdout`");
    assert!(
        dup_stdout.contains("already installed"),
        "duplicate install stdout should mention 'already installed', got: {dup_stdout}"
    );
    let dup_new_skills = dup_result
        .get("new_skills")
        .and_then(Value::as_array)
        .expect("duplicate install result must contain `new_skills` array");
    assert!(
        dup_new_skills.is_empty(),
        "duplicate install should report no new skills, got: {dup_new_skills:?}"
    );

    // ── Step 7: uninstall ─────────────────────────────────────────────────

    let uninstall_resp = post_json_rpc(
        &rpc_base,
        9007,
        "openhuman.skill_registry_uninstall",
        json!({ "name": entry_id }),
    )
    .await;
    let uninstall_result = assert_no_jsonrpc_error(&uninstall_resp, "skill_registry_uninstall");
    assert_eq!(
        uninstall_result.get("name").and_then(Value::as_str),
        Some(entry_id)
    );
    assert!(
        !skill_file.exists(),
        "SKILL.md should be removed after uninstall at {}",
        skill_file.display()
    );

    // ── Cleanup ───────────────────────────────────────────────────────────

    rpc_join.abort();
    fixture_join.abort();
}
