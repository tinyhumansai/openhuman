//! Adversarial isolation suite for SaaS mode, through the real binary.
//!
//! One SaaS core hosts three users. Each writes canaries (`CANARY-<user>-…`)
//! everywhere the user surface reaches: threads, messages, web and relayed
//! chat turns, memory, and their backend credential. A seeded random driver
//! then calls **every** `USER_METHODS` entry as a random user, with ids
//! harvested from *other* users' responses: their thread ids, request ids,
//! client ids, message and queue-item ids, relayed platform ids.
//!
//! Afterwards no user's responses or `/events` frames, and no file under one
//! user's `users/<id>/` tree, may hold another user's canary; every backend
//! request must carry the credential of the user whose content it holds; and
//! the users' protected state (a sentinel thread each, and one user's turn,
//! live through the whole run) must be untouched.
//!
//! `OH_FUZZ_SEED` and `OH_FUZZ_STEPS` (default 200) pick the run; a failure
//! prints the seed and the replay command.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

#[path = "saas_mode/support.rs"]
mod support;
use support::*;

#[path = "saas_mode/mock_llm.rs"]
mod mock_llm;

#[path = "saas_isolation/generators.rs"]
mod generators;
#[path = "saas_isolation/probes.rs"]
mod probes;
#[path = "saas_isolation/scan.rs"]
mod scan;
#[path = "saas_isolation/storm.rs"]
mod storm;
#[path = "saas_isolation/world.rs"]
mod world;

use generators::{assert_covers_user_surface, GENERATORS};
use world::{canary, credential, foreign_in, seed_from_env, steps_from_env, World, USERS, VICTIM};

const LIVE_THREAD: &str = "live-alice";
const LIVE_CLIENT: &str = "c-live";

fn keep_thread(user: usize) -> String {
    format!("keep-{}", USERS[user])
}

/// Provision every user with their own credential.
fn provision_users(node: &Node) {
    for (user, name) in USERS.iter().enumerate() {
        provision(&client(), &node.base, name);
        let body = operator(
            node,
            "openhuman.profiles_set_credential",
            json!({ "profile_id": name, "kind": "session", "token": credential(user) }),
        );
        assert!(body.get("result").is_some(), "{body}");
        assert!(
            !body.to_string().contains("CANARY"),
            "a credential is never echoed: {body}"
        );
    }
}

/// Call `method` as `actor`, check the response for other users' canaries,
/// and harvest its ids.
fn act(w: &mut World, actor: usize, method: &str, params: Value) -> Value {
    let (status, _, body) = call(&w.node, USERS[actor], method, params.clone());
    let text = body.to_string();
    let short: String = text.chars().take(160).collect();
    // Parameter names only: the values are user content and credentials.
    let keys: Vec<&str> = params
        .as_object()
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default();
    w.trace.push(format!(
        "#{} {} {method} {keys:?} -> {status} {short}",
        w.step, USERS[actor]
    ));
    let stat = w.stats.entry(method.to_string()).or_default();
    match (status, body.get("result").is_some()) {
        (200, true) => stat.0 += 1,
        (200, false) => {
            stat.1 += 1;
            let message = body["error"]["message"].as_str().unwrap_or_default();
            w.first_error
                .entry(method.to_string())
                .or_insert_with(|| message.chars().take(90).collect());
        }
        _ => stat.2 += 1,
    }
    if status != 200 {
        w.fail(&format!(
            "{method} as {} answered HTTP {status}: {body}",
            USERS[actor]
        ));
    }
    let leaked = foreign_in(&text, actor);
    if !leaked.is_empty() {
        w.fail(&format!(
            "{}'s {method} response holds {leaked:?}'s canary: {text}",
            USERS[actor]
        ));
    }
    check_response(w, actor, method, &params, &body);
    w.harvest(actor, method, &params, &body);
    body
}

/// The checks a single response allows on its own.
fn check_response(w: &mut World, actor: usize, method: &str, params: &Value, body: &Value) {
    let text = body.to_string();
    if method == "openhuman.channel_web_cancel" {
        if let Some(request) = params["request_id"].as_str() {
            let theirs = w
                .pools
                .requests
                .iter()
                .any(|o| o.owner != actor && o.value == request);
            let ours = w
                .pools
                .requests
                .iter()
                .any(|o| o.owner == actor && o.value == request);
            if theirs && !ours && text.contains("\"cancelled\":true") {
                w.fail(&format!(
                    "{} cancelled another user's request {request}: {text}",
                    USERS[actor]
                ));
            }
        }
    }
    if method == "openhuman.channel_relay_inbound" {
        let key = ["channel", "chat_id", "sender_id", "message_id"]
            .map(|k| params[k].as_str().unwrap_or_default().to_string());
        let joined = key.join("/");
        if text.contains("\"duplicate\":true") && !w.relayed[actor].contains(&joined) {
            w.fail(&format!(
                "{}'s first relay of {joined} reads as a duplicate: another profile's relay \
                 dedupe is visible to it: {text}",
                USERS[actor]
            ));
        }
        if text.contains("\"accepted\":true") || text.contains("\"duplicate\":true") {
            w.relayed[actor].insert(joined);
            w.pools.relays.push((actor, key));
        }
    }
}

/// A sentinel thread no one but `user` may change, and that `user` never
/// writes to again.
fn seed_keep(w: &mut World, user: usize) {
    let thread = keep_thread(user);
    w.protected[user].insert(thread.clone());
    act(
        w,
        user,
        "openhuman.threads_upsert",
        json!({ "id": thread, "title": canary(user, "keep-title"), "created_at": "2026-10-10T00:00:00Z" }),
    );
    act(
        w,
        user,
        "openhuman.threads_message_append",
        json!({
            "thread_id": thread,
            "message": { "id": format!("keep-msg-{}", USERS[user]), "content": canary(user, "keep"),
                         "type": "text", "extraMetadata": {}, "sender": "user",
                         "createdAt": "2026-10-10T00:00:00Z" }
        }),
    );
}

/// Every kind of state the user surface writes, once per user, plus the
/// victim's live turn.
fn seed_state(w: &mut World) {
    for user in 0..USERS.len() {
        for client in ["c1", "c2", "channel-relay", LIVE_CLIENT] {
            w.listen(user, client);
        }
    }
    for (user, name) in USERS.iter().enumerate() {
        seed_keep(w, user);
        let message = canary(user, "hello");
        act(
            w,
            user,
            "openhuman.channel_web_chat",
            json!({ "client_id": "c1", "thread_id": format!("chat-{name}"), "message": message }),
        );
        let params = json!({ "channel": "telegram", "chat_id": "777", "sender_id": "555",
                             "message_id": "tg-1", "text": canary(user, "relayed") });
        act(w, user, "openhuman.channel_relay_inbound", params);
        act(
            w,
            user,
            "openhuman.memory_learn",
            json!({ "text": canary(user, "memory") }),
        );
    }
    let body = act(
        w,
        VICTIM,
        "openhuman.channel_web_chat",
        json!({
            "client_id": LIVE_CLIENT, "thread_id": LIVE_THREAD,
            "message": format!("{} {}", canary(VICTIM, "live"), mock_llm::HANG),
        }),
    );
    let request = find_key(&body, "request_id")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("no request_id for the live turn: {body}"))
        .to_string();
    w.protected[VICTIM].insert(LIVE_THREAD.to_string());
    w.protected_requests[VICTIM].insert(request);
    let node = &w.node;
    wait_until(
        "the victim's live turn",
        node,
        Duration::from_secs(60),
        || active(node, USERS[VICTIM], LIVE_THREAD),
    );
}

fn drive(w: &mut World, steps: usize) {
    let mut order: Vec<usize> = (0..GENERATORS.len()).collect();
    w.rng.shuffle(&mut order);
    for step in 0..steps {
        w.step = step;
        let index = if step < order.len() {
            order[step]
        } else {
            w.rng.below(GENERATORS.len())
        };
        let (method, generate) = GENERATORS[index];
        let mut actor = w.rng.below(USERS.len());
        if method == "openhuman.threads_purge" && actor == VICTIM {
            // The victim's own purge would end their live turn.
            actor = 1 + w.rng.below(USERS.len() - 1);
        }
        if w.streams < 24 && w.rng.chance(3) {
            let client = w.client_for(actor);
            w.listen(actor, &client);
        }
        let params = generate(w, actor);
        let body = act(w, actor, method, params);
        if method == "openhuman.threads_purge" && body.get("result").is_some() {
            seed_keep(w, actor);
        }
    }
}

/// Wait until the backend has seen no new request for `quiet`.
fn settle(llm: &mock_llm::MockLlm, quiet: Duration, max: Duration) {
    let deadline = Instant::now() + max;
    let mut last = (llm.recorded().len(), Instant::now());
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(250));
        let n = llm.recorded().len();
        if n != last.0 {
            last = (n, Instant::now());
        } else if last.1.elapsed() >= quiet {
            return;
        }
    }
    panic!(
        "backend traffic never settled within {max:?} ({} requests recorded)",
        llm.recorded().len()
    );
}

fn verify_victims(w: &mut World) {
    for (user, name) in USERS.iter().enumerate() {
        let body = act(
            w,
            user,
            "openhuman.threads_messages_list",
            json!({ "thread_id": keep_thread(user) }),
        );
        if !body.to_string().contains(&canary(user, "keep")) {
            w.fail(&format!(
                "{name}'s sentinel thread lost its message: {body}"
            ));
        }
    }
    let status = act(
        w,
        VICTIM,
        "openhuman.channel_web_queue_status",
        json!({ "thread_id": LIVE_THREAD }),
    );
    if !status.to_string().contains("\"active\":true") {
        let state = act(
            w,
            VICTIM,
            "openhuman.threads_turn_state_get",
            json!({ "thread_id": LIVE_THREAD }),
        );
        w.fail(&format!(
            "the victim's live turn ended during the run: {status}\nturn state: {state}"
        ));
    }
    let request = w.protected_requests[VICTIM].iter().next().cloned().unwrap();
    let body = act(
        w,
        VICTIM,
        "openhuman.channel_web_cancel",
        json!({ "client_id": LIVE_CLIENT, "thread_id": LIVE_THREAD, "request_id": request }),
    );
    if !body.to_string().contains("\"cancelled\":true") {
        w.fail(&format!(
            "the victim cannot cancel their own live turn: {body}"
        ));
    }
}

fn verify_frames(w: &World) {
    let frames = w.frames.lock().unwrap().clone();
    let mut own = [0usize; 3];
    for (user, client, data) in &frames {
        let leaked = foreign_in(data, *user);
        if !leaked.is_empty() {
            w.fail(&format!(
                "{}'s /events?client_id={client} carried {leaked:?}'s data: {data}",
                USERS[*user]
            ));
        }
        if data.contains(&format!("CANARY-{}-", USERS[*user])) {
            own[*user] += 1;
        }
    }
    eprintln!(
        "[isolation] /events frames: {} total, own-canary frames per user {own:?}",
        frames.len()
    );
    assert!(
        own.iter().all(|&n| n > 0),
        "every user's stream saw their own turns: {own:?}"
    );
}

#[test]
fn random_cross_profile_calls_never_leak() {
    assert_covers_user_surface();
    let started = Instant::now();
    let (seed, steps) = (seed_from_env(), steps_from_env());
    eprintln!("[isolation] seed {seed} steps {steps} (OH_FUZZ_SEED / OH_FUZZ_STEPS)");
    let d = deployment(true);
    let llm = mock_llm::mock_llm();
    let node = start_node(
        &d,
        "1",
        None,
        "max_profiles_open = 3\nidle_evict_secs = 2\n",
        Some(llm.port),
    );
    provision_users(&node);
    let mut w = World::new(node, seed);
    seed_state(&mut w);
    let booted = started.elapsed();
    drive(&mut w, steps);
    let driven = started.elapsed();
    settle(&llm, Duration::from_secs(2), Duration::from_secs(30));
    verify_victims(&mut w);
    verify_frames(&w);

    let (violations, per_user) = scan::audit_backend(&llm.recorded());
    if !violations.is_empty() {
        w.fail(&format!(
            "backend requests crossed profiles:\n{}",
            violations.join("\n")
        ));
    }
    assert!(
        per_user.iter().all(|&n| n > 0),
        "every user reached inference: {per_user:?}"
    );
    let (violations, own) =
        scan::scan_files(&d.root, d.tmp.path(), &d.tmp.path().join("saas-storage.db"));
    if !violations.is_empty() {
        w.fail(&format!(
            "files hold other profiles' canaries:\n{}",
            violations.join("\n")
        ));
    }
    // `verify_victims` above proves each user's persisted data remains
    // readable through its tenant scope. The shared SQLite file contains all
    // tenants' bytes, so `scan_files` audits user trees and other host files,
    // while this RPC audit checks shared storage ownership.

    let summary: Vec<String> = w
        .stats
        .iter()
        .map(|(m, (ok, err, other))| {
            let first = w.first_error.get(m).map(String::as_str).unwrap_or("");
            format!("  {m}: ok {ok} err {err} other {other} {first}")
        })
        .collect();
    eprintln!(
        "[isolation] seed {seed}: {steps} steps; boot+seed {:.1}s, drive {:.1}s, total {:.1}s; \
         inference per user {per_user:?}; own files {own:?}\n{}",
        booted.as_secs_f64(),
        (driven - booted).as_secs_f64(),
        started.elapsed().as_secs_f64(),
        summary.join("\n")
    );
}
