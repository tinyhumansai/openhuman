//! One `openhuman-core` node of a deployment with its own operator file,
//! port and log, and signed calls against it (the cluster suites and the
//! isolation suite).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::*;

/// The last lines of a core's log, without the polling RPCs and without lines
/// that may carry a credential.
pub fn log_tail_of(path: &std::path::Path) -> String {
    let log = std::fs::read_to_string(path).unwrap_or_default();
    // The polling RPCs drown everything else out.
    let lines: Vec<&str> = log
        .lines()
        .filter(|line| !line.contains("rpc_handler [rpc]"))
        .collect();
    // A failure trace is shared output: drop lines that may carry a
    // credential and cap the rest.
    lines[lines.len().saturating_sub(80)..]
        .iter()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if [
                "bearer",
                "token",
                "authorization",
                "secret",
                "password",
                "canary",
            ]
            .iter()
            .any(|k| lower.contains(k))
            {
                "[redacted log line]".to_string()
            } else {
                line.chars().take(300).collect()
            }
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// One core of a deployment.
pub struct Node {
    pub name: String,
    pub server: Server,
    pub base: String,
    pub log: PathBuf,
}

impl Node {
    pub fn log_tail(&self) -> String {
        log_tail_of(&self.log)
    }

    pub fn kill(&mut self) {
        // SIGKILL on Unix: no shutdown hook runs, no lease is released.
        self.server.0.kill().expect("kill core");
        let _ = self.server.0.wait();
    }
}

/// Write node `name`'s operator file on deployment `d` and start it.
pub fn start_node(
    d: &Deployment,
    name: &str,
    storage_url: Option<&str>,
    extra: &str,
    backend: Option<u16>,
) -> Node {
    start_node_logging(
        d,
        name,
        storage_url,
        extra,
        backend,
        "info,openhuman_core::storage::lease=debug",
    )
}

/// [`start_node`] with the child's `RUST_LOG`.
pub fn start_node_logging(
    d: &Deployment,
    name: &str,
    storage_url: Option<&str>,
    extra: &str,
    backend: Option<u16>,
    rust_log: &str,
) -> Node {
    let log = d.tmp.path().join(format!("core-{name}.log"));
    let config_path = d.tmp.path().join(format!("operator-{name}.toml"));
    let (server, base) = spawn_core_with(
        |port| {
            // The operator file names the port (`advertise_url`), so it is
            // written again for each port tried.
            let base = format!("http://127.0.0.1:{port}");
            let mut config = format!(
                "root = {:?}\nnode_id = \"core-{name}\"\noperator_dir = {:?}\nlease_ttl_secs = 3\n",
                d.root.display().to_string(),
                d.tmp
                    .path()
                    .join(format!("operator-{name}"))
                    .display()
                    .to_string(),
            );
            if let Some(url) = storage_url {
                config.push_str(&format!(
                    "storage_url = {url:?}\nadvertise_url = {base:?}\n"
                ));
            }
            config.push_str(extra);
            std::fs::write(&config_path, config).unwrap();

            let mut cmd = Command::new(env!("CARGO_BIN_EXE_openhuman-core"));
            cmd.args(["run", "--mode", "saas", "--saas-config"])
                .arg(&config_path)
                .args(["--port", &port.to_string()])
                .env("HOME", d.tmp.path())
                .env("USERPROFILE", d.tmp.path())
                .env("RUST_LOG", rust_log)
                .stdout(Stdio::null())
                .stderr(std::fs::File::create(&log).unwrap());
            for var in SCRUBBED_ENV {
                cmd.env_remove(var);
            }
            cmd.env("OPENHUMAN_KEYRING_BACKEND", "file")
                .env("OPENHUMAN_KEYRING_MASTER_KEY", TEST_MASTER_KEY);
            match backend {
                Some(port) => cmd.env("BACKEND_URL", format!("http://127.0.0.1:{port}")),
                None => cmd.env("BACKEND_URL", "http://127.0.0.1:9"),
            };
            cmd
        },
        || format!("core {name}:\n{}", log_tail_of(&log)),
    );
    Node {
        name: name.to_string(),
        server,
        base,
        log,
    }
}

pub fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(45))
        .build()
        .unwrap()
}

/// POST /rpc for `user` on `node`: status, `X-OpenHuman-Profile-Owner`, body.
pub fn call(node: &Node, user: &str, method: &str, params: Value) -> (u16, Option<String>, Value) {
    use openhuman_core::profiles::gateway::{sign, USER_HEADER, USER_SIG_HEADER};
    let response = client()
        .post(format!("{}/rpc", node.base))
        .bearer_auth(BEARER)
        .header(USER_HEADER, user)
        .header(USER_SIG_HEADER, sign(BEARER, user, now()))
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
        .send()
        .unwrap_or_else(|error| {
            panic!(
                "POST /rpc method={method} profile={user}: {error}\ncore {} log:\n{}",
                node.name,
                node.log_tail()
            )
        });
    let status = response.status().as_u16();
    let owner = response
        .headers()
        .get("x-openhuman-profile-owner")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    (status, owner, response.json().unwrap_or(Value::Null))
}

pub fn operator(node: &Node, method: &str, params: Value) -> Value {
    let (_, body) = rpc_with(&client(), &node.base, Some(BEARER), method, params);
    body
}

/// Provision `user` (through `node`) with a session credential, so their
/// turns reach inference.
pub fn provision_with_credential(node: &Node, user: &str) -> String {
    let profile = provision(&client(), &node.base, user);
    let body = operator(
        node,
        "openhuman.profiles_set_credential",
        json!({ "profile_id": profile, "kind": "session", "token": format!("{user}-jwt") }),
    );
    assert!(body.get("result").is_some(), "{body}");
    profile
}

pub fn start_turn(node: &Node, user: &str, thread: &str) -> Value {
    let (status, _, body) = call(
        node,
        user,
        "openhuman.channel_web_chat",
        json!({ "client_id": "c1", "thread_id": thread, "message": "hello" }),
    );
    assert_eq!(status, 200, "{user} starts a turn on {thread}: {body}");
    assert!(body.get("result").is_some(), "{body}");
    body
}

/// Whether `user`'s turn on `thread` is in flight on `node`.
pub fn active(node: &Node, user: &str, thread: &str) -> bool {
    let (status, _, body) = call(
        node,
        user,
        "openhuman.channel_web_queue_status",
        json!({ "thread_id": thread }),
    );
    status == 200 && body.to_string().contains("\"active\":true")
}

pub fn wait_until(what: &str, node: &Node, timeout: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !done() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}; core {} log:\n{}",
            node.name,
            node.log_tail()
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The lifecycle of `user`'s latest turn snapshot on `thread`, if any.
pub fn turn_state(node: &Node, user: &str, thread: &str) -> Option<String> {
    let (status, _, body) = call(
        node,
        user,
        "openhuman.threads_turn_state_get",
        json!({ "thread_id": thread }),
    );
    assert_eq!(status, 200, "{body}");
    body.pointer("/result/data/turnState/lifecycle")
        .or_else(|| body.pointer("/result/result/data/turnState/lifecycle"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// The first value under `key` anywhere in `value`.
pub fn find_key<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => map
            .get(key)
            .or_else(|| map.values().find_map(|v| find_key(v, key))),
        Value::Array(items) => items.iter().find_map(|v| find_key(v, key)),
        _ => None,
    }
}
