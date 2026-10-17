//! Helpers shared by the SaaS end-to-end suites (`saas_mode_e2e`,
//! `saas_profile_isolation_e2e`): a deployment on a temp root, a real
//! `openhuman-core run --mode saas` child, signed gateway calls for a user,
//! their `/events` stream, and fake backends.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

pub const BEARER: &str = "saas-e2e-gateway-bearer-0123456789abcdef";
pub const TEST_MASTER_KEY: &str =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Environment a developer machine may carry that would point the child at a
/// single user, or that the boot guard refuses outright.
pub const SCRUBBED_ENV: &[&str] = &[
    "OPENHUMAN_WORKSPACE",
    "OPENHUMAN_DEV_CONNECT",
    "OPENHUMAN_BACKEND_SESSION_TOKEN",
    "OPENHUMAN_BACKEND_API_KEY",
    "OPENHUMAN_KEYRING_MASTER_KEY",
    "OPENHUMAN_KEYRING_MASTER_KEY_FILE",
    "OPENHUMAN_CORE_TOKEN",
    "OPENHUMAN_APPROVAL_GATE",
    "OPENHUMAN_SANDBOX",
    "OPENHUMAN_MODE",
    "OPENHUMAN_STORAGE_URL",
    "OPENHUMAN_NODE_ID",
    "BACKEND_URL",
    "VITE_BACKEND_URL",
];

pub struct Deployment {
    pub tmp: tempfile::TempDir,
    pub root: PathBuf,
    pub config: PathBuf,
}

pub fn deployment(write_token: bool) -> Deployment {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("saas");
    std::fs::create_dir(&root).unwrap();
    set_mode(&root, 0o700);
    if write_token {
        let token = root.join("service.token");
        std::fs::write(&token, format!("{BEARER}\n")).unwrap();
        set_mode(&token, 0o600);
    }
    let config = tmp.path().join("operator.toml");
    std::fs::write(
        &config,
        format!("root = {:?}\n", root.display().to_string()),
    )
    .unwrap();
    Deployment { tmp, root, config }
}

#[cfg(unix)]
pub fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(not(unix))]
pub fn set_mode(_: &Path, _: u32) {}

pub fn core_command(d: &Deployment, extra: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_openhuman-core"));
    cmd.args(["run", "--mode", "saas", "--saas-config"])
        .arg(&d.config)
        .args(extra);
    for var in SCRUBBED_ENV {
        cmd.env_remove(var);
    }
    // Keep the child away from the developer's real `~/.openhuman`, and give
    // each temporary deployment a deterministic host key. SaaS profile tests
    // exercise secret storage without an RPC host installing an OS keyring.
    cmd.env("HOME", d.tmp.path())
        .env("USERPROFILE", d.tmp.path())
        .env("OPENHUMAN_KEYRING_BACKEND", "file")
        .env("OPENHUMAN_KEYRING_MASTER_KEY", TEST_MASTER_KEY);
    cmd
}

pub struct Server(pub Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

pub fn rpc(
    client: &reqwest::blocking::Client,
    base: &str,
    bearer: Option<&str>,
    method: &str,
) -> (u16, Value) {
    rpc_with(client, base, bearer, method, json!({}))
}

pub fn rpc_with(
    client: &reqwest::blocking::Client,
    base: &str,
    bearer: Option<&str>,
    method: &str,
    params: Value,
) -> (u16, Value) {
    let mut request = client.post(format!("{base}/rpc")).json(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params
    }));
    if let Some(bearer) = bearer {
        request = request.bearer_auth(bearer);
    }
    let response = request.send().expect("POST /rpc");
    let status = response.status().as_u16();
    (status, response.json().unwrap_or(Value::Null))
}

/// What answered `/health` on the port a core was told to listen on.
#[derive(Debug, PartialEq, Eq)]
pub enum Serving {
    /// The core this harness spawned.
    Ours,
    /// Another process (pid) holds the port. `--port` is only a preference:
    /// a core that finds it taken moves to a fallback port
    /// (`OccupiedByCore::Fallback`), so talking to `base` would drive a
    /// different deployment's core.
    Taken(Option<u64>),
}

/// Wait until `base` answers `/health`, and say whether the answer came from
/// `server` (by the pid the health snapshot reports).
pub fn wait_until_serving(
    client: &reqwest::blocking::Client,
    base: &str,
    server: &mut Server,
    deadline: Instant,
) -> Result<Serving, String> {
    let own = u64::from(server.0.id());
    loop {
        if let Ok(response) = client.get(format!("{base}/health")).send() {
            if response.status().is_success() {
                let body: Value = response.json().unwrap_or(Value::Null);
                let pid = body.get("pid").and_then(Value::as_u64);
                return Ok(if pid == Some(own) {
                    Serving::Ours
                } else {
                    Serving::Taken(pid)
                });
            }
        }
        if let Ok(Some(status)) = server.0.try_wait() {
            return Err(format!("SaaS core exited before serving: {status}"));
        }
        if Instant::now() >= deadline {
            return Err("SaaS core never became healthy".to_string());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Spawn a core with the command `command(port)` builds and wait until it
/// serves on that port. [`free_port`] only finds a port that was free a moment
/// ago: a parallel test's core (or any socket) can take it before this core
/// binds, and the core then silently listens elsewhere. When another process
/// answers, this core is killed and started again on a fresh port.
pub fn spawn_core(command: impl FnMut(u16) -> Command) -> (Server, String) {
    spawn_core_with(command, String::new)
}

/// [`spawn_core`], appending `context()` (a log tail) to a startup failure.
pub fn spawn_core_with(
    mut command: impl FnMut(u16) -> Command,
    context: impl Fn() -> String,
) -> (Server, String) {
    const ATTEMPTS: usize = 5;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    for attempt in 1..=ATTEMPTS {
        let port = free_port();
        let child = command(port).spawn().expect("spawn openhuman-core");
        let mut server = Server(child);
        let base = format!("http://127.0.0.1:{port}");
        let deadline = Instant::now() + Duration::from_secs(120);
        match wait_until_serving(&client, &base, &mut server, deadline) {
            Ok(Serving::Ours) => return (server, base),
            Ok(Serving::Taken(pid)) => {
                eprintln!(
                    "[saas-e2e] port {port} is held by pid {pid:?}, not our core; retrying ({attempt}/{ATTEMPTS})"
                );
            }
            Err(error) => panic!("{error}\n{}", context()),
        }
    }
    panic!("no free port held for a SaaS core after {ATTEMPTS} attempts");
}

/// Start a SaaS core on deployment `d` and wait until it is healthy.
pub fn start(d: &Deployment) -> (Server, String, reqwest::blocking::Client) {
    let (server, base) = spawn_core(|port| {
        let mut cmd = core_command(d, &["--port", &port.to_string()]);
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        cmd
    });
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    (server, base, client)
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// POST /rpc for gateway user `user`, signed unless `sig` overrides it.
pub fn user_rpc(
    client: &reqwest::blocking::Client,
    base: &str,
    bearer: &str,
    user: &str,
    sig: Option<&str>,
    method: &str,
) -> (u16, Value) {
    user_rpc_with(client, base, bearer, user, sig, method, json!({}))
}

pub fn user_rpc_with(
    client: &reqwest::blocking::Client,
    base: &str,
    bearer: &str,
    user: &str,
    sig: Option<&str>,
    method: &str,
    params: Value,
) -> (u16, Value) {
    use openhuman_core::profiles::gateway::{sign, USER_HEADER, USER_SIG_HEADER};
    let signature = sig
        .map(str::to_owned)
        .unwrap_or_else(|| sign(BEARER, user, now()));
    let response = client
        .post(format!("{base}/rpc"))
        .bearer_auth(bearer)
        .header(USER_HEADER, user)
        .header(USER_SIG_HEADER, signature)
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
        .send()
        .expect("POST /rpc");
    let status = response.status().as_u16();
    (status, response.json().unwrap_or(Value::Null))
}

pub fn provision(client: &reqwest::blocking::Client, base: &str, user: &str) -> String {
    let (_, body) = rpc_with(
        client,
        base,
        Some(BEARER),
        "openhuman.profiles_provision",
        json!({ "user_id": user }),
    );
    assert!(body.get("result").is_some(), "provision {user}: {body}");
    openhuman_core::profiles::ProfileId::for_user(
        user,
        openhuman_core::profiles::ProfileIdMode::Raw,
    )
    .unwrap()
    .to_string()
}

pub fn thread_ids(body: &Value) -> Vec<String> {
    let text = body.to_string();
    let mut ids = Vec::new();
    for part in text.split("\"id\":\"").skip(1) {
        if let Some(end) = part.find('"') {
            ids.push(part[..end].to_string());
        }
    }
    ids
}

/// Open `/events?client_id=` for `user` and forward each SSE `data:` line.
pub fn user_events(base: &str, user: &str, client_id: &str) -> std::sync::mpsc::Receiver<String> {
    use openhuman_core::profiles::gateway::{sign, USER_HEADER, USER_SIG_HEADER};
    use std::io::BufRead;
    let (tx, rx) = std::sync::mpsc::channel();
    let url = format!("{base}/events?client_id={client_id}");
    let user = user.to_string();
    std::thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(None)
            .build()
            .unwrap();
        let Ok(response) = client
            .get(&url)
            .bearer_auth(BEARER)
            .header(USER_HEADER, &user)
            .header(USER_SIG_HEADER, sign(BEARER, &user, now()))
            .send()
        else {
            return;
        };
        let _ = tx.send(format!("status:{}", response.status().as_u16()));
        for line in std::io::BufReader::new(response).lines() {
            let Ok(line) = line else { break };
            if let Some(data) = line.strip_prefix("data:") {
                if tx.send(data.trim().to_string()).is_err() {
                    break;
                }
            }
        }
    });
    rx
}

/// A fake backend: answers every request `500` and reports each request's
/// path and `Authorization` header.
pub fn recording_backend() -> (u16, std::sync::mpsc::Receiver<(String, String)>) {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    return;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                let mut auth = String::new();
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':') {
                        if name.eq_ignore_ascii_case("authorization") {
                            auth = value.trim().to_string();
                        }
                    }
                }
                let _ = tx.send((path, auth));
                let mut stream = stream;
                let _ = stream.write_all(
                    b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                );
            });
        }
    });
    (port, rx)
}

/// Start a SaaS core whose backend is a closed port, so every turn fails fast
/// and its failure is itself the reply, without any real inference.
pub fn start_offline(d: &Deployment) -> (Server, String, reqwest::blocking::Client) {
    let (server, base) = spawn_core(|port| {
        let mut cmd = core_command(d, &["--port", &port.to_string()]);
        cmd.env("BACKEND_URL", "http://127.0.0.1:9")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd
    });
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    (server, base, client)
}

#[path = "node.rs"]
mod node;
pub use node::*;
