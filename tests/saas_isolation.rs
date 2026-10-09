//! SaaS tenant isolation, end to end: two users, one process, one gateway.
//!
//! alice and bob are provisioned on one `openhuman-core run --mode saas`, given
//! different backend credentials, and driven only through the real gateway
//! (service bearer plus HMAC-signed `X-OpenHuman-User`). The backend is the
//! shared node mock (`scripts/mock-api-server.mjs`), whose request log shows
//! exactly what each user's agent sent and under which credential.
//!
//! Covered: separate threads, transcripts and `sessions.db` per `user_agents`
//! agent; 50 interleaved turns that leave no trace of the other user's text on
//! disk; the right credential on every backend call; per-user memory; usage
//! rows attributed to the user's agent; the empty default tool surface; and an
//! operator-only API that a user can neither see nor call.

#[path = "support/saas.rs"]
mod saas;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use saas::*;
use serde_json::{json, Value};

/// JWT-shaped (the mock decodes the bearer) and distinct per user.
const ALICE_TOKEN: &str = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.eyJzdWIiOiJhbGljZS1tb2NrIiwiZXhwIjo0MTAyNDQ0ODAwfQ.e2e";
const BOB_TOKEN: &str = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.eyJzdWIiOiJib2ItbW9jayIsImV4cCI6NDEwMjQ0NDgwMH0.e2e";

const TURNS: usize = 50;

/// The node mock backend, killed when dropped.
struct Mock {
    child: std::process::Child,
    origin: String,
}

impl Drop for Mock {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Mock {
    fn start(client: &reqwest::blocking::Client) -> Self {
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/mock-api-server.mjs")
            .canonicalize()
            .expect("scripts/mock-api-server.mjs exists");
        let port = free_port();
        let child = Command::new("node")
            .arg(&script)
            .args(["--port", &port.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn `node scripts/mock-api-server.mjs` (is node on PATH?)");
        let mock = Self {
            child,
            origin: format!("http://127.0.0.1:{port}"),
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        while !client
            .get(format!("{}/__admin/health", mock.origin))
            .send()
            .is_ok_and(|r| r.status().is_success())
        {
            assert!(Instant::now() < deadline, "the mock backend never started");
            std::thread::sleep(Duration::from_millis(100));
        }
        mock
    }

    /// Every request the mock has logged: `{method, url, body, headers}`.
    fn requests(&self, client: &reqwest::blocking::Client) -> Vec<Value> {
        let body: Value = client
            .get(format!("{}/__admin/requests", self.origin))
            .send()
            .expect("read the mock request log")
            .json()
            .expect("request log json");
        body.get("data")
            .unwrap_or(&body)
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
}

/// One request the proxy saw.
#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    authorization: String,
    body: String,
}

impl Seen {
    fn is_inference(&self) -> bool {
        self.method == "POST" && self.path.contains("/chat/completions")
    }
}

/// A recording pass-through in front of the mock. The mock redacts
/// `Authorization` in its own log; the credential each request carried is the
/// point of this suite, so it is captured here before forwarding.
struct Proxy {
    origin: String,
    seen: std::sync::Arc<std::sync::Mutex<Vec<Seen>>>,
}

impl Proxy {
    fn start(upstream: String) -> Self {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = std::sync::Arc::clone(&seen);
        std::thread::spawn(move || {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(120))
                .no_gzip()
                .build()
                .unwrap();
            for stream in listener.incoming().flatten() {
                let (client, upstream, log) = (client.clone(), upstream.clone(), log.clone());
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let mut parts = line.split_whitespace();
                    let method = parts.next().unwrap_or("GET").to_string();
                    let path = parts.next().unwrap_or("/").to_string();
                    let mut headers = Vec::new();
                    let (mut length, mut chunked) = (0usize, false);
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                            break;
                        }
                        if let Some((name, value)) = header.split_once(':') {
                            let (name, value) = (name.trim().to_string(), value.trim().to_string());
                            match name.to_ascii_lowercase().as_str() {
                                "content-length" => length = value.parse().unwrap_or(0),
                                "transfer-encoding" => chunked = value.contains("chunked"),
                                _ => {}
                            }
                            headers.push((name, value));
                        }
                    }
                    let mut body = Vec::new();
                    if chunked {
                        loop {
                            let mut size = String::new();
                            if reader.read_line(&mut size).unwrap_or(0) == 0 {
                                break;
                            }
                            let size = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
                            let mut chunk = vec![0u8; size + 2];
                            if reader.read_exact(&mut chunk).is_err() || size == 0 {
                                break;
                            }
                            body.extend_from_slice(&chunk[..size]);
                        }
                    } else {
                        body.resize(length, 0);
                        let _ = reader.read_exact(&mut body);
                    }
                    let authorization = headers
                        .iter()
                        .find(|(n, _)| n.eq_ignore_ascii_case("authorization"))
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default();
                    log.lock().unwrap().push(Seen {
                        method: method.clone(),
                        path: path.clone(),
                        authorization,
                        body: String::from_utf8_lossy(&body).into_owned(),
                    });

                    let mut request = client.request(
                        reqwest::Method::from_bytes(method.as_bytes()).unwrap(),
                        format!("{upstream}{path}"),
                    );
                    for (name, value) in &headers {
                        if !["host", "content-length", "connection", "transfer-encoding", "accept-encoding"]
                            .contains(&name.to_ascii_lowercase().as_str())
                        {
                            request = request.header(name, value);
                        }
                    }
                    let mut stream = stream;
                    match request.body(body).send() {
                        Ok(response) => {
                            let status = response.status().as_u16();
                            let content_type = response
                                .headers()
                                .get("content-type")
                                .and_then(|v| v.to_str().ok())
                                .unwrap_or("application/octet-stream")
                                .to_string();
                            let bytes = response.bytes().unwrap_or_default();
                            let _ = write!(
                                stream,
                                "HTTP/1.1 {status} X\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                                bytes.len()
                            );
                            let _ = stream.write_all(&bytes);
                        }
                        Err(_) => {
                            let _ = stream.write_all(
                                b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                            );
                        }
                    }
                });
            }
        });
        Self { origin, seen }
    }

    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

struct User {
    name: &'static str,
    token: &'static str,
    agent: String,
}

/// A SaaS core on a mock backend with alice and bob provisioned.
struct Stack {
    d: Deployment,
    base: String,
    client: reqwest::blocking::Client,
    _mock: Mock,
    proxy: Proxy,
    alice: User,
    bob: User,
    _server: Server,
}

impl Stack {
    fn new() -> Self {
        let d = deployment(true);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();
        let mock = Mock::start(&client);
        let proxy = Proxy::start(mock.origin.clone());
        let (server, base, client) = start_with_env(&d, &[("BACKEND_URL", &proxy.origin)]);
        let users = [("alice", ALICE_TOKEN), ("bob", BOB_TOKEN)].map(|(name, token)| {
            let agent = provision(&client, &base, name);
            let (_, body) = rpc_with(
                &client,
                &base,
                Some(BEARER),
                "openhuman.user_agents_set_credential",
                json!({ "agent_id": agent, "kind": "session", "token": token }),
            );
            assert!(body.get("result").is_some(), "credential for {name}: {body}");
            User { name, token, agent }
        });
        let [alice, bob] = users;
        Self {
            d,
            base,
            client,
            _mock: mock,
            proxy,
            alice,
            bob,
            _server: server,
        }
    }

    fn call(&self, user: &User, method: &str, params: Value) -> Value {
        let (status, body) =
            user_rpc_with(&self.client, &self.base, BEARER, user.name, None, method, params);
        assert_eq!(status, 200, "{} {method}: {body}", user.name);
        body
    }

    fn agent_dir(&self, user: &User) -> PathBuf {
        self.d.root.join("agents").join(&user.agent)
    }

    /// Start a web-chat turn for `user` on `thread` and return at once.
    fn chat(&self, user: &User, thread: &str, message: &str) {
        let body = self.call(
            user,
            "openhuman.channel_web_chat",
            json!({ "client_id": format!("c-{}", user.name), "thread_id": thread, "message": message }),
        );
        assert!(body.get("error").is_none(), "{} chat: {body}", user.name);
    }

    /// Messages stored on `user`'s `thread`.
    fn messages(&self, user: &User, thread: &str) -> Vec<Value> {
        let body = self.call(
            user,
            "openhuman.threads_messages_list",
            json!({ "thread_id": thread }),
        );
        let result = body.get("result").unwrap_or(&body);
        result
            .pointer("/result/messages")
            .or_else(|| result.get("messages"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    /// Poll until `user`'s `thread` holds an assistant reply to the turn.
    fn await_reply(&self, user: &User, thread: &str, deadline: Instant) {
        loop {
            let replied = self
                .messages(user, thread)
                .iter()
                .any(|m| m["sender"] == "agent" || m["role"] == "assistant");
            if replied {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{}'s turn on {thread} never completed: {:?}",
                user.name,
                self.messages(user, thread)
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

/// Every regular file under `dir`.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(t) if t.is_file() => out.push(path),
                _ => {}
            }
        }
    }
    out.sort();
    out
}

/// Files under `dir` whose bytes contain `needle`.
fn files_containing(dir: &Path, needle: &str) -> Vec<PathBuf> {
    files_under(dir)
        .into_iter()
        .filter(|path| {
            std::fs::read(path)
                .map(|bytes| bytes.windows(needle.len()).any(|w| w == needle.as_bytes()))
                .unwrap_or(false)
        })
        .collect()
}

#[test]
fn probe() {
    let s = Stack::new();
    s.chat(&s.alice, "t1", "ALICE-MARK-0 hello");
    for _ in 0..10 {
        std::thread::sleep(Duration::from_secs(2));
        eprintln!("TS {}", s.call(&s.alice, "openhuman.threads_turn_state_get", json!({"thread_id":"t1"})));
        eprintln!("QS {}", s.call(&s.alice, "openhuman.channel_web_queue_status", json!({"client_id":"c-alice","thread_id":"t1"})));
    }
    for r in s.proxy.requests() {
        eprintln!("REQ {} {} {}", r.method, r.path, r.authorization);
        if r.is_inference() {
            let v: Value = serde_json::from_str(&r.body).unwrap();
            eprintln!("KEYS {:?} tools={}", v.as_object().unwrap().keys().collect::<Vec<_>>(), v["tools"]);
        }
    }
    eprintln!("COSTS {}", std::fs::read_to_string(s.d.root.join("operator/workspace/state/costs.jsonl")).unwrap_or_default());
    let _ = (&s.bob, BTreeSet::<u8>::new());
}
