//! A detached sub-agent's result survives a restart and reaches the profile
//! that spawned it, through the real binary.
//!
//! The state under test is what a core leaves when it dies between a
//! sub-agent finishing and its result being delivered: a pending record in
//! the profile's completion queue (the `orchestration_completions` documents
//! of its scope in `<workspace>/storage`), addressed by thread id. The SaaS user
//! surface does not offer `spawn_subagent` (the `agent` domain is off
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
    CompletionRecord, CompletionResult, CompletionStatus, CompletionStore, DriverCompletionStore,
    NotifyMode,
};

use super::*;

/// Presents the host's storage backend as the storage-kit backend the
/// completion store opens, as the core's own adapter does.
struct KitBackend(Arc<dyn openhuman_core::storage::StorageBackend>);

impl tinyagents_storage_kit::StorageBackend for KitBackend {
    fn for_scope(
        &self,
        scope: &tinyagents_storage_kit::Scope,
    ) -> tinyagents_storage_kit::error::Result<tinyagents_storage_kit::ScopedStorage> {
        let host_scope = openhuman_core::storage::Scope::new(scope.as_str())?;
        let storage = self.0.for_scope(&host_scope)?;
        Ok(tinyagents_storage_kit::ScopedStorage::new(
            storage,
            scope.clone(),
        ))
    }

    fn for_scope_conditioned(
        &self,
        scope: &tinyagents_storage_kit::Scope,
        condition: &tinyagents_storage_kit::WriteCondition,
    ) -> tinyagents_storage_kit::error::Result<tinyagents_storage_kit::ScopedStorage> {
        let host_scope = openhuman_core::storage::Scope::new(scope.as_str())?;
        let storage = self.0.for_scope_conditioned(&host_scope, condition)?;
        Ok(tinyagents_storage_kit::ScopedStorage::new(
            storage,
            scope.clone(),
        ))
    }
}

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
/// completion queue.
fn leave_undelivered_result(d: &Deployment, user: &str) {
    let id = ProfileId::for_user(user, ProfileIdMode::Raw).unwrap();
    let workspace = ProfileLayout::new(&d.root, &id).workspace_dir;
    // SaaS nodes default to one deployment database with profile-scoped
    // collections; seed the same backend that ProfileHost reads on restart.
    let url = format!("sqlite:{}/saas-storage.db", d.tmp.path().display());
    std::fs::create_dir_all(workspace.join(openhuman_core::storage::config::DEFAULT_DIR)).unwrap();
    let backend = openhuman_core::storage::block_on_anyhow(async move {
        Ok(openhuman_core::storage::open(&url).await?)
    })
    .expect("open the profile's database");
    let scope = openhuman_core::storage::scope_for_profile(id.as_str());
    let scope = tinyagents_storage_kit::Scope::new(scope.as_str()).expect("the profile's scope");
    let store: Arc<dyn CompletionStore> = Arc::new(
        DriverCompletionStore::new(Arc::new(KitBackend(backend)), scope)
            .expect("open the completion queue"),
    );
    let record = CompletionRecord::new(
        "task-delivery-recovery-1",
        THREAD,
        "researcher",
        CompletionStatus::Success,
        CompletionResult::text(RESULT),
    )
    .with_notify_mode(NotifyMode::Followup);
    let gate = store
        .acquire_parent_gate(
            THREAD,
            "delivery-recovery-fixture",
            std::time::Duration::from_secs(30),
        )
        .expect("acquire the parent's completion gate")
        .expect("the completion gate is free");
    store
        .put_fenced(&record, THREAD, &gate)
        .expect("write the pending record under the parent gate");
    assert!(store
        .release_parent_gate(THREAD, &gate)
        .expect("release the fixture's parent gate"));
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
    leave_undelivered_result(&d, OWNER);

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
