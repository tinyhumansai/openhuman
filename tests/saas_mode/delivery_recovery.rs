//! A detached sub-agent's result survives a restart and reaches the profile
//! that spawned it, through the real binary.
//!
//! The state under test is what a core leaves when it dies between a
//! sub-agent finishing and its result being delivered: a pending record in
//! the profile's completion log (`<workspace>/.openhuman/
//! background_completions.jsonl`), addressed by thread id. The SaaS user
//! surface does not offer `spawn_async_subagent` (the `agent` domain is off
//! for profiles), so the record is written here, through the harness's own
//! store, while the core is down, exactly as `record_completion` would have
//! written it.
//!
//! `ann` and `bob` both hold a thread named [`THREAD`]. After the restart
//! nothing is open: opening `bob` first must not deliver anything, and
//! opening `ann` (`ProfileHost::open` → `recover_on_open`) must deliver the
//! result into `ann`'s thread, under `ann`'s credential, and leave `bob`'s
//! thread of the same id untouched.

use std::sync::Arc;

use openhuman_core::profiles::{ProfileId, ProfileIdMode, ProfileLayout};
use tinyagents_tasks::{
    CompletionRecord, CompletionResult, CompletionStatus, CompletionStore, JsonlCompletionStore,
    NotifyMode,
};

use super::*;

const OWNER: &str = "ann";
const OTHER: &str = "bob";
const THREAD: &str = "shared-thread";
/// The sub-agent's result: unique, so it can be traced to every place it
/// lands.
const RESULT: &str = "subagent result delivery-ann-7c1e";

fn node_logging(d: &Deployment, backend: u16) -> Node {
    start_node_logging(
        d,
        "1",
        None,
        "",
        Some(backend),
        "info,openhuman_core::agent::orchestration=debug",
    )
}

fn rpc_ok(node: &Node, user: &str, method: &str, params: Value) -> Value {
    let (status, _, body) = call(node, user, method, params);
    assert_eq!(status, 200, "{user} {method}: {body}");
    assert!(body.get("result").is_some(), "{user} {method}: {body}");
    body
}

/// `user`'s messages on [`THREAD`], as JSON text.
fn messages(node: &Node, user: &str) -> String {
    let body = rpc_ok(
        node,
        user,
        "openhuman.threads_messages_list",
        json!({ "thread_id": THREAD }),
    );
    find_key(&body, "messages")
        .unwrap_or_else(|| panic!("{user} lists no messages: {body}"))
        .to_string()
}

/// `user`'s thread [`THREAD`] with one message of their own.
fn seed_thread(node: &Node, user: &str) {
    rpc_ok(
        node,
        user,
        "openhuman.threads_upsert",
        json!({ "id": THREAD, "title": format!("{user}'s thread"),
                "created_at": "2026-10-10T00:00:00Z" }),
    );
    rpc_ok(
        node,
        user,
        "openhuman.threads_message_append",
        json!({
            "thread_id": THREAD,
            "message": { "id": format!("{user}-msg-1"), "content": format!("{user}'s own words"),
                         "type": "text", "extraMetadata": {}, "sender": "user",
                         "createdAt": "2026-10-10T00:00:00Z" }
        }),
    );
}

/// The pending record a core killed before delivery leaves in `user`'s
/// completion log.
fn leave_undelivered_result(root: &std::path::Path, user: &str) {
    let id = ProfileId::for_user(user, ProfileIdMode::Raw).unwrap();
    let workspace = ProfileLayout::new(root, &id).workspace_dir;
    let log = workspace
        .join(".openhuman")
        .join("background_completions.jsonl");
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    let store: Arc<dyn CompletionStore> =
        Arc::new(JsonlCompletionStore::open(&log).expect("open the completion log"));
    let record = CompletionRecord::new(
        "task-delivery-recovery-1",
        THREAD,
        "researcher",
        CompletionStatus::Success,
        CompletionResult::text(RESULT),
    )
    .with_notify_mode(NotifyMode::Followup);
    store.put(&record).expect("write the pending record");
}

#[test]
fn an_undelivered_subagent_result_reaches_its_own_profile_after_a_restart() {
    let d = deployment(true);
    let llm = mock_llm::mock_llm();
    let mut node = node_logging(&d, llm.port);
    provision_with_credential(&node, OWNER);
    provision_with_credential(&node, OTHER);
    seed_thread(&node, OWNER);
    seed_thread(&node, OTHER);
    assert!(!messages(&node, OWNER).contains(RESULT));

    // The core dies with ann's sub-agent result recorded but undelivered.
    node.kill();
    leave_undelivered_result(&d.root, OWNER);

    // Restart. Opening bob, whose thread has the same id, delivers nothing.
    let node = node_logging(&d, llm.port);
    let bob_before = messages(&node, OTHER);
    assert!(bob_before.contains("bob's own words"), "{bob_before}");
    assert!(!bob_before.contains(RESULT), "{bob_before}");

    // Opening ann recovers the result into ann's thread.
    wait_until(
        "the recovered delivery into ann's thread",
        &node,
        Duration::from_secs(120),
        || messages(&node, OWNER).contains(RESULT),
    );
    let ann = messages(&node, OWNER);
    assert!(ann.contains("ann's own words"), "{ann}");

    // Let any stray drain land, then check bob's thread is as it was.
    std::thread::sleep(Duration::from_secs(5));
    let bob_after = messages(&node, OTHER);
    assert!(
        !bob_after.contains(RESULT),
        "ann's result reached bob's thread of the same id: {bob_after}"
    );
    assert_eq!(bob_before, bob_after, "recovery changed bob's thread");

    // Every inference request that carried the result ran as ann.
    let carriers: Vec<String> = llm
        .recorded()
        .into_iter()
        .filter(|r| r.is_inference() && r.body.contains(RESULT))
        .map(|r| r.auth)
        .collect();
    assert!(!carriers.is_empty(), "the delivery turn reached inference");
    for auth in &carriers {
        assert!(
            auth.contains(&format!("{OWNER}-jwt")),
            "ann's result went to inference under another credential"
        );
    }
}
