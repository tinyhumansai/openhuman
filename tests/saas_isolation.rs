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
#[path = "support/saas_backend.rs"]
mod saas_backend;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use saas::*;
use saas_backend::{Mock, Proxy, Seen};
use serde_json::{json, Value};
/// A JWT-shaped session token (the mock decodes the bearer), distinct per user.
/// Built at run time so no token literal sits in the source.
fn token_for(user: &str) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
    use base64::Engine;
    let claims = json!({ "sub": format!("{user}-mock"), "exp": 4_102_444_800u64 });
    format!(
        "{}.{}.e2e",
        B64.encode(r#"{"alg":"none","typ":"JWT"}"#),
        B64.encode(claims.to_string())
    )
}

const TURNS: usize = 50;

struct User {
    name: &'static str,
    token: String,
    agent: String,
}

impl User {
    fn bearer(&self) -> String {
        format!("Bearer {}", self.token)
    }
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
        let users = ["alice", "bob"].map(|name| {
            let token = token_for(name);
            let agent = provision(&client, &base, name);
            let (_, body) = rpc_with(
                &client,
                &base,
                Some(BEARER),
                "openhuman.user_agents_set_credential",
                json!({ "agent_id": agent, "kind": "session", "token": &token }),
            );
            assert!(
                body.get("result").is_some(),
                "credential for {name}: {body}"
            );
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

    /// An RPC as `user`, through the gateway; the JSON-RPC envelope.
    fn call(&self, user: &User, method: &str, params: Value) -> Value {
        let (status, body) = user_rpc_with(
            &self.client,
            &self.base,
            BEARER,
            user.name,
            None,
            method,
            params,
        );
        assert_eq!(status, 200, "{} {method}: {body}", user.name);
        body
    }

    fn agent_dir(&self, user: &User) -> PathBuf {
        self.d.root.join("agents").join(&user.agent)
    }

    fn operator_dir(&self) -> PathBuf {
        self.d.root.join("operator")
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

    /// Poll until the turn on `user`'s `thread` has completed.
    fn await_turn(&self, user: &User, thread: &str, deadline: Instant) {
        loop {
            let body = self.call(
                user,
                "openhuman.threads_turn_state_get",
                json!({ "thread_id": thread }),
            );
            let state = body
                .pointer("/result/data/turnState")
                .unwrap_or(&Value::Null);
            match state["lifecycle"].as_str() {
                Some("completed") => return,
                Some("failed" | "error" | "cancelled" | "interrupted") => {
                    panic!("{}'s turn on {thread} ended badly: {state}", user.name)
                }
                _ => {}
            }
            assert!(
                Instant::now() < deadline,
                "{}'s turn on {thread} never completed: {state}",
                user.name
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

/// `body` holds an RPC error that says the method does not exist.
fn is_unknown_method(body: &Value) -> bool {
    body.get("error")
        .is_some_and(|e| e.to_string().contains("unknown method"))
}

/// Tool names a model request declares.
fn tool_names(request: &Seen) -> Vec<String> {
    let body: Value = serde_json::from_str(&request.body).unwrap_or(Value::Null);
    body["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|t| t.pointer("/function/name").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn the_same_thread_id_is_two_separate_conversations() {
    let s = Stack::new();
    for user in [&s.alice, &s.bob] {
        let body = s.call(
            user,
            "openhuman.threads_upsert",
            json!({ "id": "shared", "title": format!("{}'s", user.name), "created_at": "2026-10-09T00:00:00Z" }),
        );
        assert!(body.get("result").is_some(), "{} upsert: {body}", user.name);
    }
    let deadline = Instant::now() + Duration::from_secs(120);
    s.chat(
        &s.alice,
        "shared",
        "ALICE-MARK-shared please remember apples",
    );
    s.chat(&s.bob, "shared", "BOB-MARK-shared please remember pears");
    s.await_turn(&s.alice, "shared", deadline);
    s.await_turn(&s.bob, "shared", deadline);

    for (me, other, mark, other_mark) in [
        (&s.alice, &s.bob, "ALICE-MARK-", "BOB-MARK-"),
        (&s.bob, &s.alice, "BOB-MARK-", "ALICE-MARK-"),
    ] {
        // threads_list shows each only their own `shared`.
        let list = s.call(me, "openhuman.threads_list", json!({}));
        assert_eq!(thread_ids(&list), vec!["shared".to_string()], "{list}");
        assert!(
            list.to_string().contains(&format!("{}'s", me.name)),
            "{list}"
        );
        assert!(
            !list.to_string().contains(&format!("{}'s", other.name)),
            "{} sees {}'s thread: {list}",
            me.name,
            other.name
        );

        // A transcript of their own, in their own workspace.
        let workspace = s.agent_dir(me).join("workspace");
        let transcripts: Vec<_> = files_under(&workspace.join("session_raw"))
            .into_iter()
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("shared-"))
            })
            .collect();
        assert!(
            !transcripts.is_empty(),
            "{} has no `shared` transcript under {}",
            me.name,
            workspace.display()
        );
        for transcript in &transcripts {
            let text = std::fs::read_to_string(transcript).unwrap_or_default();
            assert!(
                text.contains(mark),
                "{} transcript lacks their message",
                me.name
            );
            assert!(
                !text.contains(other_mark),
                "{}'s transcript holds {}'s message",
                me.name,
                other.name
            );
        }

        // And a session database of their own.
        let db = workspace.join("session_db").join("sessions.db");
        assert!(db.is_file(), "{}: {} missing", me.name, db.display());
    }
    assert_ne!(
        s.agent_dir(&s.alice)
            .join("workspace/session_db/sessions.db"),
        s.agent_dir(&s.bob).join("workspace/session_db/sessions.db"),
    );

    // Neither conversation reached the operator workspace.
    let strays = files_containing(&s.operator_dir(), "-MARK-");
    assert!(strays.is_empty(), "user text under operator/: {strays:?}");
}

#[test]
fn fifty_interleaved_turns_leave_each_user_only_their_own_traces() {
    let s = Stack::new();
    let deadline = Instant::now() + Duration::from_secs(300);

    // Alternate users; every message carries a marker unique to the turn.
    let mut turns: Vec<(&User, String, String)> = Vec::new();
    for n in 0..TURNS {
        let (user, tag) = if n % 2 == 0 {
            (&s.alice, "ALICE")
        } else {
            (&s.bob, "BOB")
        };
        let thread = format!("{}-{n}", user.name);
        let marker = format!("{tag}-MARK-{n}");
        s.chat(user, &thread, &format!("{marker} what is two plus two?"));
        turns.push((user, thread, marker));
    }
    for (user, thread, _) in &turns {
        s.await_turn(user, thread, deadline);
    }

    // 2. No file in either agent's home holds the other user's markers, and
    //    the operator's holds none.
    for (me, other_tag, my_tag) in [
        (&s.alice, "BOB-MARK-", "ALICE-MARK-"),
        (&s.bob, "ALICE-MARK-", "BOB-MARK-"),
    ] {
        let home = s.agent_dir(me);
        let leaked = files_containing(&home, other_tag);
        assert!(
            leaked.is_empty(),
            "{}'s home holds {other_tag}: {leaked:?}",
            me.name
        );
        assert!(
            !files_containing(&home, my_tag).is_empty(),
            "{}'s own markers are on disk (the scan is not vacuous)",
            me.name
        );
    }
    let operator = files_containing(&s.operator_dir(), "-MARK-");
    assert!(
        operator.is_empty(),
        "user text under operator/: {operator:?}"
    );

    // 3. Each model request carried the credential of the user whose text it
    //    holds, and never both users' text.
    let seen = s.proxy.requests();
    let inference: Vec<&Seen> = seen.iter().filter(|r| r.is_inference()).collect();
    let mut by_user = [0usize; 2];
    for request in &inference {
        let (alice, bob) = (
            request.body.contains("ALICE-MARK-"),
            request.body.contains("BOB-MARK-"),
        );
        assert!(!(alice && bob), "one model request holds both users' text");
        if alice {
            assert_eq!(
                request.authorization,
                s.alice.bearer(),
                "alice's text under another credential"
            );
            by_user[0] += 1;
        }
        if bob {
            assert_eq!(
                request.authorization,
                s.bob.bearer(),
                "bob's text under another credential"
            );
            by_user[1] += 1;
        }
    }
    assert!(
        by_user.iter().all(|n| *n >= TURNS / 2),
        "every turn reaches inference: {by_user:?}"
    );
    // No backend call of any kind rode a credential that is not a user's.
    for request in &seen {
        let known = request.authorization.is_empty()
            || request.authorization == s.alice.bearer()
            || request.authorization == s.bob.bearer();
        assert!(
            known,
            "{} {} used an unknown credential",
            request.method, request.path
        );
    }

    // 5. Usage rows: every turn's cost record names the user's own agent.
    let thread_owner = |thread: &str| -> Option<&User> {
        turns
            .iter()
            .find(|(_, t, _)| t == thread)
            .map(|(u, _, _)| *u)
    };
    let mut recorded: BTreeSet<String> = BTreeSet::new();
    let until = Instant::now() + Duration::from_secs(60);
    loop {
        recorded.clear();
        for path in files_under(&s.d.root)
            .into_iter()
            .filter(|p| p.file_name().is_some_and(|n| n == "costs.jsonl"))
        {
            for line in std::fs::read_to_string(&path).unwrap_or_default().lines() {
                let Ok(row) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                let scope = &row["usage"]["scope"];
                let Some(thread) = scope["thread_id"].as_str() else {
                    continue;
                };
                let Some(owner) = thread_owner(thread) else {
                    continue;
                };
                assert_eq!(
                    scope["session_agent"].as_str(),
                    Some(owner.agent.as_str()),
                    "usage for {thread} is attributed to the wrong agent: {row}"
                );
                recorded.insert(thread.to_string());
            }
        }
        if recorded.len() == turns.len() {
            break;
        }
        assert!(
            Instant::now() < until,
            "usage rows for {} of {} turns",
            recorded.len(),
            turns.len()
        );
        std::thread::sleep(Duration::from_millis(250));
    }

    // 6. Tool surface: no allowlist, so the same closed set for both users and
    //    nothing that reaches the host.
    const HOST_REACHING: &[&str] = &[
        "shell", "bash", "exec", "file", "files", "patch", "browser", "computer", "desktop",
        "terminal", "git", "mcp", "cron", "sandbox",
    ];
    let tool_sets: Vec<BTreeSet<String>> = [&s.alice, &s.bob]
        .iter()
        .map(|user| {
            let mut all = BTreeSet::new();
            for request in inference
                .iter()
                .filter(|r| r.authorization == user.bearer())
            {
                all.extend(tool_names(request));
            }
            all
        })
        .collect();
    for (user, tools) in [&s.alice, &s.bob].iter().zip(&tool_sets) {
        assert!(
            !tools.is_empty(),
            "{}'s turns declare a tool list",
            user.name
        );
        for tool in tools {
            let host = tool.split('_').any(|part| HOST_REACHING.contains(&part));
            assert!(!host, "{} is advertised a host tool: {tool}", user.name);
        }
    }
    assert_eq!(
        tool_sets[0], tool_sets[1],
        "both users see the same default tools"
    );
}

#[test]
fn memory_is_per_user() {
    let s = Stack::new();
    let alice_fact = "ALICE-MARK-fact alice keeps a pet axolotl named Quillon";
    let bob_fact = "BOB-MARK-fact bob keeps a pet pangolin named Tessaly";
    for (user, fact) in [(&s.alice, alice_fact), (&s.bob, bob_fact)] {
        let body = s.call(
            user,
            "openhuman.memory_learn",
            json!({ "text": fact, "kind": "fact", "confidence": 0.9 }),
        );
        assert!(body.get("result").is_some(), "{} learn: {body}", user.name);
    }
    for (me, other, mine, theirs) in [
        (&s.alice, &s.bob, "Quillon", "Tessaly"),
        (&s.bob, &s.alice, "Tessaly", "Quillon"),
    ] {
        let items = s
            .call(me, "openhuman.memory_items_list", json!({}))
            .to_string();
        assert!(
            items.contains(mine),
            "{} cannot list their own fact: {items}",
            me.name
        );
        assert!(
            !items.contains(theirs),
            "{} lists {}'s fact: {items}",
            me.name,
            other.name
        );

        let recall = s
            .call(
                me,
                "openhuman.memory_recall",
                json!({ "question": "which pet do I keep?" }),
            )
            .to_string();
        assert!(
            !recall.contains(theirs),
            "{} recalls {}'s fact: {recall}",
            me.name,
            other.name
        );
    }

    // On the wire too: every memory request that names an agent's tree carries
    // that agent's own credential and never the other's tree.
    let seen = s.proxy.requests();
    let memory: Vec<&Seen> = seen
        .iter()
        .filter(|r| r.path.starts_with("/memory"))
        .collect();
    assert!(!memory.is_empty(), "memory reached the backend");
    for request in memory {
        let text = format!("{} {}", request.path, request.body);
        let (a, b) = (text.contains(&s.alice.agent), text.contains(&s.bob.agent));
        assert!(
            !(a && b),
            "one memory request spans both trees: {}",
            request.path
        );
        if a {
            assert_eq!(request.authorization, s.alice.bearer(), "{}", request.path);
        }
        if b {
            assert_eq!(request.authorization, s.bob.bearer(), "{}", request.path);
        }
    }
}

#[test]
fn operator_only_methods_are_absent_from_a_users_surface() {
    let s = Stack::new();
    let operator_only = [
        "openhuman.user_agents_provision",
        "openhuman.user_agents_list",
        "openhuman.user_agents_set_credential",
        "openhuman.config_get_config",
        "openhuman.threads_delete",
        "openhuman.threads_purge",
        "openhuman.memory_engine_set",
    ];

    // Dispatch: refused as unknown, with or without valid params.
    for method in operator_only {
        for params in [
            json!({}),
            json!({ "user_id": "mallory", "thread_id": "shared" }),
        ] {
            let body = s.call(&s.alice, method, params);
            assert!(
                is_unknown_method(&body),
                "{method} must not dispatch for a user: {body}"
            );
        }
    }
    // The operator, by contrast, does have the operator plane.
    let (_, body) = rpc(
        &s.client,
        &s.base,
        Some(BEARER),
        "openhuman.user_agents_list",
    );
    assert!(body.get("result").is_some(), "{body}");

    // /schema: the user's listing omits them and keeps what a user may call.
    let schema = user_get(&s.client, &s.base, "alice", "/schema")
        .text()
        .unwrap();
    for method in operator_only {
        let short = method.trim_start_matches("openhuman.");
        assert!(
            !schema.contains(short),
            "{method} is listed in a user's /schema"
        );
    }
    assert!(
        schema.contains("threads_list"),
        "a user's /schema lists their own surface"
    );
    let operator_schema = s
        .client
        .get(format!("{}/schema", s.base))
        .bearer_auth(BEARER)
        .send()
        .unwrap()
        .text()
        .unwrap();
    assert!(
        !operator_schema.contains("threads_list"),
        "the operator's /schema lists no user family"
    );
}

#[test]
fn the_gateway_refuses_every_malformed_or_unauthorised_request() {
    use openhuman_core::user_agents::gateway::{sign, USER_HEADER, USER_SIG_HEADER};
    use reqwest::header::HeaderValue;

    let s = Stack::new();
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "core.ping", "params": {} });
    let valid = |user: &str| sign(BEARER, user, now());
    let stale = sign(BEARER, "alice", now().saturating_sub(24 * 3600));
    let unreadable = HeaderValue::from_bytes(b"alice\xff").unwrap();

    // (case, bearer, user headers, signature headers, expected status)
    type Case<'a> = (&'a str, Option<&'a str>, Vec<HeaderValue>, Vec<String>, u16);
    let user = |name: &str| HeaderValue::from_str(name).unwrap();
    let cases: Vec<Case> = vec![
        (
            "no bearer",
            None,
            vec![user("alice")],
            vec![valid("alice")],
            401,
        ),
        (
            "wrong bearer",
            Some("not-the-token"),
            vec![user("alice")],
            vec![valid("alice")],
            401,
        ),
        (
            "no signature",
            Some(BEARER),
            vec![user("alice")],
            vec![],
            401,
        ),
        (
            "garbage signature",
            Some(BEARER),
            vec![user("alice")],
            vec!["t=1,v1=00".into()],
            401,
        ),
        (
            "another user's signature",
            Some(BEARER),
            vec![user("bob")],
            vec![valid("alice")],
            401,
        ),
        (
            "stale signature",
            Some(BEARER),
            vec![user("alice")],
            vec![stale],
            401,
        ),
        (
            "unprovisioned user",
            Some(BEARER),
            vec![user("mallory")],
            vec![valid("mallory")],
            403,
        ),
        (
            "two user headers",
            Some(BEARER),
            vec![user("alice"), user("bob")],
            vec![valid("alice")],
            400,
        ),
        (
            "unreadable user header",
            Some(BEARER),
            vec![unreadable],
            vec![valid("alice")],
            400,
        ),
    ];
    for name in ["alice", "bob"] {
        provision(&s.client, &s.base, name);
    }
    for (case, bearer, users, signatures, expected) in cases {
        let mut request = s.client.post(format!("{}/rpc", s.base)).json(&body);
        if let Some(bearer) = bearer {
            request = request.bearer_auth(bearer);
        }
        for value in users {
            request = request.header(USER_HEADER, value);
        }
        for value in signatures {
            request = request.header(USER_SIG_HEADER, value);
        }
        let response = request.send().expect("POST /rpc");
        assert_eq!(response.status().as_u16(), expected, "{case}");
    }

    // Control: the same request, correctly signed, goes through.
    let (status, reply) = user_rpc(&s.client, &s.base, BEARER, "alice", None, "core.ping");
    assert_eq!(status, 200, "{reply}");
}
