//! E2E tests for the memory_sources domain.
//!
//! Boots a real axum JSON-RPC server against an isolated workspace and
//! exercises the full user flow: add source → list → list_items →
//! read_item → ingest into memory tree → verify chunks indexed.
//!
//! Run with: `cargo test --test memory_sources_e2e`

use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::http::header::AUTHORIZATION;
use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;

const TEST_RPC_TOKEN: &str = "memory-sources-e2e-token";
static AUTH_INIT: OnceLock<()> = OnceLock::new();
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    let mutex = ENV_LOCK.get_or_init(|| Mutex::new(()));
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn ensure_rpc_auth() {
    AUTH_INIT.get_or_init(|| {
        unsafe { std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN) };
        let token_dir = std::env::temp_dir().join("openhuman-memory-sources-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth");
    });
}

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

fn write_config(dir: &Path) {
    std::fs::create_dir_all(dir).expect("mkdir");
    let cfg = r#"
default_model = "e2e-mock-model"
default_temperature = 0.7

[secrets]
encrypt = false

[memory_tree]
embedding_strict = false
"#;
    std::fs::write(dir.join("config.toml"), cfg).expect("write config");

    let user_dir = dir.join("users").join("local");
    std::fs::create_dir_all(&user_dir).expect("mkdir user dir");
    std::fs::write(user_dir.join("config.toml"), cfg).expect("write user config");
}

async fn serve() -> (String, tokio::task::JoinHandle<Result<(), std::io::Error>>) {
    ensure_rpc_auth();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle =
        tokio::spawn(async move { axum::serve(listener, build_core_http_router(false)).await });
    (format!("http://{addr}"), handle)
}

async fn rpc(base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("client");
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    let url = format!("{}/rpc", base.trim_end_matches('/'));
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
        resp.status(),
    );
    resp.json::<Value>()
        .await
        .unwrap_or_else(|e| panic!("json parse for {method}: {e}"))
}

fn ok(v: &Value, ctx: &str) -> Value {
    if let Some(err) = v.get("error") {
        panic!("{ctx}: JSON-RPC error: {err}");
    }
    let outer = v
        .get("result")
        .unwrap_or_else(|| panic!("{ctx}: missing result: {v}"));
    // RpcOutcome wraps the payload under an inner "result" key alongside "logs".
    if let Some(inner) = outer.get("result") {
        inner.clone()
    } else {
        outer.clone()
    }
}

// ── Tests ─────────────────────────────────────────────────────────

#[tokio::test]
async fn memory_sources_crud_and_folder_read_flow() {
    let _guard = env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home = EnvVarGuard::set_to_path("HOME", home);
    let _ws = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend = EnvVarGuard::unset("BACKEND_URL");
    let _vite = EnvVarGuard::unset("VITE_BACKEND_URL");

    write_config(&openhuman_home);

    // Create a folder with test markdown files.
    let notes_dir = home.join("test-notes");
    std::fs::create_dir_all(&notes_dir).expect("mkdir notes");
    std::fs::write(
        notes_dir.join("architecture.md"),
        "# System Architecture\n\n\
         The platform uses microservices with three core services.\n\n\
         ## Auth Service\n\
         Handles OAuth2 flows and JWT rotation. Contact: alice@platform.io\n\n\
         ## Data Pipeline\n\
         Event-driven pipeline using Kafka. Throughput: ~50k events/sec. \
         Owner: bob@platform.io\n\n\
         Last reviewed: 2025-12-15 by the platform team.",
    )
    .expect("write architecture.md");

    std::fs::write(
        notes_dir.join("runbook.md"),
        "# Incident Runbook\n\n\
         ## P1: Database Connection Pool Exhaustion\n\
         1. Check connection count via pg_stat_activity\n\
         2. Scale read replicas if > 90% utilization\n\
         3. Escalate to alice@platform.io if write-primary affected\n\n\
         Last updated: 2025-11-30",
    )
    .expect("write runbook.md");

    let (rpc_base, rpc_join) = serve().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // ── Step 1: list sources (empty initially) ──

    let list0 = rpc(&rpc_base, 1, "openhuman.memory_sources_list", json!({})).await;
    let list0_result = ok(&list0, "initial list");
    let sources = list0_result
        .get("sources")
        .and_then(Value::as_array)
        .expect("sources array");
    assert!(sources.is_empty(), "should start with no sources");

    // ── Step 2: add a folder source ──

    let add = rpc(
        &rpc_base,
        2,
        "openhuman.memory_sources_add",
        json!({
            "kind": "folder",
            "label": "Test Research Notes",
            "path": notes_dir.to_string_lossy(),
            "glob": "**/*.md",
        }),
    )
    .await;
    let add_result = ok(&add, "add folder source");
    let source = add_result.get("source").expect("source in add response");
    let source_id = source.get("id").and_then(Value::as_str).expect("source id");
    assert_eq!(source.get("kind").and_then(Value::as_str), Some("folder"));
    assert_eq!(
        source.get("label").and_then(Value::as_str),
        Some("Test Research Notes")
    );
    assert_eq!(source.get("enabled"), Some(&json!(true)));

    // ── Step 3: list sources (now has 1) ──

    let list1 = rpc(&rpc_base, 3, "openhuman.memory_sources_list", json!({})).await;
    let list1_result = ok(&list1, "list after add");
    let sources = list1_result
        .get("sources")
        .and_then(Value::as_array)
        .expect("sources array");
    assert_eq!(sources.len(), 1);

    // ── Step 4: get source by id ──

    let get = rpc(
        &rpc_base,
        4,
        "openhuman.memory_sources_get",
        json!({ "id": source_id }),
    )
    .await;
    let get_result = ok(&get, "get source");
    let fetched = get_result.get("source").expect("source");
    assert_eq!(fetched.get("id").and_then(Value::as_str), Some(source_id));

    // ── Step 5: list items from the folder source ──

    let items_resp = rpc(
        &rpc_base,
        5,
        "openhuman.memory_sources_list_items",
        json!({ "source_id": source_id }),
    )
    .await;
    let items_result = ok(&items_resp, "list_items");
    let items = items_result
        .get("items")
        .and_then(Value::as_array)
        .expect("items array");
    assert_eq!(items.len(), 2, "should list 2 markdown files");

    let item_ids: Vec<&str> = items
        .iter()
        .filter_map(|i| i.get("id").and_then(Value::as_str))
        .collect();
    assert!(item_ids.contains(&"architecture.md"));
    assert!(item_ids.contains(&"runbook.md"));

    // ── Step 6: read one item's content ──

    let read = rpc(
        &rpc_base,
        6,
        "openhuman.memory_sources_read_item",
        json!({
            "source_id": source_id,
            "item_id": "architecture.md",
        }),
    )
    .await;
    let read_result = ok(&read, "read_item");
    let content = read_result.get("content").expect("content");
    let body = content.get("body").and_then(Value::as_str).expect("body");
    assert!(body.contains("System Architecture"));
    assert!(body.contains("alice@platform.io"));
    assert_eq!(
        content.get("content_type").and_then(Value::as_str),
        Some("markdown")
    );

    // ── Step 7: ingest the content into the memory tree ──

    let ingest = rpc(
        &rpc_base,
        7,
        "openhuman.memory_tree_ingest",
        json!({
            "source_kind": "document",
            "source_id": format!("memory_sources:{source_id}:architecture.md"),
            "owner": "user",
            "tags": ["memory_sources", "folder"],
            "payload": {
                "provider": "memory_sources",
                "title": "System Architecture",
                "body": body,
            },
        }),
    )
    .await;
    let ingest_result = ok(&ingest, "memory_tree ingest");
    let chunks_written = ingest_result
        .get("chunks_written")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    assert!(
        chunks_written >= 1,
        "should ingest at least 1 chunk, got {chunks_written}; full result: {ingest_result}"
    );

    // ── Step 8: verify chunks exist via list_sources ──

    let ls = rpc(
        &rpc_base,
        8,
        "openhuman.memory_tree_list_sources",
        json!({}),
    )
    .await;
    let ls_result = ok(&ls, "memory_tree list_sources");
    // list_sources returns {sources: [...]} — but some RPCs wrap it differently.
    let mem_sources = if let Some(arr) = ls_result.as_array() {
        arr.clone()
    } else if let Some(arr) = ls_result.get("sources").and_then(Value::as_array) {
        arr.clone()
    } else {
        panic!("expected sources array in: {ls_result}");
    };
    assert!(
        !mem_sources.is_empty(),
        "memory tree should have at least one source after ingest"
    );

    // ── Step 9: update source label ──

    let update = rpc(
        &rpc_base,
        9,
        "openhuman.memory_sources_update",
        json!({
            "id": source_id,
            "label": "Renamed Notes",
        }),
    )
    .await;
    let update_result = ok(&update, "update source");
    assert_eq!(
        update_result
            .get("source")
            .and_then(|s| s.get("label"))
            .and_then(Value::as_str),
        Some("Renamed Notes")
    );

    // ── Step 10: disable source ──

    let disable = rpc(
        &rpc_base,
        10,
        "openhuman.memory_sources_update",
        json!({
            "id": source_id,
            "enabled": false,
        }),
    )
    .await;
    let disable_result = ok(&disable, "disable source");
    assert_eq!(
        disable_result.get("source").and_then(|s| s.get("enabled")),
        Some(&json!(false))
    );

    // ── Step 11: remove source ──

    let remove = rpc(
        &rpc_base,
        11,
        "openhuman.memory_sources_remove",
        json!({ "id": source_id }),
    )
    .await;
    let remove_result = ok(&remove, "remove source");
    assert_eq!(remove_result.get("removed"), Some(&json!(true)));

    // Verify it's gone.
    let list_final = rpc(&rpc_base, 12, "openhuman.memory_sources_list", json!({})).await;
    let final_sources = ok(&list_final, "final list")
        .get("sources")
        .and_then(Value::as_array)
        .expect("sources")
        .len();
    assert_eq!(final_sources, 0, "source should be removed");

    // ── Step 12: removing again is idempotent ──

    let remove_again = rpc(
        &rpc_base,
        13,
        "openhuman.memory_sources_remove",
        json!({ "id": source_id }),
    )
    .await;
    let remove_again_result = ok(&remove_again, "remove again");
    assert_eq!(remove_again_result.get("removed"), Some(&json!(false)));

    rpc_join.abort();
}

#[tokio::test]
async fn memory_sources_validation_rejects_bad_input() {
    let _guard = env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home = EnvVarGuard::set_to_path("HOME", home);
    let _ws = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend = EnvVarGuard::unset("BACKEND_URL");
    let _vite = EnvVarGuard::unset("VITE_BACKEND_URL");

    write_config(&openhuman_home);

    let (rpc_base, rpc_join) = serve().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Folder source without path → should error.
    let bad_add = rpc(
        &rpc_base,
        20,
        "openhuman.memory_sources_add",
        json!({
            "kind": "folder",
            "label": "Missing path",
        }),
    )
    .await;
    assert!(
        bad_add.get("error").is_some(),
        "adding folder without path should fail: {bad_add}"
    );

    // GitHub source without url → should error.
    let bad_gh = rpc(
        &rpc_base,
        21,
        "openhuman.memory_sources_add",
        json!({
            "kind": "github_repo",
            "label": "No URL",
        }),
    )
    .await;
    assert!(
        bad_gh.get("error").is_some(),
        "adding github_repo without url should fail: {bad_gh}"
    );

    // list_items for nonexistent source → should error.
    let bad_items = rpc(
        &rpc_base,
        22,
        "openhuman.memory_sources_list_items",
        json!({ "source_id": "nonexistent" }),
    )
    .await;
    assert!(
        bad_items.get("error").is_some(),
        "list_items for missing source should fail: {bad_items}"
    );

    rpc_join.abort();
}

/// GitHub source E2E: add a public repo → list_items (commits/issues/PRs)
/// → read one commit and one issue → ingest into memory tree.
///
/// Requires network + `gh` CLI (or unauthenticated GitHub API access).
/// The test targets a small, stable public repo so API responses are
/// predictable. Gated behind `OPENHUMAN_E2E_NETWORK=1` so CI without
/// outbound GitHub access doesn't fail on rate limits or transient
/// network blips. Run locally with:
///   OPENHUMAN_E2E_NETWORK=1 cargo test --test memory_sources_e2e \
///     memory_sources_github_repo_activity_flow
#[tokio::test]
async fn memory_sources_github_repo_activity_flow() {
    if std::env::var("OPENHUMAN_E2E_NETWORK").ok().as_deref() != Some("1") {
        eprintln!(
            "skipping memory_sources_github_repo_activity_flow — set OPENHUMAN_E2E_NETWORK=1 to enable"
        );
        return;
    }
    let _guard = env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home = EnvVarGuard::set_to_path("HOME", home);
    let _ws = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend = EnvVarGuard::unset("BACKEND_URL");
    let _vite = EnvVarGuard::unset("VITE_BACKEND_URL");

    write_config(&openhuman_home);

    let (rpc_base, rpc_join) = serve().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // ── Step 1: add a GitHub repo source ──

    let add = rpc(
        &rpc_base,
        100,
        "openhuman.memory_sources_add",
        json!({
            "kind": "github_repo",
            "label": "kelseyhightower/nocode",
            "url": "https://github.com/kelseyhightower/nocode",
        }),
    )
    .await;
    let add_result = ok(&add, "add github source");
    let source = add_result.get("source").expect("source");
    let source_id = source.get("id").and_then(Value::as_str).expect("id");
    assert_eq!(
        source.get("kind").and_then(Value::as_str),
        Some("github_repo")
    );

    // ── Step 2: list items — should return commits, issues, PRs ──

    let items_resp = rpc(
        &rpc_base,
        101,
        "openhuman.memory_sources_list_items",
        json!({ "source_id": source_id }),
    )
    .await;
    let items_result = ok(&items_resp, "github list_items");
    let items = items_result
        .get("items")
        .and_then(Value::as_array)
        .expect("items array");

    assert!(
        !items.is_empty(),
        "github repo should have at least some activity items"
    );

    let has_commits = items.iter().any(|i| {
        i.get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .starts_with("commit:")
    });
    assert!(has_commits, "should have commit items");

    // ── Step 3: read a commit ──

    let commit_item = items
        .iter()
        .find(|i| {
            i.get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .starts_with("commit:")
        })
        .expect("at least one commit");
    let commit_id = commit_item
        .get("id")
        .and_then(Value::as_str)
        .expect("commit id");

    let read_commit = rpc(
        &rpc_base,
        102,
        "openhuman.memory_sources_read_item",
        json!({
            "source_id": source_id,
            "item_id": commit_id,
        }),
    )
    .await;
    let commit_content = ok(&read_commit, "read commit");
    let content = commit_content.get("content").expect("content");
    let body = content.get("body").and_then(Value::as_str).expect("body");
    assert!(body.contains("Commit:"), "commit body should have header");
    assert!(body.contains("SHA:"), "commit body should have SHA");
    assert_eq!(
        content.get("content_type").and_then(Value::as_str),
        Some("markdown")
    );

    // ── Step 4: ingest the commit into memory tree ──

    let ingest = rpc(
        &rpc_base,
        103,
        "openhuman.memory_tree_ingest",
        json!({
            "source_kind": "document",
            "source_id": format!("github:{source_id}:{commit_id}"),
            "owner": "user",
            "tags": ["memory_sources", "github", "commit"],
            "payload": {
                "provider": "github",
                "title": commit_item.get("title").and_then(Value::as_str).unwrap_or("commit"),
                "body": body,
            },
        }),
    )
    .await;
    let ingest_result = ok(&ingest, "ingest github commit");
    let chunks = ingest_result
        .get("chunks_written")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    assert!(
        chunks >= 1,
        "should ingest at least 1 chunk from commit, got {chunks}"
    );

    // ── Step 5: read an issue if one exists ──

    if let Some(issue_item) = items.iter().find(|i| {
        i.get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .starts_with("issue:")
    }) {
        let issue_id = issue_item
            .get("id")
            .and_then(Value::as_str)
            .expect("issue id");

        let read_issue = rpc(
            &rpc_base,
            104,
            "openhuman.memory_sources_read_item",
            json!({
                "source_id": source_id,
                "item_id": issue_id,
            }),
        )
        .await;
        let issue_content = ok(&read_issue, "read issue");
        let icontent = issue_content.get("content").expect("content");
        let ibody = icontent.get("body").and_then(Value::as_str).expect("body");
        assert!(ibody.contains("Issue #"), "issue body should have header");
        assert!(ibody.contains("State:"), "issue body should have state");
    }

    // ── Cleanup ──

    let remove = rpc(
        &rpc_base,
        105,
        "openhuman.memory_sources_remove",
        json!({ "id": source_id }),
    )
    .await;
    ok(&remove, "remove github source");

    rpc_join.abort();
}

/// Composio source E2E: add a composio source entry → verify it's in
/// the registry → list_items returns the connection as an item →
/// read_item returns a descriptive placeholder → remove.
///
/// Does NOT require an actual Composio connection — tests the registry
/// and reader behavior with synthetic config.
#[tokio::test]
async fn memory_sources_composio_registry_flow() {
    let _guard = env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home = EnvVarGuard::set_to_path("HOME", home);
    let _ws = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend = EnvVarGuard::unset("BACKEND_URL");
    let _vite = EnvVarGuard::unset("VITE_BACKEND_URL");

    write_config(&openhuman_home);

    let (rpc_base, rpc_join) = serve().await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // ── Step 1: add a composio source ──

    let add = rpc(
        &rpc_base,
        200,
        "openhuman.memory_sources_add",
        json!({
            "kind": "composio",
            "label": "Gmail · test@example.com",
            "toolkit": "gmail",
            "connection_id": "cmp_test_123",
        }),
    )
    .await;
    let add_result = ok(&add, "add composio source");
    let source = add_result.get("source").expect("source");
    let source_id = source.get("id").and_then(Value::as_str).expect("id");
    assert_eq!(source.get("kind").and_then(Value::as_str), Some("composio"));
    assert_eq!(source.get("toolkit").and_then(Value::as_str), Some("gmail"));
    assert_eq!(
        source.get("connection_id").and_then(Value::as_str),
        Some("cmp_test_123")
    );

    // ── Step 2: verify it shows up in list ──

    let list = rpc(&rpc_base, 201, "openhuman.memory_sources_list", json!({})).await;
    let list_result = ok(&list, "list with composio");
    let sources = list_result
        .get("sources")
        .and_then(Value::as_array)
        .expect("sources");
    assert_eq!(sources.len(), 1);
    assert_eq!(
        sources[0].get("toolkit").and_then(Value::as_str),
        Some("gmail")
    );

    // ── Step 3: list_items returns the connection as an item ──

    let items = rpc(
        &rpc_base,
        202,
        "openhuman.memory_sources_list_items",
        json!({ "source_id": source_id }),
    )
    .await;
    let items_result = ok(&items, "composio list_items");
    let item_list = items_result
        .get("items")
        .and_then(Value::as_array)
        .expect("items");
    assert_eq!(item_list.len(), 1);
    assert_eq!(
        item_list[0].get("id").and_then(Value::as_str),
        Some("cmp_test_123")
    );

    // ── Step 4: read_item returns descriptive content ──

    let read = rpc(
        &rpc_base,
        203,
        "openhuman.memory_sources_read_item",
        json!({
            "source_id": source_id,
            "item_id": "cmp_test_123",
        }),
    )
    .await;
    let read_result = ok(&read, "composio read_item");
    let content = read_result.get("content").expect("content");
    let body = content.get("body").and_then(Value::as_str).expect("body");
    assert!(
        body.contains("gmail"),
        "composio read should mention the toolkit"
    );

    // ── Step 5: add a second composio source (slack) ──

    let add2 = rpc(
        &rpc_base,
        204,
        "openhuman.memory_sources_add",
        json!({
            "kind": "composio",
            "label": "Slack · workspace",
            "toolkit": "slack",
            "connection_id": "cmp_test_456",
        }),
    )
    .await;
    let add2_result = ok(&add2, "add slack composio source");
    let slack_id = add2_result
        .get("source")
        .and_then(|s| s.get("id"))
        .and_then(Value::as_str)
        .expect("slack source id");

    // ── Step 6: list should have both ──

    let list2 = rpc(&rpc_base, 205, "openhuman.memory_sources_list", json!({})).await;
    let sources2 = ok(&list2, "list with both")
        .get("sources")
        .and_then(Value::as_array)
        .expect("sources")
        .len();
    assert_eq!(sources2, 2);

    // ── Step 7: disable gmail, verify it persists ──

    let disable = rpc(
        &rpc_base,
        206,
        "openhuman.memory_sources_update",
        json!({
            "id": source_id,
            "enabled": false,
        }),
    )
    .await;
    let disabled = ok(&disable, "disable gmail");
    assert_eq!(
        disabled.get("source").and_then(|s| s.get("enabled")),
        Some(&json!(false))
    );

    // ── Step 8: remove both ──

    for (idx, sid) in [source_id, slack_id].iter().enumerate() {
        let r = rpc(
            &rpc_base,
            210 + idx as i64,
            "openhuman.memory_sources_remove",
            json!({ "id": sid }),
        )
        .await;
        ok(&r, &format!("remove {sid}"));
    }

    rpc_join.abort();
}
