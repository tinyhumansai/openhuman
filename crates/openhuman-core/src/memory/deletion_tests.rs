use super::*;

use tinymemory_api::{
    ItemKind, LearningKind, MemoryEngine, MemoryMeta, MetaFilter, Role, StoreItem, Turn,
};

use crate::memory::test_fixtures::{bind_reference, config_in, stored};

fn conversation(thread_id: &str, text: &str) -> StoreItem {
    StoreItem::Conversation {
        turns: vec![Turn::new(Role::User, text.to_string())],
        meta: MemoryMeta {
            thread_id: Some(thread_id.to_string()),
            ..MemoryMeta::default()
        },
    }
}

fn thread(id: &str) -> PendingDeletion {
    PendingDeletion::Thread {
        thread_id: id.to_string(),
    }
}

#[tokio::test]
async fn offline_conversation_purge_queues_exact_thread_ids_in_its_workspace() {
    use crate::threads::store::{ensure_thread, purge_threads, CreateConversationThread};

    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let workspace = config.workspace_dir.clone();
    let other_workspace = tmp.path().join("other-workspace");
    let ctx = crate::core::runtime::DispatchCtx::unbound();
    let create = |id: &str| CreateConversationThread {
        id: id.to_string(),
        title: format!("Title {id}"),
        created_at: "2026-10-18T00:00:00Z".to_string(),
        parent_thread_id: None,
        labels: None,
        personality_id: None,
        working_dir: None,
    };

    ensure_thread(&ctx, workspace.clone(), create("channel-thread-a")).unwrap();
    ensure_thread(&ctx, workspace.clone(), create("channel-thread-b")).unwrap();
    enqueue(&other_workspace, thread("other-workspace-thread"));

    let stats = purge_threads(&ctx, workspace.clone()).unwrap();

    assert_eq!(stats.thread_count, 2);
    assert_eq!(
        pending(&workspace)
            .into_iter()
            .collect::<std::collections::HashSet<_>>(),
        [thread("channel-thread-a"), thread("channel-thread-b")]
            .into_iter()
            .collect()
    );
    assert_eq!(
        pending(&other_workspace),
        vec![thread("other-workspace-thread")]
    );

    let engine = bind_reference(&config);
    engine
        .store(conversation("channel-thread-a", "temporary transcript A"))
        .await
        .unwrap();
    engine
        .store(conversation("channel-thread-b", "temporary transcript B"))
        .await
        .unwrap();
    engine
        .store(conversation("unrelated-thread", "keep this transcript"))
        .await
        .unwrap();
    engine
        .store(StoreItem::learning(
            "durable preference",
            LearningKind::Preference,
            0.9,
            MemoryMeta {
                thread_id: Some("channel-thread-a".to_string()),
                ..MemoryMeta::default()
            },
        ))
        .await
        .unwrap();
    engine
        .store(StoreItem::document(
            "durable brain document",
            MemoryMeta {
                thread_id: Some("channel-thread-a".to_string()),
                channel: Some("telegram".to_string()),
                ..MemoryMeta::default()
            },
        ))
        .await
        .unwrap();

    assert_eq!(drain(&config).await, 2);

    let left = stored(&engine, MetaFilter::default()).await;
    assert_eq!(left.len(), 3);
    assert!(left
        .iter()
        .any(|item| { item.kind == ItemKind::Learning && item.text == "durable preference" }));
    assert!(left
        .iter()
        .any(|item| { item.kind == ItemKind::Document && item.text == "durable brain document" }));
    assert!(left.iter().any(|item| {
        item.kind == ItemKind::Conversation
            && item.meta.thread_id.as_deref() == Some("unrelated-thread")
    }));
    assert!(!left.iter().any(|item| {
        item.kind == ItemKind::Conversation
            && matches!(
                item.meta.thread_id.as_deref(),
                Some("channel-thread-a" | "channel-thread-b")
            )
    }));
    assert!(pending(&workspace).is_empty());
    assert_eq!(
        pending(&other_workspace),
        vec![thread("other-workspace-thread")]
    );
}

#[tokio::test]
async fn a_deleted_threads_conversation_is_forgotten_for_good() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    engine
        .store(conversation("t-gone", "secret plan"))
        .await
        .unwrap();
    engine
        .store(conversation("t-kept", "keep me"))
        .await
        .unwrap();

    assert_eq!(forget_thread(&config, "t-gone").await, 1);

    let left = stored(&engine, MetaFilter::default()).await;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].meta.thread_id.as_deref(), Some("t-kept"));
    assert!(pending(&config.workspace_dir).is_empty());
}

#[tokio::test]
async fn a_thread_deleted_while_signed_out_is_forgotten_on_the_next_sign_in() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    // Memory off: no credential, no engine.
    assert_eq!(forget_thread(&config, "t-1").await, 0);
    assert_eq!(pending(&config.workspace_dir), vec![thread("t-1")]);
    // Nothing runs while memory stays off, and nothing is lost.
    assert_eq!(drain(&config).await, 0);
    assert_eq!(pending(&config.workspace_dir), vec![thread("t-1")]);

    // Signed in: the engine is reachable and holds the thread's memory.
    let engine = bind_reference(&config);
    engine
        .store(conversation("t-1", "from before"))
        .await
        .unwrap();
    assert_eq!(drain(&config).await, 1);
    assert!(stored(&engine, MetaFilter::default()).await.is_empty());
    assert!(pending(&config.workspace_dir).is_empty());
}

#[tokio::test]
async fn a_failed_forget_is_kept_for_the_next_drain() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    crate::memory::test_fixtures::RefusingEngine::out_of_credits().bind(&config);
    assert_eq!(forget_thread(&config, "t-2").await, 0);
    assert_eq!(pending(&config.workspace_dir), vec![thread("t-2")]);
    assert_eq!(drain(&config).await, 0);
    assert_eq!(pending(&config.workspace_dir), vec![thread("t-2")]);
}

#[test]
fn an_unparsable_legacy_queue_is_quarantined_and_retry_queue_remains_usable() {
    let tmp = tempfile::tempdir().unwrap();
    let file = crate::memory::state_store::legacy_path(tmp.path(), "pending_deletions");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "not json").unwrap();
    assert!(pending(tmp.path()).is_empty());
    enqueue(tmp.path(), thread("t"));
    assert_eq!(pending(tmp.path()), vec![thread("t")]);
    assert!(
        !file.exists(),
        "the corrupt legacy file is removed from the active path"
    );
    assert_eq!(
        std::fs::read_to_string(file.with_extension("json.corrupt")).unwrap(),
        "not json",
        "quarantine preserves the original bytes"
    );
}

#[test]
fn a_legacy_queue_file_is_imported_once() {
    let tmp = tempfile::tempdir().unwrap();
    let file = crate::memory::state_store::legacy_path(tmp.path(), "pending_deletions");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, serde_json::to_vec(&vec![thread("old")]).unwrap()).unwrap();
    assert_eq!(pending(tmp.path()), vec![thread("old")]);
    assert!(file.exists(), "the legacy file is not touched");
}

#[tokio::test]
async fn a_blank_thread_id_queues_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    assert_eq!(forget_thread(&config, "  ").await, 0);
    assert!(pending(&config.workspace_dir).is_empty());
}
