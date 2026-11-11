//! Eviction storm (leak d): with one free profile slot, two users take turns
//! evicting each other while a third user's turn stays live with a queued
//! follow-up. The live user's queue status, queue control and cancel must
//! keep working throughout, nobody else's cancel may reach the live turn, and
//! the churning users' state must survive each evict-and-reopen.

use super::mock_llm::{mock_llm, HANG};
use super::world::{canary, credential, USERS};
use super::*;

/// Rounds of the storm; each admits bob and carol once.
const ROUNDS: usize = 12;

fn queue(node: &Node, user: &str, thread: &str) -> Value {
    let (status, _, body) = call(
        node,
        user,
        "openhuman.channel_web_queue_status",
        json!({ "thread_id": thread }),
    );
    assert_eq!(
        status, 200,
        "{user}'s queue status during the storm: {body}"
    );
    body
}

#[test]
fn an_eviction_storm_keeps_live_turns_controllable() {
    let started = Instant::now();
    let d = deployment(true);
    let llm = mock_llm();
    let node = start_node_logging(
        &d,
        "1",
        None,
        "max_profiles_open = 2\nidle_evict_secs = 0\n",
        Some(llm.port),
        "info,openhuman_core::profiles=debug",
    );
    for (user, name) in USERS.iter().enumerate() {
        provision(&client(), &node.base, name);
        let body = operator(
            &node,
            "openhuman.profiles_set_credential",
            json!({ "profile_id": name, "kind": "session", "token": credential(user) }),
        );
        assert!(body.get("result").is_some(), "{body}");
    }
    let (alice, bob, carol) = (USERS[0], USERS[1], USERS[2]);

    // Alice's turn goes live, and a follow-up queues behind it.
    let (_, _, live) = call(
        &node,
        alice,
        "openhuman.channel_web_chat",
        json!({
            "client_id": "c-live", "thread_id": "live",
            "message": format!("{} {HANG}", canary(0, "live")),
        }),
    );
    let request = find_key(&live, "request_id")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{live}"))
        .to_string();
    wait_until("alice's live turn", &node, Duration::from_secs(60), || {
        active(&node, alice, "live")
    });
    let (_, _, body) = call(
        &node,
        alice,
        "openhuman.channel_web_chat",
        json!({
            "client_id": "c-live", "thread_id": "live",
            "message": canary(0, "followup"), "queue_mode": "followup",
        }),
    );
    assert!(body.get("result").is_some(), "{body}");
    let queued = queue(&node, alice, "live");
    assert!(
        queued.to_string().contains("CANARY-alice-followup"),
        "the follow-up is queued: {queued}"
    );

    // The storm: bob and carol share the one free slot.
    let (mut opened, mut refused) = (0, 0);
    let mut written: [Vec<String>; 2] = Default::default();
    for round in 0..ROUNDS {
        for (i, user) in [bob, carol].into_iter().enumerate() {
            let thread = format!("storm-{round}");
            // The slot frees once the other user's work is done; a profile
            // that stays busy past the deadline is pinned (leak d).
            let deadline = Instant::now() + Duration::from_secs(20);
            let body = loop {
                let (status, _, body) = call(
                    &node,
                    user,
                    "openhuman.threads_upsert",
                    json!({
                        "id": thread, "title": canary(i + 1, &thread), "created_at": "2026-10-10T00:00:00Z"
                    }),
                );
                match status {
                    200 => break body,
                    503 if Instant::now() < deadline => {
                        refused += 1;
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    other => panic!(
                        "{user} never got a profile slot back (HTTP {other}, round {round}): {body}\n{}",
                        node.log_tail()
                    ),
                }
            };
            assert!(body.get("result").is_some(), "{body}");
            opened += 1;
            written[i].push(thread.clone());
            if round % 3 == 0 {
                // A short turn of their own, which pins the profile while it
                // runs and must stop pinning it once it is over.
                call(
                    &node,
                    user,
                    "openhuman.channel_web_chat",
                    json!({
                        "client_id": "c1", "thread_id": thread, "message": canary(i + 1, "storm")
                    }),
                );
                // Give the turn a moment to start (it may also have finished
                // already) so the wait below cannot pass before it began.
                let start = Instant::now();
                while start.elapsed() < Duration::from_secs(3) && !active(&node, user, &thread) {
                    std::thread::sleep(Duration::from_millis(50));
                }
                wait_until(
                    "a churning user's short turn",
                    &node,
                    Duration::from_secs(30),
                    || !active(&node, user, &thread),
                );
            }
            // Aimed at alice's turn: a no-op in their own profile.
            let (_, _, body) = call(
                &node,
                user,
                "openhuman.channel_web_cancel",
                json!({ "client_id": "c-live", "thread_id": "live", "request_id": request }),
            );
            assert!(
                !body.to_string().contains("\"cancelled\":true"),
                "{user} cancelled alice: {body}"
            );
            let status = queue(&node, alice, "live");
            assert!(
                status.to_string().contains("\"active\":true"),
                "alice's live turn is still visible mid-storm (round {round}): {status}\n{}",
                node.log_tail()
            );
        }
    }
    let log = std::fs::read_to_string(&node.log).unwrap_or_default();
    let evictions = log.matches("[profiles] evicted").count();
    eprintln!("[isolation] storm: {opened} opens, {refused} busy refusals, {evictions} evictions");
    // Each admission after the first evicted the other churner.
    assert!(
        evictions >= 2 * ROUNDS - 1,
        "the storm evicted profiles ({evictions})\n{}",
        node.log_tail()
    );
    assert!(
        !log.contains("evicted least recently used profile=alice")
            && !log.contains("evicted idle profile=alice"),
        "alice was evicted mid-turn"
    );

    // Alice still controls her queue and her turn.
    let status = queue(&node, alice, "live");
    if let Some(item) = find_key(&status, "id").and_then(Value::as_str) {
        let (_, _, body) = call(
            &node,
            alice,
            "openhuman.channel_web_queue_remove",
            json!({ "client_id": "c-live", "thread_id": "live", "item_id": item }),
        );
        assert!(
            body.get("result").is_some(),
            "alice removes her queued follow-up: {body}"
        );
    }
    let (_, _, body) = call(
        &node,
        alice,
        "openhuman.channel_web_cancel",
        json!({ "client_id": "c-live", "thread_id": "live", "request_id": request }),
    );
    assert!(
        body.to_string().contains("\"cancelled\":true"),
        "alice cancels her turn: {body}"
    );
    wait_until(
        "alice's turn to stop",
        &node,
        Duration::from_secs(30),
        || !active(&node, alice, "live"),
    );

    // Bob's and carol's writes survived every evict-and-reopen.
    for (i, user) in [bob, carol].into_iter().enumerate() {
        let (_, _, body) = call(&node, user, "openhuman.threads_list", json!({}));
        let text = body.to_string();
        for thread in &written[i] {
            assert!(
                text.contains(&format!("\"{thread}\"")),
                "{user} lost {thread}: {text}"
            );
        }
        assert!(world::foreign_in(&text, i + 1).is_empty(), "{text}");
    }
    eprintln!(
        "[isolation] storm took {:.1}s",
        started.elapsed().as_secs_f64()
    );
}
