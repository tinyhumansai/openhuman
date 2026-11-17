use super::*;
#[cfg(feature = "storage-sqlite")]
use sha2::Digest;
use std::path::Path;
#[cfg(feature = "storage-sqlite")]
use std::sync::Arc;
#[cfg(feature = "storage-sqlite")]
use tinystoragedrivers::{CollectionSpec, Precondition, WriteOptions};

fn hex(id: &str) -> String {
    id.bytes().map(|b| format!("{b:02x}")).collect()
}

fn write_legacy(root: &Path, log: &[&str], threads: &[(&str, &[&str])]) {
    std::fs::create_dir_all(root.join("threads")).unwrap();
    std::fs::write(root.join("threads.jsonl"), log.join("\n") + "\n").unwrap();
    for (id, lines) in threads {
        std::fs::write(
            root.join("threads").join(format!("{}.jsonl", hex(id))),
            lines.join("\n") + "\n",
        )
        .unwrap();
    }
}

fn message(id: &str, content: &str, at: &str) -> String {
    format!(
        r#"{{"id":"{id}","content":"{content}","type":"text","extraMetadata":null,"sender":"user","createdAt":"{at}"}}"#
    )
}

fn upsert(id: &str, title: &str) -> String {
    format!(
        r#"{{"op":"upsert","thread_id":"{id}","title":"{title}","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}}"#
    )
}

fn snapshot(root: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    files.sort();
    files
}

#[test]
#[cfg(feature = "storage-sqlite")]
fn legacy_jsonl_conversations_are_imported_and_the_files_stay_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    crate::storage::local::use_persistent_default_for_test(dir.path());
    let root = backend::legacy_root(dir.path());
    let (m1, m2) = (
        message("m1", "hello", "2026-01-01T00:00:01Z"),
        message("m2", "world", "2026-01-01T00:00:02Z"),
    );
    let log = [
        upsert("t1", "First"),
        upsert("t2", "Doomed"),
        r#"{"op":"delete","thread_id":"t2","deleted_at":"2026-01-02T00:00:00Z"}"#.to_string(),
        upsert("t1", "Renamed"),
        r#"{"op":"message_appended","thread_id":"t1","last_message_at":"2026-01-01T00:00:02Z"}"#
            .to_string(),
    ];
    let log_refs: Vec<&str> = log.iter().map(String::as_str).collect();
    write_legacy(&root, &log_refs, &[("t1", &[m1.as_str(), m2.as_str()])]);
    let before = snapshot(&root);

    let store = ConversationStore::new(
        crate::core::runtime::DispatchCtx::ambient(),
        dir.path().to_path_buf(),
    );
    let threads = store.list_threads().unwrap();
    assert_eq!(threads.len(), 1, "the deleted thread is not imported");
    assert_eq!(threads[0].id, "t1");
    assert_eq!(threads[0].title, "Renamed");
    let messages = store.get_messages("t1").unwrap();
    let ids: Vec<&str> = messages.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["m1", "m2"]);
    assert_eq!(messages[1].content, "world");

    assert_eq!(snapshot(&root), before, "legacy files must not change");
}

#[cfg(feature = "storage-sqlite")]
#[test]
fn legacy_tsd_v1_threads_and_messages_are_migrated_before_owner_reads() {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = crate::storage::test_support::sqlite_backend().unwrap();
    let backend = Arc::clone(&sqlite.backend);
    let _installed =
        crate::storage::test_support::install_for_test(&sqlite.url, Arc::clone(&backend));
    let scope = crate::storage::Scope::local();
    let scoped = backend.for_scope(&scope).unwrap();
    let thread_id = "legacy-thread".to_owned();
    let incarnation = "6f9619ff-8b86-d011-b42d-00c04fc964ff".to_owned();
    let digest = hex::encode(sha2::Sha256::digest(thread_id.as_bytes()));
    let stream = format!("conversation_messages_v1:{digest}:{incarnation}");

    let legacy_thread_id = thread_id.clone();
    let legacy_incarnation = incarnation.clone();
    let legacy_digest = digest.clone();
    crate::storage::block_on(async move {
        let docs = scoped.documents();
        docs.ensure_collection(&CollectionSpec::new("conversation_threads_v1"))
            .await?;
        docs.put_with(
            "conversation_threads_v1",
            &legacy_digest,
            serde_json::json!({
                "schema": 1,
                "incarnation": legacy_incarnation,
                "deleted": false,
                "visible_end": 1,
                "thread": {
                    "id": legacy_thread_id,
                    "title": "Historical thread",
                    "isActive": true,
                    "messageCount": 1,
                    "lastMessageAt": "2026-01-02T00:00:02Z",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "labels": ["history"]
                }
            }),
            Precondition::Absent,
            WriteOptions::new().created_ms(1_700_000_000_000),
        )
        .await?;
        scoped
            .streams()
            .append(
                &stream,
                serde_json::json!({
                    "schema": 1,
                    "incarnation": incarnation,
                    "message": {
                        "id": "legacy-message",
                        "content": "message from the TSD v1 profile",
                        "type": "text",
                        "extraMetadata": {},
                        "sender": "user",
                        "createdAt": "2026-01-02T00:00:02Z"
                    }
                }),
            )
            .await?;
        Ok::<(), crate::storage::StorageError>(())
    })
    .unwrap();

    let store = ConversationStore::new(
        crate::core::runtime::DispatchCtx::ambient(),
        dir.path().to_path_buf(),
    );
    let listed = store.list_threads().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, thread_id);
    assert_eq!(listed[0].title, "Historical thread");
    let messages = store.get_messages(&thread_id).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, "legacy-message");
    assert_eq!(messages[0].content, "message from the TSD v1 profile");
}

#[test]
#[cfg(feature = "storage-sqlite")]
fn legacy_import_runs_once_even_when_the_files_change_afterwards() {
    let dir = tempfile::tempdir().unwrap();
    crate::storage::local::use_persistent_default_for_test(dir.path());
    let root = backend::legacy_root(dir.path());
    let m1 = message("m1", "hello", "2026-01-01T00:00:01Z");
    write_legacy(&root, &[&upsert("t1", "First")], &[("t1", &[m1.as_str()])]);

    let store = ConversationStore::new(
        crate::core::runtime::DispatchCtx::ambient(),
        dir.path().to_path_buf(),
    );
    assert_eq!(store.list_threads().unwrap().len(), 1);

    // A new process on the same workspace: the marker, not the process cache,
    // stops the second import.
    crate::storage::local::forget(dir.path());
    write_legacy(
        &root,
        &[&upsert("t1", "First"), &upsert("t9", "Late")],
        &[("t1", &[m1.as_str()])],
    );
    let after = ConversationStore::new(
        crate::core::runtime::DispatchCtx::ambient(),
        dir.path().to_path_buf(),
    );
    let threads = after.list_threads().unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(after.get_messages("t1").unwrap().len(), 1);
}

#[test]
fn a_workspace_without_legacy_files_starts_empty() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConversationStore::new(
        crate::core::runtime::DispatchCtx::ambient(),
        dir.path().to_path_buf(),
    );
    assert!(store.list_threads().unwrap().is_empty());
    assert!(store.get_messages("nope").unwrap().is_empty());
    assert!(!backend::legacy_root(dir.path()).exists());
}

#[test]
fn threads_round_trip_through_the_backend_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConversationStore::new(
        crate::core::runtime::DispatchCtx::ambient(),
        dir.path().to_path_buf(),
    );
    store
        .ensure_thread(CreateConversationThread {
            id: "t".into(),
            title: "T".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        })
        .unwrap();
    let appended = store
        .append_message(
            "t",
            ConversationMessage {
                id: "a".into(),
                content: "findable needle".into(),
                message_type: "text".into(),
                extra_metadata: serde_json::Value::Null,
                sender: "user".into(),
                created_at: "2026-01-01T00:00:01Z".into(),
            },
        )
        .unwrap();
    assert_eq!(appended.id, "a");
    assert_eq!(store.get_messages("t").unwrap().len(), 1);
    assert!(!store
        .search_cross_thread_messages("needle", 5, None)
        .unwrap()
        .is_empty());
    assert!(store.delete_thread("t", "2026-01-02T00:00:00Z").unwrap());
    assert!(store.list_threads().unwrap().is_empty());
}

#[test]
fn legacy_import_marker_is_per_source_workspace() {
    let a = crate::storage::source_key(Path::new("/ws/a"));
    let b = crate::storage::source_key(Path::new("/ws/b"));
    assert_ne!(a, b);
    assert_eq!(a, crate::storage::source_key(Path::new("/ws/a")));
}

#[cfg(unix)]
#[test]
fn an_unreadable_legacy_log_is_an_error_not_an_empty_store() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let root = backend::legacy_root(dir.path());
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("threads.jsonl"), "").unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o000)).unwrap();
    // Root bypasses permission bits; only assert when the stat truly fails.
    let blocked = std::fs::metadata(root.join("threads.jsonl")).is_err();
    let result = ConversationStore::new(
        crate::core::runtime::DispatchCtx::ambient(),
        dir.path().to_path_buf(),
    )
    .list_threads();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
    if blocked {
        assert!(
            result.is_err(),
            "a stat failure must surface so a retry can import"
        );
    }
}
