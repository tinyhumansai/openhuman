//! Smoke test of the released TinyMCP cdylib under the real host and its
//! storage backend (#7737): add a server with a secret, restart the core, and
//! list it again.
//!
//! The module is a loadable cdylib. Its servers, name claims and env secrets
//! live in the host's storage backend, reached over the `tinystorage` bus
//! contract, not in files of its own. The test spawns `openhuman-core serve`
//! twice on one workspace and one storage URL and checks, through JSON-RPC,
//! that:
//!
//! * the server added in the first process is listed by the second;
//! * its env key is listed but the secret value is never returned over RPC;
//! * the value is not in the storage file in clear text;
//! * the module wrote nothing to its old `mcp_clients/` SQLite files.
//!
//! Needs a released TinyMCP library on `OPENHUMAN_MODULE_PATH` (the runner,
//! `scripts/test-rust-e2e.sh`, stages the registry-pinned release) or a build
//! from `scripts/modules/build.mjs tinymcp`, so the target is opt-in via
//! `e2e-test-support` rather than included in a bare `cargo test`. Nothing here
//! reaches the network: the server is a stub that is added disabled and never
//! dialled.

#![cfg(all(feature = "mcp", feature = "modules", feature = "storage-sqlite"))]

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const TOKEN: &str = "tinymcp-storage-smoke-token";
const SERVER: &str = "@smoke/files";
const SECRET: &str = "sk-smoke-secret-value";

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

fn start(home: &Path, workspace: &Path, url: &str, modules: &Path) -> (Server, String) {
    for _attempt in 0..3 {
        if let Some(started) = try_start(home, workspace, url, modules) {
            return started;
        }
    }
    panic!("openhuman-core never started on a free port");
}

fn try_start(home: &Path, workspace: &Path, url: &str, modules: &Path) -> Option<(Server, String)> {
    let port = free_port();
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
    .env("OPENHUMAN_STORAGE_URL", url)
    .env("OPENHUMAN_MODULE_PATH", modules)
    // A shared backend keeps secrets under the keyring master key. CI has no OS
    // keychain, so both processes get the same fixed key, as a headless
    // deployment would.
    .env("OPENHUMAN_KEYRING_MASTER_KEY", "44".repeat(32))
    .env_remove("OPENHUMAN_KEYRING_MASTER_KEY_FILE")
    .env_remove("OPENHUMAN_MODE")
    // Only the module under test: the others would be fetched from the release
    // host, and this test never reaches the network.
    // Only the module under test: the others would be fetched from the release
    // host, and this test never reaches the network.
    .envs(
        [
            "SANDBOX",
            "CONNECTORS",
            "CHANNELS",
            "HOSTING",
            "MEMES",
            "COMPUTER",
            "SEARCH",
            "DOCUMENTS",
            "WALLET",
            "JUICE",
            "VOICE",
        ]
        .map(|name| (format!("OPENHUMAN_COMPONENT_{name}"), "0")),
    )
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    let mut server = Server(cmd.spawn().expect("spawn openhuman-core"));
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::blocking::Client::new();
    let deadline = Instant::now() + Duration::from_secs(180);
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
    let response: Value = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap()
        .post(format!("{base}/rpc"))
        .bearer_auth(TOKEN)
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
        .send()
        .expect("POST /rpc")
        .json()
        .expect("json-rpc response");
    assert!(
        response.get("error").is_none(),
        "{method} failed: {response}"
    );
    response
}

/// Every regular file under `dir`, relative.
fn files(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                out.push(path.strip_prefix(root).unwrap().display().to_string());
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[test]
fn a_server_added_through_the_released_module_survives_a_restart_in_the_storage_backend() {
    let artifact = openhuman_core::modules::local_artifact("tinymcp")
        .expect("build or stage tinymcp (OPENHUMAN_MODULE_SOURCE_DIR / OPENHUMAN_MODULE_PATH)");
    let modules = artifact.parent().expect("artifact directory").to_path_buf();

    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let storage = tmp.path().join("storage");
    // `scripts/e2e-core.sh` points the run at a backend with OPENHUMAN_STORAGE_URL
    // (sqlite or a MongoDB replica set); on its own the test uses a sqlite file.
    let inherited = std::env::var("OPENHUMAN_STORAGE_URL")
        .ok()
        .filter(|url| !url.trim().is_empty());
    let url = inherited
        .clone()
        .unwrap_or_else(|| format!("sqlite:{}", storage.display()));

    // First process: add a stdio server (disabled, so nothing is dialled) with
    // an env secret.
    {
        let (_server, base) = start(tmp.path(), &workspace, &url, &modules);
        rpc(
            &base,
            "openhuman.mcp_clients_config_set",
            json!({
                "mcpServers": {
                    SERVER: {
                        "command": env!("CARGO_BIN_EXE_test-mcp-stub"),
                        "args": [],
                        "env": { "API_KEY": SECRET },
                        "enabled": false
                    }
                }
            }),
        );
        let listed = rpc(&base, "openhuman.mcp_clients_installed_list", json!({})).to_string();
        assert!(listed.contains(SERVER), "added server is listed: {listed}");
    }

    // Second process on the same workspace and URL.
    {
        let (_server, base) = start(tmp.path(), &workspace, &url, &modules);
        let listed = rpc(&base, "openhuman.mcp_clients_installed_list", json!({})).to_string();
        assert!(
            listed.contains(SERVER),
            "the server is still listed after a restart: {listed}"
        );
        let status = rpc(&base, "openhuman.mcp_clients_status", json!({})).to_string();
        assert!(status.contains(SERVER), "status lists it: {status}");
        let document = rpc(&base, "openhuman.mcp_clients_config_get", json!({})).to_string();
        assert!(document.contains(SERVER), "mcp.json renders it: {document}");
        assert!(document.contains("API_KEY"), "the env key survives");
        assert!(
            !document.contains(SECRET) && !listed.contains(SECRET) && !status.contains(SECRET),
            "env values are write-only over RPC"
        );
    }

    // The secret is in the backend, encrypted: not in clear text in any file.
    for file in files(tmp.path()) {
        let bytes = std::fs::read(tmp.path().join(&file)).unwrap();
        assert!(
            !bytes
                .windows(SECRET.len())
                .any(|window| window == SECRET.as_bytes()),
            "{file} holds the env secret in clear text"
        );
    }
    // The module keeps no SQLite file of its own any more.
    let leftovers: Vec<_> = files(&workspace)
        .into_iter()
        .filter(|file| file.starts_with("mcp_clients/") || file.starts_with("mcp_audit/"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "legacy module databases were created: {leftovers:?}"
    );
    if inherited.is_none() {
        assert!(
            storage.exists(),
            "the server record lives in the configured backend"
        );
    }
}
