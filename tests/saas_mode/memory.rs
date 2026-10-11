//! Memory round trips across SaaS profiles, through the real binary and a
//! backend that actually serves `/memory/*` ([`MockMemory`]).
//!
//! Each profile runs the hosted `tinyhumans` engine with its own credential,
//! confined to `user:<id>` on the legacy layout (`profiles::layout`). The
//! mock keeps one store for every bearer, so nothing but the core's own
//! confinement keeps the profiles apart: the case `memory::user_scope`
//! defends, users sharing one engine.
//!
//! `ann` and `anna` are deliberate: one root is a string prefix of the
//! other, and a scope listing is a string-prefix match.

use std::path::PathBuf;

use super::mock_memory::{credential_id, MemoryRequest, MockMemory};
use super::*;

const USERS: [&str; 3] = ["ann", "anna", "bob"];

fn canary(user: &str) -> String {
    format!("canary {user} keeps the secret word {user}-quince-7f3a")
}

/// The secret part of `user`'s canary, unique to that user.
fn secret(user: &str) -> String {
    format!("{user}-quince-7f3a")
}

/// The legacy scope path of `user`'s memory root.
fn root_path(profile: &str) -> String {
    format!("app:tinymemory/user:{profile}")
}

fn token(user: &str) -> String {
    format!("{user}-memory-jwt")
}

/// Whether `scope` is `root` or lies below it (segment-wise).
fn inside(scope: &str, root: &str) -> bool {
    scope == root || scope.starts_with(&format!("{root}/"))
}

struct Stack {
    _server: Server,
    base: String,
    client: reqwest::blocking::Client,
    mock: MockMemory,
    log: PathBuf,
    _d: Deployment,
}

impl Stack {
    fn start() -> Self {
        let d = deployment(true);
        let mock = MockMemory::start();
        let log = d.tmp.path().join("core.log");
        let (server, base) = spawn_core(|port| {
            let mut cmd = core_command(&d, &["--port", &port.to_string()]);
            cmd.env("BACKEND_URL", format!("http://127.0.0.1:{}", mock.port))
                .env("RUST_LOG", "info,openhuman::memory=debug")
                .stdout(std::fs::File::create(&log).unwrap())
                .stderr(Stdio::null());
            cmd
        });
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        Self {
            _server: server,
            base,
            client,
            mock,
            log,
            _d: d,
        }
    }

    /// Provisions `user` with a session credential of their own; returns the
    /// profile id.
    fn provision(&self, user: &str) -> String {
        let profile = provision(&self.client, &self.base, user);
        let (_, body) = rpc_with(
            &self.client,
            &self.base,
            Some(BEARER),
            "openhuman.profiles_set_credential",
            json!({ "profile_id": profile, "kind": "session", "token": token(user) }),
        );
        assert!(body.get("result").is_some(), "{body}");
        profile
    }

    /// `method` as `user`; the whole JSON-RPC body.
    fn call(&self, user: &str, method: &str, params: Value) -> Value {
        let (status, body) =
            user_rpc_with(&self.client, &self.base, BEARER, user, None, method, params);
        assert_eq!(status, 200, "{user} {method}: {body}");
        body
    }

    /// `method` as `user`, which must succeed; its result.
    fn ok(&self, user: &str, method: &str, params: Value) -> Value {
        let body = self.call(user, method, params);
        let result = body.get("result").unwrap_or_else(|| {
            panic!(
                "{user} {method} failed: {body}\ncore log:\n{}",
                self.log_tail()
            )
        });
        result.get("result").unwrap_or(result).clone()
    }

    fn log_tail(&self) -> String {
        let log = std::fs::read_to_string(&self.log).unwrap_or_default();
        let lines: Vec<&str> = log.lines().collect();
        lines[lines.len().saturating_sub(60)..].join("\n")
    }
}

/// Asserts `body` holds none of the secrets of anyone but `user`.
fn holds_only_own(user: &str, what: &str, body: &Value) {
    let text = body.to_string();
    for other in USERS.iter().filter(|other| **other != user) {
        assert!(
            !text.contains(&secret(other)),
            "{what} as {user} leaked {other}'s memory: {text}"
        );
    }
}

#[test]
fn profiles_round_trip_their_own_memory_and_never_anothers() {
    let s = Stack::start();
    let profiles: Vec<(String, String)> = USERS
        .iter()
        .map(|user| (user.to_string(), s.provision(user)))
        .collect();

    // Each profile stores its canary through the user surface.
    let mut learned = std::collections::HashMap::new();
    for (user, _) in &profiles {
        let view = s.ok(
            user,
            "openhuman.memory_learn",
            json!({ "text": canary(user), "kind": "fact" }),
        );
        let id = view["id"]
            .as_str()
            .expect("learn returns an id")
            .to_string();
        learned.insert(user.clone(), id);
    }
    for (user, profile) in &profiles {
        let scopes = s.mock.scopes_holding(&secret(user));
        assert!(!scopes.is_empty(), "{user}'s canary reached the backend");
        for scope in scopes {
            assert!(
                inside(&scope, &root_path(profile)),
                "{user}'s canary landed outside their root: {scope}"
            );
        }
    }

    // Each reads its own back, and only its own.
    for (user, _) in &profiles {
        let listed = s.ok(
            user,
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["learning"] } }),
        );
        assert!(
            listed.to_string().contains(&secret(user)),
            "{user} lists their canary: {listed}"
        );
        holds_only_own(user, "items_list", &listed);

        let fetched = s.ok(
            user,
            "openhuman.memory_fetch",
            json!({ "query": "canary secret quince" }),
        );
        assert!(
            fetched.to_string().contains(&secret(user)),
            "{user} fetches their canary: {fetched}"
        );
        holds_only_own(user, "fetch", &fetched);

        let recalled = s.ok(
            user,
            "openhuman.memory_recall",
            json!({ "question": "what is the canary secret quince word?" }),
        );
        assert!(
            recalled.to_string().contains(&secret(user)),
            "{user} recalls their canary: {recalled}"
        );
        holds_only_own(user, "recall", &recalled);

        for facet in ["namespace", "kind", "agent"] {
            let explored = s.ok(user, "openhuman.memory_explore", json!({ "facet": facet }));
            holds_only_own(user, &format!("explore by {facet}"), &explored);
            let text = explored.to_string();
            for (other, other_profile) in &profiles {
                if other != user {
                    assert!(
                        !text.contains(&format!("user:{other_profile}\"")),
                        "explore by {facet} as {user} names {other}'s root: {text}"
                    );
                }
            }
        }
    }

    // A malicious user aims every parameter they control at someone else.
    let (attacker, attacker_profile) = (&profiles[0].0, &profiles[0].1);
    for (victim, victim_profile) in profiles.iter().skip(1) {
        let foreign = format!("user:{victim_profile}");
        let reaches = [
            json!({ "at": foreign, "descendants": true, "inherit": false }),
            json!({ "at": foreign, "descendants": false, "inherit": true }),
            json!({ "at": format!("{foreign}/agent:x"), "descendants": true }),
            json!({ "at": "", "descendants": true }),
        ];
        for reach in &reaches {
            let filter = json!({ "reach": reach });
            for (method, params) in [
                ("openhuman.memory_items_list", json!({ "filter": filter })),
                (
                    "openhuman.memory_fetch",
                    json!({ "query": "canary quince", "filter": filter }),
                ),
                (
                    "openhuman.memory_recall",
                    json!({ "question": "canary quince", "filter": filter }),
                ),
                (
                    "openhuman.memory_explore",
                    json!({ "facet": "namespace", "filter": filter }),
                ),
            ] {
                let body = s.call(attacker, method, params);
                holds_only_own(attacker, &format!("{method} reaching {reach}"), &body);
            }
            let body = s.call(
                attacker,
                "openhuman.memory_forget",
                json!({ "ids": [learned[victim]], "reach": reach }),
            );
            holds_only_own(attacker, "forget", &body);
        }
        for step in [foreign.clone(), String::new()] {
            let body = s.call(
                attacker,
                "openhuman.memory_items_list",
                json!({ "path": [{ "facet": "namespace", "value": step }] }),
            );
            holds_only_own(attacker, "items_list down a foreign path", &body);
        }

        // Forgetting the victim's id, with no reach at all, forgets nothing.
        let body = s.call(
            attacker,
            "openhuman.memory_forget",
            json!({ "ids": [learned[victim]] }),
        );
        if let Some(result) = body.get("result") {
            let result = result.get("result").unwrap_or(result);
            assert_eq!(result["forgotten"], json!(0), "{body}");
        }

        // Writing into the victim's tree lands in the attacker's, or not at all.
        let planted = format!("planted by {attacker} into {victim}");
        for namespace in [foreign.clone(), format!("{foreign}/agent:main")] {
            let _ = s.call(
                attacker,
                "openhuman.memory_learn",
                json!({ "text": planted, "meta": { "namespace": namespace } }),
            );
        }
        for scope in s.mock.scopes_holding(&planted) {
            assert!(
                inside(&scope, &root_path(attacker_profile)),
                "{attacker} planted into {scope}"
            );
        }
        let listed = s.ok(
            victim,
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["learning"] } }),
        );
        assert!(
            !listed.to_string().contains(&planted),
            "{victim} reads what {attacker} planted: {listed}"
        );
        assert!(
            listed.to_string().contains(&secret(victim)),
            "{victim} still holds their canary after {attacker}'s attempts: {listed}"
        );
    }

    // A profile forgets its own memory; the others keep theirs.
    let sweep_start = s.mock.requests().len();
    let forgotten = s.ok(
        attacker,
        "openhuman.memory_forget",
        json!({ "ids": [learned[attacker]] }),
    );
    let sweep = sweep_start..s.mock.requests().len();
    assert_eq!(forgotten["forgotten"], json!(1), "{forgotten}");
    for (user, _) in profiles.iter().skip(1) {
        let listed = s.ok(
            user,
            "openhuman.memory_items_list",
            json!({ "filter": { "kinds": ["learning"] } }),
        );
        assert!(
            listed.to_string().contains(&secret(user)),
            "{user} keeps their canary after {attacker} forgot theirs: {listed}"
        );
    }

    // On the wire: every memory request carried one profile's credential,
    // and named only scopes inside that profile's root. No exception for
    // forgetting by id: with a reach, `memory::ops::forget` hands the ids to
    // the engine's `forget_within`, which looks for their events inside that
    // reach only. A sweep from the tree's root (the unscoped
    // `ForgetTarget::Ids`) would list and read every other profile's scopes
    // with the caller's credential, and fails here.
    let requests = s.mock.requests();
    assert!(!requests.is_empty());
    let owners: std::collections::HashMap<String, String> = profiles
        .iter()
        .map(|(user, profile)| (credential_id(&token(user)), root_path(profile)))
        .collect();
    let mut seen = std::collections::HashSet::new();
    for (index, request) in requests.iter().enumerate() {
        let MemoryRequest {
            method,
            path,
            bearer,
            scopes,
        } = request;
        let root = owners.get(bearer).unwrap_or_else(|| {
            panic!("request {index}, {method} {path}, carried no profile's credential")
        });
        seen.insert(bearer.clone());
        let forgetting = if sweep.contains(&index) {
            " (during the forget)"
        } else {
            ""
        };
        for scope in scopes {
            assert!(
                inside(scope, root),
                "request {index}{forgetting}, {method} {path} with {root}'s credential, named {scope}"
            );
        }
    }
    assert_eq!(seen.len(), USERS.len(), "every profile reached the backend");
}
