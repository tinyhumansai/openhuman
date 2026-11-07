//! Saving an API key from Settings over JSON-RPC, through the real
//! `openhuman-core serve` binary and its native TinySecurity cipher.
//!
//! The save seals the key, which needs a one-time `native_secret_grant` from a
//! person. A Settings RPC has no chat thread behind it, so the grant must park
//! as an unrouted pending approval (listed by `approval_list_pending`, which
//! the desktop's Settings deck polls) rather than fail before anyone is asked.
//! Approving it completes the save with the key sealed on disk; denying it
//! fails the save with a reason the Settings UI can show.
//!
//! No keyring consent responder answers here: `OPENHUMAN_TEST_KEYRING_CONSENT`
//! is removed from the child, as in a product build.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const TOKEN: &str = "settings-secret-grant-e2e-token";
const API_KEY: &str = "settings-secret-grant-e2e-value";

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start(home: &Path) -> (Server, String) {
    // The port is chosen, then released for the child to bind; if another
    // process takes it in between, the child exits and a fresh port is tried.
    for _attempt in 0..3 {
        if let Some(started) = try_start(home) {
            return started;
        }
    }
    panic!("openhuman-core never started on a free port");
}

fn try_start(home: &Path) -> Option<(Server, String)> {
    let port = free_port();
    let workspace = home.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
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
    .env("OPENHUMAN_WORKSPACE", &workspace)
    .env("OPENHUMAN_CORE_TOKEN", TOKEN)
    .env("OPENHUMAN_KEYRING_BACKEND", "file")
    .env_remove("OPENHUMAN_TEST_KEYRING_CONSENT")
    .env_remove("OPENHUMAN_KEYRING_MASTER_KEY")
    .env_remove("OPENHUMAN_KEYRING_MASTER_KEY_FILE")
    .env_remove("OPENHUMAN_STORAGE_URL")
    .env_remove("OPENHUMAN_MODE")
    .env_remove("BACKEND_URL")
    .stdout(Stdio::null())
    .stderr(Stdio::null());
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
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap()
        .post(format!("{base}/rpc"))
        .bearer_auth(TOKEN)
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
        .send()
        .expect("POST /rpc")
        .json()
        .expect("json-rpc response")
}

/// Wait for the save's secret grant to park in the durable pending queue the
/// Settings deck reads, and return its request id.
fn wait_for_secret_grant(base: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let listed = rpc(base, "openhuman.approval_list_pending", json!({}));
        let rows = listed["result"]
            .as_array()
            .or_else(|| listed["result"]["result"].as_array())
            .cloned()
            .unwrap_or_default();
        if let Some(row) = rows
            .iter()
            .find(|row| row["tool_name"] == "native_secret_grant")
        {
            return row["request_id"].as_str().unwrap().to_owned();
        }
        assert!(
            Instant::now() < deadline,
            "the Settings save never parked a native_secret_grant: {listed}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Save an API key from "Settings" and answer its secret grant with `decision`.
/// Returns the save's JSON-RPC response and the workspace home.
fn save_api_key_and_decide(decision: &str) -> (Value, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let (_server, base) = start(home.path());
    let save_base = base.clone();
    let save = std::thread::spawn(move || {
        rpc(
            &save_base,
            "openhuman.config_update_model_settings",
            json!({ "api_key": API_KEY }),
        )
    });
    let request_id = wait_for_secret_grant(&base);
    assert!(
        !save.is_finished(),
        "the save must wait for the person's decision"
    );
    let decided = rpc(
        &base,
        "openhuman.approval_decide",
        json!({ "request_id": request_id, "decision": decision }),
    );
    assert!(decided.get("error").is_none(), "decide failed: {decided}");
    (save.join().expect("save thread"), home)
}

/// Every regular file under `root`, recursively.
fn files_under(root: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.is_file() {
                files.push(path);
            }
        }
    }
    files
}

#[test]
fn settings_api_key_save_parks_its_secret_grant_and_completes_on_approve() {
    let (saved, home) = save_api_key_and_decide("approve_once");
    assert!(
        saved.get("error").is_none(),
        "an approved save must complete: {saved}"
    );
    for path in files_under(home.path()) {
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !bytes
                .windows(API_KEY.len())
                .any(|window| window == API_KEY.as_bytes()),
            "the API key was written in plaintext at {}",
            path.display()
        );
    }
}

#[test]
fn settings_api_key_save_fails_closed_with_the_reason_when_its_grant_is_denied() {
    let (saved, _home) = save_api_key_and_decide("deny");
    let message = saved["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("a denied save must fail: {saved}"));
    assert!(
        message.contains(
            "Failed to encrypt api_key: the approval to unlock configuration secrets was denied"
        ),
        "{message}"
    );
}
