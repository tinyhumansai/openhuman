use super::*;

use tinymemory_api::{MemoryEngine, MemoryMeta, Role, StoreItem, Turn};

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
fn an_unparsable_queue_is_set_aside_not_lost() {
    let tmp = tempfile::tempdir().unwrap();
    let file = path(tmp.path());
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "not json").unwrap();
    assert!(pending(tmp.path()).is_empty());
    assert!(file.with_extension("json.corrupt").exists());
}

#[tokio::test]
async fn a_blank_thread_id_queues_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    assert_eq!(forget_thread(&config, "  ").await, 0);
    assert!(pending(&config.workspace_dir).is_empty());
}

#[cfg(unix)]
#[test]
fn the_pending_queue_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    enqueue(tmp.path(), thread("t"));
    let mode = std::fs::metadata(path(tmp.path()))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0, "{mode:o}");
}
