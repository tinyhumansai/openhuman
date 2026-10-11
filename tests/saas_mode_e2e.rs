//! `openhuman-core run --mode saas` end to end, through the real binary.
//!
//! A SaaS core must refuse an unsafe deployment before it binds anything, and
//! a safe one must serve nothing but its core built-ins behind the gateway
//! bearer until per-user isolation opens domain families.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

#[path = "saas_mode/support.rs"]
mod support;
use support::*;

#[path = "saas_mode/cluster.rs"]
mod cluster;
#[path = "saas_mode/delivery_recovery.rs"]
mod delivery_recovery;
#[path = "saas_mode/hashed_ids.rs"]
mod hashed_ids;
#[path = "saas_mode/memory.rs"]
mod memory;
#[path = "saas_mode/mock_llm.rs"]
#[allow(dead_code)]
mod mock_llm;
#[path = "saas_mode/mock_memory.rs"]
mod mock_memory;

#[test]
fn an_unsafe_deployment_is_refused_before_it_binds() {
    let d = deployment(false);
    let output = core_command(&d, &[])
        .env("OPENHUMAN_WORKSPACE", d.tmp.path())
        .output()
        .expect("run openhuman-core");
    assert!(!output.status.success(), "an unsafe SaaS boot must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("refusing to boot"), "{stderr}");
    assert!(stderr.contains("service token"), "{stderr}");
    assert!(stderr.contains("OPENHUMAN_WORKSPACE"), "{stderr}");
    assert!(
        !d.root.join("operator").exists(),
        "a refused boot must not create state"
    );
}

#[test]
fn an_unsafe_tool_allowlist_is_refused_before_it_binds() {
    let d = deployment(true);
    std::fs::write(
        &d.config,
        format!(
            "root = {:?}\ntool_allowlist = [\"coding\", \"host_shell\"]\n\n[sandbox]\nnetwork = \"host\"\n",
            d.root.display().to_string()
        ),
    )
    .unwrap();
    let output = core_command(&d, &[]).output().expect("run openhuman-core");
    assert!(!output.status.success(), "an unsafe allowlist must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not tool groups"), "{stderr}");
    assert!(stderr.contains("network `host`"), "{stderr}");
    assert!(!d.root.join("operator").exists());
}

#[test]
fn saas_mode_without_an_operator_config_is_refused() {
    let output = Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .args(["run", "--mode", "saas"])
        .env_remove("OPENHUMAN_MODE")
        .output()
        .expect("run openhuman-core");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--saas-config"), "{stderr}");
}

#[test]
fn a_safe_deployment_serves_core_and_the_operator_plane_behind_the_gateway_bearer() {
    let d = deployment(true);
    let (server, base, client) = start(&d);

    let (status, _) = rpc(&client, &base, None, "core.ping");
    assert_eq!(status, 401, "no bearer, no access");
    let (status, _) = rpc(&client, &base, Some("wrong-bearer"), "core.ping");
    assert_eq!(status, 401, "only the gateway bearer is accepted");

    let (status, body) = rpc(&client, &base, Some(BEARER), "core.ping");
    assert_eq!(status, 200);
    assert!(body.get("result").is_some(), "core.ping answers: {body}");

    // The operator plane serves none of the user families.
    for method in [
        "openhuman.threads_list",
        "openhuman.config_get_config",
        "openhuman.memory_search",
    ] {
        let (_, body) = rpc(&client, &base, Some(BEARER), method);
        assert!(
            body.get("error").is_some(),
            "{method} must not be served: {body}"
        );
    }

    // The operator plane provisions one profile per user. A user id outside
    // the raw charset is hashed, and never echoed back.
    let (_, body) = rpc_with(
        &client,
        &base,
        Some(BEARER),
        "openhuman.profiles_provision",
        json!({ "user_id": "alice@example.com" }),
    );
    let result = body
        .get("result")
        .unwrap_or_else(|| panic!("provision: {body}"));
    let profile_id = result
        .pointer("/result/profile_id")
        .or_else(|| result.get("profile_id"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("profile_id in {result}"))
        .to_string();
    assert_eq!(
        profile_id,
        openhuman_core::profiles::ProfileId::for_user(
            "alice@example.com",
            openhuman_core::profiles::ProfileIdMode::Raw
        )
        .unwrap()
        .to_string(),
        "a user id outside the raw charset maps to its deterministic hash"
    );
    assert!(!body.to_string().contains("alice"), "{body}");
    assert!(d
        .root
        .join("users")
        .join(&profile_id)
        .join("workspace")
        .is_dir());

    let (_, body) = rpc(&client, &base, Some(BEARER), "openhuman.profiles_list");
    assert!(body.to_string().contains(&profile_id), "{body}");
    let (_, body) = rpc_with(
        &client,
        &base,
        Some(BEARER),
        "openhuman.profiles_status",
        json!({ "profile_id": profile_id }),
    );
    assert!(body.get("result").is_some(), "{body}");
    let (_, body) = rpc_with(
        &client,
        &base,
        Some(BEARER),
        "openhuman.profiles_deprovision",
        json!({ "profile_id": profile_id }),
    );
    assert!(body.get("result").is_some(), "{body}");
    assert!(!d.root.join("users").join(&profile_id).exists());
    assert!(
        d.root.join("deprovisioned").is_dir(),
        "archived, not deleted"
    );

    assert!(
        d.root.join("operator").join("workspace").is_dir(),
        "the operator plane lives under the SaaS root"
    );
    assert!(
        !d.tmp
            .path()
            .join(".openhuman")
            .join("active_user.toml")
            .exists(),
        "a SaaS boot never activates a desktop user"
    );
    let desktop = d.tmp.path().join(".openhuman");
    let leaked: Vec<_> = std::fs::read_dir(&desktop)
        .map(|entries| entries.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(
        leaked.is_empty(),
        "a SaaS boot writes nothing under ~/.openhuman (keyring included): {leaked:?}"
    );
    drop(server);
}

#[test]
fn gateway_requests_run_under_the_named_users_profile() {
    let d = deployment(true);
    let (server, base, client) = start(&d);

    // Provision alice and hand the core her credential; bob stays unknown.
    let (_, body) = rpc_with(
        &client,
        &base,
        Some(BEARER),
        "openhuman.profiles_provision",
        json!({ "user_id": "alice" }),
    );
    let alice = openhuman_core::profiles::ProfileId::for_user(
        "alice",
        openhuman_core::profiles::ProfileIdMode::Raw,
    )
    .unwrap();
    assert!(body.to_string().contains(alice.as_str()), "{body}");
    let (_, body) = rpc_with(
        &client,
        &base,
        Some(BEARER),
        "openhuman.profiles_set_credential",
        json!({ "profile_id": alice.as_str(), "kind": "session", "token": "alice-session-jwt" }),
    );
    assert!(body.get("result").is_some(), "{body}");
    assert!(!body.to_string().contains("alice-session-jwt"), "{body}");
    let (_, body) = rpc_with(
        &client,
        &base,
        Some(BEARER),
        "openhuman.profiles_status",
        json!({ "profile_id": alice.as_str() }),
    );
    assert!(
        body.to_string().contains("\"has_credential\":true"),
        "{body}"
    );

    // A signed request for alice runs under her agent.
    let (status, body) = user_rpc(&client, &base, BEARER, "alice", None, "core.ping");
    assert_eq!(status, 200, "{body}");
    assert!(body.get("result").is_some(), "{body}");

    // A user's scope cannot reach the operator plane.
    let (_, body) = user_rpc(
        &client,
        &base,
        BEARER,
        "alice",
        None,
        "openhuman.profiles_list",
    );
    assert!(
        body.get("error").is_some(),
        "operator methods are not a user's: {body}"
    );

    // Refusals: bad bearer first, then signatures, then provisioning.
    let (status, body) = user_rpc(&client, &base, "wrong-bearer", "alice", None, "core.ping");
    assert_eq!(status, 401, "{body}");
    let (status, body) = user_rpc(&client, &base, "wrong-bearer", "bob", None, "core.ping");
    assert_eq!(
        status, 401,
        "an unauthenticated caller cannot probe users: {body}"
    );
    let (status, body) = user_rpc(
        &client,
        &base,
        BEARER,
        "alice",
        Some("t=1,v1=00"),
        "core.ping",
    );
    assert_eq!(status, 401, "{body}");
    let forged = openhuman_core::profiles::gateway::sign(BEARER, "alice", now());
    let (status, body) = user_rpc(&client, &base, BEARER, "bob", Some(&forged), "core.ping");
    assert_eq!(status, 401, "alice's signature does not cover bob: {body}");
    let (status, body) = user_rpc(&client, &base, BEARER, "bob", None, "core.ping");
    assert_eq!(status, 403, "bob is not provisioned: {body}");
    // A repeated signature header is refused, not resolved to the first value.
    {
        use openhuman_core::profiles::gateway::{sign, USER_HEADER, USER_SIG_HEADER};
        let response = client
            .post(format!("{base}/rpc"))
            .bearer_auth(BEARER)
            .header(USER_HEADER, "alice")
            .header(USER_SIG_HEADER, sign(BEARER, "alice", now()))
            .header(USER_SIG_HEADER, sign(BEARER, "alice", now()))
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "core.ping", "params": {} }))
            .send()
            .expect("POST /rpc");
        assert_eq!(
            response.status().as_u16(),
            400,
            "a repeated signature header"
        );
    }

    // Single-user surfaces are closed.
    for path in ["/events", "/events/domain", "/v1/models", "/dev/connect"] {
        let status = client
            .get(format!("{base}{path}"))
            .bearer_auth(BEARER)
            .send()
            .unwrap()
            .status()
            .as_u16();
        assert_eq!(status, 404, "{path}");
    }

    // The credential lives in alice's own directory.
    let profile_dir = d.root.join("users").join(alice.as_str());
    let stored: Vec<_> = std::fs::read_dir(&profile_dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        stored.iter().any(|name| name.contains("auth")),
        "credential store beside alice's config: {stored:?}"
    );
    drop(server);
}

#[test]
fn each_user_sees_only_their_own_threads() {
    let d = deployment(true);
    let (server, base, client) = start(&d);
    let alice = provision(&client, &base, "alice");
    let bob = provision(&client, &base, "bob");
    let call = |user: &str, method: &str, params: Value| {
        user_rpc_with(&client, &base, BEARER, user, None, method, params)
    };

    let (status, body) = call("alice", "openhuman.threads_create_new", json!({}));
    assert_eq!(status, 200, "{body}");
    assert!(body.get("result").is_some(), "{body}");

    // The same caller-chosen id in two users' scopes is two threads.
    for user in ["alice", "bob"] {
        let (_, body) = call(
            user,
            "openhuman.threads_upsert",
            json!({ "id": "shared-id", "title": format!("{user}'s"), "created_at": "2026-10-07T00:00:00Z" }),
        );
        assert!(body.get("result").is_some(), "{user} upsert: {body}");
    }

    let (_, alice_list) = call("alice", "openhuman.threads_list", json!({}));
    let (_, bob_list) = call("bob", "openhuman.threads_list", json!({}));
    let alice_ids = thread_ids(&alice_list);
    let bob_ids = thread_ids(&bob_list);
    assert_eq!(alice_ids.len(), 2, "alice: {alice_list}");
    assert_eq!(bob_ids, vec!["shared-id".to_string()], "bob: {bob_list}");
    assert!(bob_list.to_string().contains("bob's"), "{bob_list}");
    assert!(!bob_list.to_string().contains("alice's"), "{bob_list}");
    // And the other way: alice keeps her own `shared-id`, untouched by bob's.
    assert!(
        alice_ids.contains(&"shared-id".to_string()),
        "alice: {alice_list}"
    );
    assert!(alice_list.to_string().contains("alice's"), "{alice_list}");
    assert!(!alice_list.to_string().contains("bob's"), "{alice_list}");

    // A SaaS user cannot point a thread at a host folder.
    let (_, body) = call(
        "alice",
        "openhuman.threads_create_new",
        json!({ "action_dir": "/etc" }),
    );
    assert!(body.get("error").is_some(), "{body}");

    // A hidden method answers unknown-method even with bad params, rather
    // than its parameter errors.
    let (_, body) = call("alice", "openhuman.threads_update_working_dir", json!({}));
    let error = body["error"].to_string();
    assert!(!error.contains("missing"), "{body}");

    // Each user's threads live in their own workspace.
    for (agent, owner) in [(&alice, "alice"), (&bob, "bob")] {
        let threads = d.root.join("users").join(agent).join("workspace");
        assert!(threads.is_dir(), "{owner}'s workspace");
    }
    // Boot migrations leave an empty index in the operator workspace; no user
    // thread may ever reach it.
    let operator_index = d
        .root
        .join("operator/workspace/memory/conversations/threads.jsonl");
    let operator_threads = std::fs::read_to_string(&operator_index).unwrap_or_default();
    assert!(
        !operator_threads.contains("shared-id") && operator_threads.trim().is_empty(),
        "no user thread lands in the operator workspace: {operator_threads}"
    );

    // Reserved and path-like ids are refused; turn-starting methods are closed.
    for id in ["channel:telegram/1", "../escape"] {
        let (_, body) = call(
            "alice",
            "openhuman.threads_upsert",
            json!({ "id": id, "title": "x", "created_at": "2026-10-07T00:00:00Z" }),
        );
        assert!(body.get("error").is_some(), "{id}: {body}");
    }
    // A method off the user surface is unknown, not a parameter error.
    let (_, body) = call("alice", "openhuman.config_get_config", json!({}));
    assert!(body.to_string().contains("unknown method"), "{body}");
    drop(server);
}

#[test]
fn chat_events_reach_only_the_user_whose_turn_produced_them() {
    let d = deployment(true);
    // Point the backend at a closed port so the turn fails fast — the failure
    // is itself an event on the owner's stream, without any real inference.
    let (server, base) = spawn_core(|port| {
        let mut cmd = core_command(&d, &["--port", &port.to_string()]);
        cmd.env("BACKEND_URL", "http://127.0.0.1:9")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd
    });
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    provision(&client, &base, "alice");
    provision(&client, &base, "bob");

    // The operator has no chat stream.
    let status = client
        .get(format!("{base}/events?client_id=c1"))
        .bearer_auth(BEARER)
        .send()
        .unwrap()
        .status()
        .as_u16();
    assert_eq!(status, 404);

    // Both users listen on the same client id.
    let alice_events = user_events(&base, "alice", "c1");
    let bob_events = user_events(&base, "bob", "c1");
    for (who, rx) in [("alice", &alice_events), ("bob", &bob_events)] {
        let first = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(first, "status:200", "{who}'s stream opens");
    }

    // A reserved thread id is refused before any turn starts.
    let (_, body) = user_rpc_with(
        &client,
        &base,
        BEARER,
        "alice",
        None,
        "openhuman.channel_web_chat",
        json!({ "client_id": "c1", "thread_id": "channel:slack:x", "message": "hello" }),
    );
    assert!(body.get("error").is_some(), "{body}");

    let (status, body) = user_rpc_with(
        &client,
        &base,
        BEARER,
        "alice",
        None,
        "openhuman.channel_web_chat",
        json!({ "client_id": "c1", "thread_id": "chat-1", "message": "hello" }),
    );
    assert_eq!(status, 200, "{body}");

    let mut alice_got = Vec::new();
    let until = Instant::now() + Duration::from_secs(30);
    while Instant::now() < until {
        match alice_events.recv_timeout(Duration::from_millis(500)) {
            Ok(data) if data.contains("chat-1") => {
                alice_got.push(data);
                if alice_got
                    .iter()
                    .any(|e| e.contains("error") || e.contains("done"))
                {
                    break;
                }
            }
            Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(e) => panic!("alice's stream closed: {e}"),
        }
    }
    assert!(!alice_got.is_empty(), "alice receives her turn's events");
    assert!(
        alice_got.iter().all(|e| !e.contains("\"agent\"")),
        "the routing stamp is not on the wire: {alice_got:?}"
    );

    std::thread::sleep(Duration::from_secs(1));
    let leaked: Vec<String> = bob_events.try_iter().collect();
    assert!(
        leaked.is_empty(),
        "bob shares the client id but must see none of alice's events: {leaked:?}"
    );
    drop(server);
}

#[test]
fn users_reach_their_memory_but_not_its_configuration() {
    let d = deployment(true);
    let (server, base, client) = start(&d);
    provision(&client, &base, "alice");
    let call = |method: &str, params: Value| {
        user_rpc_with(&client, &base, BEARER, "alice", None, method, params)
    };

    // Reachable: with no backend in this test the engine is off, so recall
    // answers with memory's own error — not "unknown method".
    let (_, body) = call(
        "openhuman.memory_recall",
        json!({ "question": "anything?" }),
    );
    let text = body.to_string();
    assert!(
        !text.contains("unknown method"),
        "memory_recall is on the surface: {text}"
    );

    // Not reachable: anything that changes where memory lives or reads the host.
    // The engine's settings carry its credential: operator-only.
    for method in [
        "openhuman.memory_engine_get",
        "openhuman.memory_engine_set",
        "openhuman.memory_policy_set",
        "openhuman.memory_sources_add",
        "openhuman.memory_import_start",
    ] {
        let (_, body) = call(method, json!({}));
        assert!(
            body.to_string().contains("unknown method"),
            "{method} must be absent: {body}"
        );
    }
    drop(server);
}

#[test]
fn a_users_turn_reaches_inference_with_their_own_credential() {
    // The operator holds no credential. A process-wide "signed out" flag used
    // to park every user's model call behind it; each user's credential is
    // what counts.
    let d = deployment(true);
    let (backend, requests) = recording_backend();
    let (server, base) = spawn_core(|port| {
        let mut cmd = core_command(&d, &["--port", &port.to_string()]);
        cmd.env("BACKEND_URL", format!("http://127.0.0.1:{backend}"))
            .env("RUST_LOG", "debug")
            .stdout(std::fs::File::create(d.tmp.path().join("core.log")).unwrap())
            .stderr(Stdio::null());
        cmd
    });
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let alice = provision(&client, &base, "alice");
    let (_, body) = rpc_with(
        &client,
        &base,
        Some(BEARER),
        "openhuman.profiles_set_credential",
        json!({ "profile_id": alice, "kind": "session", "token": "alice-session-jwt" }),
    );
    assert!(body.get("result").is_some(), "{body}");

    let (status, body) = user_rpc_with(
        &client,
        &base,
        BEARER,
        "alice",
        None,
        "openhuman.channel_web_chat",
        json!({ "client_id": "c1", "thread_id": "chat-1", "message": "hello" }),
    );
    assert_eq!(status, 200, "{body}");

    let until = Instant::now() + Duration::from_secs(60);
    let mut seen = Vec::new();
    let inference = loop {
        let left = until.saturating_duration_since(Instant::now());
        match requests.recv_timeout(left) {
            Ok((path, auth)) if path.contains("/chat/completions") => break Some((path, auth)),
            Ok(other) => seen.push(other),
            Err(_) => break None,
        }
    };
    let (_, auth) = inference.unwrap_or_else(|| {
        let log = std::fs::read_to_string(d.tmp.path().join("core.log")).unwrap_or_default();
        let notable: Vec<&str> = log
            .lines()
            .filter(|l| !l.contains("[scheduler_gate]"))
            .collect();
        panic!(
            "alice's turn never reached inference; saw {} other request(s). Core log:\n{}",
            seen.len(),
            notable[notable.len().saturating_sub(120)..].join("\n")
        )
    });
    assert_eq!(auth, "Bearer alice-session-jwt");
    drop(server);
}

#[test]
fn a_duplicate_or_unreadable_user_header_is_refused() {
    use openhuman_core::profiles::gateway::USER_HEADER;
    let d = deployment(true);
    let (server, base, client) = start(&d);
    let body =
        json!({ "jsonrpc": "2.0", "id": 1, "method": "openhuman.profiles_list", "params": {} });

    // Two user headers: refused, never run as the operator.
    let status = client
        .post(format!("{base}/rpc"))
        .bearer_auth(BEARER)
        .header(USER_HEADER, "alice")
        .header(USER_HEADER, "bob")
        .json(&body)
        .send()
        .unwrap()
        .status()
        .as_u16();
    assert_eq!(status, 400);

    // A header value that is valid HTTP but not text: refused too.
    let unreadable = reqwest::header::HeaderValue::from_bytes(b"alice\xff").unwrap();
    let status = client
        .post(format!("{base}/rpc"))
        .bearer_auth(BEARER)
        .header(USER_HEADER, unreadable)
        .json(&body)
        .send()
        .unwrap()
        .status()
        .as_u16();
    assert_eq!(status, 400);
    drop(server);
}

#[test]
fn a_profile_holds_web_and_relayed_channel_threads() {
    let d = deployment(true);
    let (server, base, client) = start_offline(&d);
    provision(&client, &base, "alice");
    provision(&client, &base, "bob");
    let call = |user: &str, method: &str, params: Value| {
        user_rpc_with(&client, &base, BEARER, user, None, method, params)
    };

    // The gateway listens on each user's stream under one client id.
    let alice_events = user_events(&base, "alice", "channel-relay");
    let bob_events = user_events(&base, "bob", "channel-relay");
    for (who, rx) in [("alice", &alice_events), ("bob", &bob_events)] {
        let first = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(first, "status:200", "{who}'s stream opens");
    }

    // Alice chats on the web...
    let (_, body) = call(
        "alice",
        "openhuman.threads_upsert",
        json!({ "id": "web-1", "title": "web", "created_at": "2026-10-09T00:00:00Z" }),
    );
    assert!(body.get("result").is_some(), "{body}");
    let (status, body) = call(
        "alice",
        "openhuman.channel_web_chat",
        json!({ "client_id": "c1", "thread_id": "web-1", "message": "hello on the web" }),
    );
    assert_eq!(status, 200, "{body}");

    // ...and the gateway relays a Telegram message of hers.
    let relayed = json!({
        "channel": "telegram",
        "chat_id": "777",
        "sender_id": "555",
        "sender_name": "Alice",
        "message_id": "tg-1",
        "text": "hello from telegram",
    });
    let (status, body) = call("alice", "openhuman.channel_relay_inbound", relayed.clone());
    assert_eq!(status, 200, "{body}");
    let text = body.to_string();
    assert!(text.contains("\"accepted\":true"), "{body}");
    let channel_thread = "channel:telegram/555/777";
    assert!(text.contains(channel_thread), "{body}");

    // The reply (here the offline backend's error) reaches alice's stream as
    // `channel_outbound`, for the gateway to deliver.
    let mut outbound = None;
    let until = Instant::now() + Duration::from_secs(90);
    while Instant::now() < until && outbound.is_none() {
        match alice_events.recv_timeout(Duration::from_millis(500)) {
            Ok(data) if data.contains("channel_outbound") => outbound = Some(data),
            Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(e) => panic!("alice's stream closed: {e}"),
        }
    }
    let outbound = outbound.expect("alice receives channel_outbound");
    let event: Value = serde_json::from_str(&outbound).expect("event json");
    assert_eq!(event["thread_id"], channel_thread, "{event}");
    assert_eq!(event["structured"]["channel"], "telegram", "{event}");
    assert_eq!(event["structured"]["chat_id"], "777", "{event}");
    assert!(
        event["full_response"]
            .as_str()
            .is_some_and(|t| !t.is_empty()),
        "{event}"
    );
    assert!(event.get("agent").is_none(), "no routing stamp on the wire");
    // The turn reached the agent: the offline backend's error, never the
    // dispatch failing to find an `agent.run_turn` handler.
    assert!(
        !event["full_response"]
            .as_str()
            .unwrap_or_default()
            .contains("no native handler"),
        "relayed turn must reach the agent: {event}"
    );

    // A gateway retry of the same message runs nothing twice.
    let (_, body) = call("alice", "openhuman.channel_relay_inbound", relayed);
    assert!(body.to_string().contains("\"duplicate\":true"), "{body}");

    // A relay cannot forge a thread id through its fields.
    let (_, body) = call(
        "alice",
        "openhuman.channel_relay_inbound",
        json!({ "channel": "telegram", "chat_id": "1/2", "sender_id": "3",
                "message_id": "x", "text": "hi" }),
    );
    assert!(body.get("error").is_some(), "{body}");

    // Alice's profile holds both threads; bob's holds neither.
    let (_, alice_list) = call("alice", "openhuman.threads_list", json!({}));
    let alice_ids = thread_ids(&alice_list);
    assert!(alice_ids.contains(&"web-1".to_string()), "{alice_list}");
    assert!(
        alice_ids.contains(&channel_thread.to_string()),
        "{alice_list}"
    );
    let (_, bob_list) = call("bob", "openhuman.threads_list", json!({}));
    let bob_ids = thread_ids(&bob_list);
    assert!(
        !bob_ids.contains(&"web-1".to_string()) && !bob_ids.contains(&channel_thread.to_string()),
        "{bob_list}"
    );

    // The relayed exchange is on alice's channel thread.
    let (_, messages) = call(
        "alice",
        "openhuman.threads_messages_list",
        json!({ "thread_id": channel_thread }),
    );
    assert!(
        messages.to_string().contains("hello from telegram"),
        "{messages}"
    );

    // Bob's stream carried none of alice's events.
    std::thread::sleep(Duration::from_secs(1));
    let leaked: Vec<String> = bob_events.try_iter().collect();
    assert!(
        leaked.is_empty(),
        "bob must see none of alice's events: {leaked:?}"
    );

    // No relayed turn lands in the operator workspace.
    let operator_index = d
        .root
        .join("operator/workspace/memory/conversations/threads.jsonl");
    let operator_threads = std::fs::read_to_string(&operator_index).unwrap_or_default();
    assert!(
        !operator_threads.contains("telegram"),
        "no relayed thread in the operator workspace: {operator_threads}"
    );
    drop(server);
}

/// `--port` is a preference: a core that finds it taken listens elsewhere.
/// The harness must notice that another core answers on the port, or the test
/// drives that core's deployment and checks its own (an empty `users/`).
#[test]
fn a_core_on_a_taken_port_is_not_mistaken_for_ours() {
    let first = deployment(true);
    let (mut first_server, base, client) = start(&first);
    let deadline = Instant::now() + Duration::from_secs(120);
    assert_eq!(
        wait_until_serving(&client, &base, &mut first_server, deadline),
        Ok(Serving::Ours)
    );

    let second = deployment(true);
    let port = base.rsplit(':').next().unwrap().to_string();
    let child = core_command(&second, &["--port", &port])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn openhuman-core");
    let mut second_server = Server(child);
    let deadline = Instant::now() + Duration::from_secs(120);
    assert_eq!(
        wait_until_serving(&client, &base, &mut second_server, deadline),
        Ok(Serving::Taken(Some(u64::from(first_server.0.id())))),
        "the first core still answers on its port"
    );
    drop(second_server);
    drop(first_server);
}
