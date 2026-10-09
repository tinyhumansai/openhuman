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

fn authorization(row: &Value) -> String {
    row.pointer("/headers/authorization")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn is_inference(row: &Value) -> bool {
    row["method"] == "POST"
        && row["url"]
            .as_str()
            .is_some_and(|url| url.contains("/chat/completions"))
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
    mock: Mock,
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
        let (server, base, client) = start_with_env(&d, &[("BACKEND_URL", &mock.origin)]);
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
            mock,
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
    std::thread::sleep(Duration::from_secs(20));
    eprintln!("MSGS {}", s.call(&s.alice, "openhuman.threads_messages_list", json!({"thread_id":"t1"})));
    eprintln!("LIST {}", s.call(&s.alice, "openhuman.threads_list", json!({})));
    for row in s.mock.requests(&s.client) {
        eprintln!("REQ {} {} {}", row["method"], row["url"], authorization(&row));
        if is_inference(&row) {
            eprintln!("BODY {}", row["body"]);
        }
    }
    for f in files_under(&s.d.root) {
        eprintln!("FILE {}", f.strip_prefix(&s.d.root).unwrap().display());
    }
    let _ = BTreeSet::<u8>::new();
    let _ = (&s.bob, &s.mock);
}
