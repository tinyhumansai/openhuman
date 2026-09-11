use super::*;

#[test]
fn creates_child_thread_linked_to_parent_and_seeds_prompt() {
    let dir = std::env::temp_dir().join(format!("wt-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();

    let id = create_worker_thread(
        dir.clone(),
        "parent-thread-1",
        "researcher",
        "Q3",
        "Find Q3",
    )
    .expect("thread should be created");

    // The new thread is labelled `tasks` and linked to the parent so it
    // stays grouped with delegated task work in the chat sidebar.
    let threads = conversations::list_threads(dir.clone()).unwrap();
    let thread = threads
        .iter()
        .find(|t| t.id == id)
        .expect("thread persisted");
    assert_eq!(thread.parent_thread_id.as_deref(), Some("parent-thread-1"));
    assert!(thread.labels.contains(&"tasks".to_string()));

    // It opens with the delegation prompt as the user message, so the
    // drawer can render the parent↔subagent chat from memory on reopen.
    let messages = conversations::get_messages(dir.clone(), &id).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].sender, "user");
    assert_eq!(messages[0].content, "Find Q3");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn appends_follow_up_prompt_to_existing_worker_thread() {
    let dir = std::env::temp_dir().join(format!("wt-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let id = create_worker_thread(
        dir.clone(),
        "parent-thread-1",
        "researcher",
        "Initial",
        "Initial prompt",
    )
    .unwrap();

    append_worker_user_message(dir.clone(), &id, "researcher", "sub-2", "Follow-up prompt")
        .unwrap();

    let messages = conversations::get_messages(dir.clone(), &id).unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].sender, "user");
    assert_eq!(messages[1].content, "Follow-up prompt");
    let _ = std::fs::remove_dir_all(dir);
}
