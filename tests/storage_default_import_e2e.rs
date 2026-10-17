//! `openhuman-core serve` on a workspace that already has legacy approval
//! rows: with no storage URL configured (the default), the host opens
//! `sqlite:<workspace>/storage`, the importer copies the old `approval.db`
//! rows into it once, and the legacy file stays byte-for-byte untouched.
//! Through the real binary and its JSON-RPC surface.
//!
//! The second test boots the same workspace on an explicit storage URL, where
//! the importer does the same into that backend.

use openhuman_core::storage::rusqlite;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const TOKEN: &str = "storage-default-import-e2e-token";

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A legacy `approval/approval.db` under `workspace`, laid out as the retired
/// store wrote it: one undecided request, one approved request and one flow
/// trust grant.
fn legacy_workspace(workspace: &Path) {
    let db = workspace.join("approval").join("approval.db");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE pending_approvals (
             request_id TEXT PRIMARY KEY, tool_name TEXT NOT NULL,
             action_summary TEXT NOT NULL, args_redacted TEXT NOT NULL,
             session_id TEXT NOT NULL, created_at TEXT NOT NULL, expires_at TEXT,
             decided_at TEXT, decision TEXT, executed_at TEXT, execution_outcome TEXT,
             execution_error TEXT, source_context TEXT, tool_call_id TEXT, agent_id TEXT);
         CREATE TABLE flow_tool_trust (
             flow_id TEXT NOT NULL, tool_name TEXT NOT NULL, created_at TEXT NOT NULL,
             PRIMARY KEY (flow_id, tool_name));",
    )
    .unwrap();
    let now = chrono::Utc::now();
    let soon = (now + chrono::Duration::hours(1)).to_rfc3339();
    for (id, decided) in [("still-pending", false), ("already-approved", true)] {
        conn.execute(
            "INSERT INTO pending_approvals
                (request_id, tool_name, action_summary, args_redacted, session_id,
                 created_at, expires_at, decided_at, decision)
             VALUES (?1, 'shell', ?2, '{\"cmd\":\"ls\"}', 'sess', ?3, ?4, ?5, ?6)",
            rusqlite::params![
                id,
                format!("run {id}"),
                now.to_rfc3339(),
                soon,
                decided.then(|| now.to_rfc3339()),
                decided.then_some("approve_once"),
            ],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO flow_tool_trust (flow_id, tool_name, created_at) VALUES ('flow-1', 'shell', ?1)",
        [now.to_rfc3339()],
    )
    .unwrap();
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start(home: &Path, workspace: &Path, url: Option<&str>) -> (Server, String) {
    // The port is chosen, then released for the child to bind; if another
    // process takes it in between, the child exits and a fresh port is tried.
    for _attempt in 0..3 {
        if let Some(started) = try_start(home, workspace, url) {
            return started;
        }
    }
    panic!("openhuman-core never started on a free port");
}

fn try_start(home: &Path, workspace: &Path, url: Option<&str>) -> Option<(Server, String)> {
    let port = free_port();
    // A debug build of the full product set needs more than the default 8 MiB
    // main-thread stack to finish booting, so raise the limit first.
    let mut cmd = Command::new("sh");
    cmd.args([
        "-c",
        "ulimit -s unlimited 2>/dev/null || ulimit -s 1048576 2>/dev/null; exec \"$0\" \"$@\"",
        env!("CARGO_BIN_EXE_openhuman-core"),
        "serve",
        "--port",
        &port.to_string(),
    ])
    .env("HOME", home)
    .env("USERPROFILE", home)
    .env("OPENHUMAN_WORKSPACE", workspace)
    .env("OPENHUMAN_CORE_TOKEN", TOKEN)
    .env_remove("OPENHUMAN_STORAGE_URL")
    .env_remove("OPENHUMAN_MODE")
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    if let Some(url) = url {
        cmd.env("OPENHUMAN_STORAGE_URL", url);
    }
    let mut server = Server(cmd.spawn().expect("spawn openhuman-core"));
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::blocking::Client::new();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Ok(response) = client.get(format!("{base}/health")).send() {
            if response.status().is_success() {
                return Some((server, base));
            }
        }
        if let Ok(Some(_)) = server.0.try_wait() {
            return None;
        }
        assert!(
            Instant::now() < deadline,
            "openhuman-core never became healthy"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn rpc(base: &str, method: &str, params: Value) -> Value {
    reqwest::blocking::Client::new()
        .post(format!("{base}/rpc"))
        .bearer_auth(TOKEN)
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
        .send()
        .expect("POST /rpc")
        .json()
        .expect("json-rpc response")
}

/// Every file under `dir` (relative path, bytes), for proving nothing in it
/// was modified, created or removed.
fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        out.push((
            path.file_name().unwrap().to_string_lossy().into_owned(),
            std::fs::read(&path).unwrap(),
        ));
    }
    out.sort();
    out
}

#[test]
fn serve_imports_legacy_approvals_by_default_and_leaves_the_file_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("workspace");
    legacy_workspace(&workspace);
    let legacy_dir = workspace.join("approval");
    let before = snapshot(&legacy_dir);

    // Two boots: the rows are served from the workspace's default database
    // each time, and the second one imports nothing again.
    for boot in 0..2 {
        let (_server, base) = start(tmp.path(), &workspace, None);
        let listed = rpc(&base, "openhuman.approval_list_pending", json!({})).to_string();
        assert!(listed.contains("still-pending"), "boot {boot}: {listed}");
        assert!(
            !listed.contains("already-approved"),
            "boot {boot}: {listed}"
        );
        let recent = rpc(&base, "openhuman.approval_list_recent_decisions", json!({})).to_string();
        assert!(recent.contains("already-approved"), "boot {boot}: {recent}");
    }

    assert!(
        workspace.join("storage").join("storage.db").is_file(),
        "the default backend lives in the workspace"
    );
    assert_eq!(
        snapshot(&legacy_dir),
        before,
        "the legacy database was not modified and no companion file was created"
    );
}

#[test]
fn serve_on_an_explicit_url_imports_legacy_approvals_without_touching_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("workspace");
    legacy_workspace(&workspace);
    let legacy_dir = workspace.join("approval");
    let before = snapshot(&legacy_dir);
    let url = format!("sqlite:{}", tmp.path().join("storage").display());

    for boot in 0..2 {
        let (_server, base) = start(tmp.path(), &workspace, Some(&url));
        let listed = rpc(&base, "openhuman.approval_list_pending", json!({})).to_string();
        assert!(listed.contains("still-pending"), "boot {boot}: {listed}");
        assert!(
            !listed.contains("already-approved"),
            "boot {boot}: {listed}"
        );
        let recent = rpc(&base, "openhuman.approval_list_recent_decisions", json!({})).to_string();
        assert!(recent.contains("already-approved"), "boot {boot}: {recent}");
    }

    assert_eq!(
        snapshot(&legacy_dir),
        before,
        "the legacy database was not modified and no companion file was created"
    );
    assert!(
        !workspace.join("storage").exists(),
        "an explicit URL replaces the workspace default"
    );
}
