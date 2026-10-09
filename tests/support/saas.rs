//! Shared fixtures for the SaaS-mode suites (`saas_mode_e2e`, `saas_isolation`):
//! an operator deployment on disk, a spawned `openhuman-core run --mode saas`,
//! operator-plane RPC under the service bearer, and gateway RPC signed for one
//! user.
//!
//! Include with `#[path = "support/saas.rs"] mod saas;`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

pub const BEARER: &str = "saas-e2e-gateway-bearer-0123456789abcdef";

/// Environment a developer machine may carry that would point the child at a
/// single user, or that the boot guard refuses outright.
const SCRUBBED_ENV: &[&str] = &[
    "OPENHUMAN_WORKSPACE",
    "OPENHUMAN_DEV_CONNECT",
    "OPENHUMAN_BACKEND_SESSION_TOKEN",
    "OPENHUMAN_BACKEND_API_KEY",
    "OPENHUMAN_CORE_TOKEN",
    "OPENHUMAN_APPROVAL_GATE",
    "OPENHUMAN_SANDBOX",
    "OPENHUMAN_MODE",
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
        .args(extra)
        // Keep the child away from the developer's real `~/.openhuman`.
        .env("HOME", d.tmp.path())
        .env("USERPROFILE", d.tmp.path());
    for var in SCRUBBED_ENV {
        cmd.env_remove(var);
    }
    cmd
}

/// A spawned child, killed when dropped.
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

/// Start a SaaS core on deployment `d` and wait until it is healthy.
pub fn start(d: &Deployment) -> (Server, String, reqwest::blocking::Client) {
    start_with_env(d, &[])
}

/// [`start`] with extra environment for the child (`BACKEND_URL`, ...).
pub fn start_with_env(
    d: &Deployment,
    envs: &[(&str, &str)],
) -> (Server, String, reqwest::blocking::Client) {
    let port = free_port();
    let mut cmd = core_command(d, &["--port", &port.to_string()]);
    cmd.envs(envs.iter().copied())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = cmd.spawn().expect("spawn openhuman-core");
    let mut server = Server(child);
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Ok(response) = client.get(format!("{base}/health")).send() {
            if response.status().is_success() {
                break;
            }
        }
        if let Ok(Some(status)) = server.0.try_wait() {
            panic!("SaaS core exited before serving: {status}");
        }
        assert!(Instant::now() < deadline, "SaaS core never became healthy");
        std::thread::sleep(Duration::from_millis(250));
    }
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
    use openhuman_core::user_agents::gateway::{sign, USER_HEADER, USER_SIG_HEADER};
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

/// `GET path` for gateway user `user` (signed), e.g. `/schema`.
pub fn user_get(
    client: &reqwest::blocking::Client,
    base: &str,
    user: &str,
    path: &str,
) -> reqwest::blocking::Response {
    use openhuman_core::user_agents::gateway::{sign, USER_HEADER, USER_SIG_HEADER};
    client
        .get(format!("{base}{path}"))
        .bearer_auth(BEARER)
        .header(USER_HEADER, user)
        .header(USER_SIG_HEADER, sign(BEARER, user, now()))
        .send()
        .expect("GET")
}

/// Provision `user` through the operator plane and return their agent id.
pub fn provision(client: &reqwest::blocking::Client, base: &str, user: &str) -> String {
    let (_, body) = rpc_with(
        client,
        base,
        Some(BEARER),
        "openhuman.user_agents_provision",
        json!({ "user_id": user }),
    );
    assert!(body.get("result").is_some(), "provision {user}: {body}");
    openhuman_core::user_agents::UserAgentId::for_user(user)
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
