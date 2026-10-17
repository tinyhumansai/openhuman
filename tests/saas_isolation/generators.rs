//! One parameter generator per user-surface method. Each writes the actor's
//! own canaries and aims at ids other users' responses revealed.
//!
//! [`GENERATORS`] must name exactly `profiles::surface::USER_METHODS`: the
//! suite fails when a method is added to the surface without a generator
//! here, so every new user method is fuzzed from day one.

use super::world::{World, VICTIM};
use super::*;

pub type Generator = fn(&mut World, usize) -> Value;

const NOW: &str = "2026-10-10T00:00:00Z";

pub const GENERATORS: &[(&str, Generator)] = &[
    ("openhuman.threads_list", |_, _| json!({})),
    ("openhuman.threads_upsert", |w, a| {
        let title = w.canary(a);
        let mut params = json!({ "id": w.thread_for(a, true), "title": title, "created_at": NOW });
        if w.rng.chance(30) {
            params["labels"] = json!([w.canary(a)]);
        }
        params
    }),
    (
        "openhuman.threads_delete",
        |w, a| json!({ "thread_id": w.thread_for(a, true), "deleted_at": NOW }),
    ),
    ("openhuman.threads_purge", |_, _| json!({})),
    ("openhuman.threads_create_new", |w, a| {
        match w.rng.below(6) {
            // A SaaS user may not point a thread at a host folder.
            0 => json!({ "action_dir": "/etc" }),
            1 => json!({ "action_dir": format!("../../{}/workspace", w.other(a)) }),
            _ => json!({ "labels": [w.canary(a)] }),
        }
    }),
    (
        "openhuman.threads_messages_list",
        |w, a| json!({ "thread_id": w.thread_for(a, false) }),
    ),
    ("openhuman.threads_message_append", |w, a| {
        let id = if w.rng.chance(40) {
            w.message_for(a)
        } else {
            format!("m-{}", w.next_id())
        };
        json!({
            "thread_id": w.thread_for(a, true),
            "message": {
                "id": id,
                "content": w.canary(a),
                "type": "text",
                "extraMetadata": {},
                "sender": "user",
                "createdAt": NOW,
            }
        })
    }),
    ("openhuman.threads_message_update", |w, a| {
        json!({
            "thread_id": w.thread_for(a, true),
            "message_id": w.message_for(a),
            "extra_metadata": { "note": w.canary(a) },
        })
    }),
    (
        "openhuman.threads_update_labels",
        |w, a| json!({ "thread_id": w.thread_for(a, true), "labels": [w.canary(a)] }),
    ),
    (
        "openhuman.threads_update_title",
        |w, a| json!({ "thread_id": w.thread_for(a, true), "title": w.canary(a) }),
    ),
    (
        "openhuman.threads_turn_state_get",
        |w, a| json!({ "thread_id": w.thread_for(a, false) }),
    ),
    ("openhuman.threads_turn_state_list", |_, _| json!({})),
    (
        "openhuman.threads_turn_state_history",
        |w, a| json!({ "thread_id": w.thread_for(a, false) }),
    ),
    ("openhuman.threads_turn_state_get_turn", |w, a| {
        let (request, thread) = w.request_on_thread(a);
        json!({ "thread_id": thread, "request_id": request })
    }),
    (
        "openhuman.threads_turn_state_clear",
        |w, a| json!({ "thread_id": w.thread_for(a, true) }),
    ),
    (
        "openhuman.threads_token_usage",
        |w, a| json!({ "thread_id": w.thread_for(a, false) }),
    ),
    (
        "openhuman.threads_transcript_get",
        |w, a| json!({ "thread_id": w.thread_for(a, false), "limit": 50 }),
    ),
    (
        "openhuman.threads_goal_get",
        |w, a| json!({ "thread_id": w.thread_for(a, false) }),
    ),
    (
        "openhuman.threads_todos_get",
        |w, a| json!({ "thread_id": w.thread_for(a, false) }),
    ),
    (
        "openhuman.threads_generate_title",
        |w, a| json!({ "thread_id": w.thread_for(a, true), "assistant_message": w.canary(a) }),
    ),
    ("openhuman.threads_edit_message", |w, a| {
        json!({
            "thread_id": w.thread_for(a, true),
            "message_id": w.message_for(a),
            "content": w.canary(a),
            "client_id": w.client_for(a),
        })
    }),
    ("openhuman.threads_regenerate", |w, a| {
        json!({
            "thread_id": w.thread_for(a, true),
            "message_id": w.message_for(a),
            "client_id": w.client_for(a),
        })
    }),
    ("openhuman.channel_web_chat", |w, a| {
        let mut message = w.canary(a);
        let thread = w.thread_for(a, true);
        // Now and then a turn of the actor's own stays in flight, so others
        // have live turns and queue items to aim at.
        if a != VICTIM && w.rng.chance(4) {
            message.push_str(&format!(" {}", mock_llm::HANG));
        }
        let mut params =
            json!({ "client_id": w.client_for(a), "thread_id": thread, "message": message });
        let mode = ["interrupt", "steer", "followup", "collect"][w.rng.below(4)];
        if w.rng.chance(60) {
            params["queue_mode"] = json!(mode);
        }
        params
    }),
    ("openhuman.channel_web_cancel", |w, a| {
        if w.rng.chance(70) {
            let (request, thread) = w.request_on_thread(a);
            json!({ "client_id": w.client_for(a), "thread_id": thread, "request_id": request })
        } else {
            json!({ "client_id": w.client_for(a), "thread_id": w.thread_for(a, true) })
        }
    }),
    (
        "openhuman.channel_web_queue_status",
        |w, a| json!({ "thread_id": w.thread_for(a, false) }),
    ),
    (
        "openhuman.channel_web_queue_clear",
        |w, a| json!({ "thread_id": w.thread_for(a, true) }),
    ),
    ("openhuman.channel_web_queue_remove", |w, a| {
        json!({
            "client_id": w.client_for(a),
            "thread_id": w.thread_for(a, true),
            "item_id": w.queue_item_for(a),
        })
    }),
    ("openhuman.channel_relay_inbound", |w, a| {
        let text = w.canary(a);
        // Replay the platform ids another user relayed: in this user's
        // profile they are a new message on a new thread, never a duplicate.
        let others: Vec<[String; 4]> = w
            .pools
            .relays
            .iter()
            .filter(|(owner, _)| *owner != a)
            .map(|(_, key)| key.clone())
            .collect();
        let [channel, chat, sender, message] = if !others.is_empty() && w.rng.chance(50) {
            others[w.rng.below(others.len())].clone()
        } else {
            let n = w.next_id();
            [
                "telegram".to_string(),
                format!("chat-{n}"),
                format!("s-{}", n % 3),
                format!("tg-{n}"),
            ]
        };
        let mut params = json!({
            "channel": channel, "chat_id": chat, "sender_id": sender,
            "message_id": message, "text": text,
        });
        if w.rng.chance(30) {
            params["client_id"] = json!(w.client_for(a));
        }
        params
    }),
    (
        "openhuman.memory_recall",
        |w, a| json!({ "question": w.canary(a) }),
    ),
    (
        "openhuman.memory_fetch",
        |w, a| json!({ "query": w.canary(a), "limit": 5 }),
    ),
    (
        "openhuman.memory_learn",
        |w, a| json!({ "text": w.canary(a) }),
    ),
    (
        "openhuman.memory_forget",
        |w, a| json!({ "ids": [w.memory_id_for(a)] }),
    ),
    // Sub-agent ids are minted by live turns the mock model cannot spawn, so
    // these aim at ids no profile owns: another profile's or an unknown task
    // must answer the same "nothing running" and never touch a worker.
    (
        "openhuman.subagent_cancel",
        |w, a| json!({ "taskId": format!("sub-{}", w.next_id()), "reason": w.canary(a) }),
    ),
    ("openhuman.subagent_steer", |w, a| {
        let mode = ["steer", "collect"][w.rng.below(2)];
        json!({
            "taskId": format!("sub-{}", w.next_id()),
            "message": w.canary(a),
            "mode": mode,
        })
    }),
    ("openhuman.memory_items_list", |_, _| json!({ "limit": 20 })),
    ("openhuman.memory_explore", |w, _| {
        let facet = ["kind", "source", "workspace", "folder"][w.rng.below(4)];
        json!({ "facet": facet })
    }),
];

impl World {
    /// Some user other than `actor`.
    pub fn other(&mut self, actor: usize) -> &'static str {
        let n = super::world::USERS.len();
        super::world::USERS[(actor + 1 + self.rng.below(n - 1)) % n]
    }
}

/// Fail unless [`GENERATORS`] covers exactly the user surface.
pub fn assert_covers_user_surface() {
    use openhuman_core::profiles::surface::USER_METHODS;
    let ours: Vec<&str> = GENERATORS.iter().map(|(m, _)| *m).collect();
    let missing: Vec<&&str> = USER_METHODS.iter().filter(|m| !ours.contains(m)).collect();
    let stale: Vec<&&str> = ours.iter().filter(|m| !USER_METHODS.contains(m)).collect();
    let mut seen = std::collections::HashSet::new();
    let duplicate: Vec<&&str> = ours.iter().filter(|m| !seen.insert(**m)).collect();
    assert!(
        duplicate.is_empty(),
        "duplicate isolation-fuzz generators: {duplicate:?}"
    );
    assert!(
        missing.is_empty(),
        "USER_METHODS entries with no isolation-fuzz generator (add one to \
         tests/saas_isolation/generators.rs): {missing:?}"
    );
    assert!(
        stale.is_empty(),
        "generators for methods no longer on the user surface: {stale:?}"
    );
}
