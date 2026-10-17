use super::*;

#[test]
fn creates_child_thread_linked_to_parent_and_seeds_prompt() {
    let dir = std::env::temp_dir().join(format!("wt-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();

    let id = create_worker_thread(
        &crate::core::runtime::DispatchCtx::ambient(),
        dir.clone(),
        "parent-thread-1",
        "researcher",
        "Q3",
        "Find Q3",
    )
    .expect("thread should be created");

    // The new thread is labelled `tasks` and linked to the parent so it
    // stays grouped with delegated task work in the chat sidebar.
    let threads =
        conversations::list_threads(&crate::core::runtime::DispatchCtx::ambient(), dir.clone())
            .unwrap();
    let thread = threads
        .iter()
        .find(|t| t.id == id)
        .expect("thread persisted");
    assert_eq!(thread.parent_thread_id.as_deref(), Some("parent-thread-1"));
    assert!(thread.labels.contains(&"tasks".to_string()));

    // It opens with the delegation prompt as the user message, so the
    // drawer can render the parent↔subagent chat from memory on reopen.
    let messages = conversations::get_messages(
        &crate::core::runtime::DispatchCtx::ambient(),
        dir.clone(),
        &id,
    )
    .unwrap();
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
        &crate::core::runtime::DispatchCtx::ambient(),
        dir.clone(),
        "parent-thread-1",
        "researcher",
        "Initial",
        "Initial prompt",
    )
    .unwrap();

    append_worker_user_message(
        &crate::core::runtime::DispatchCtx::ambient(),
        dir.clone(),
        &id,
        "researcher",
        "sub-2",
        "Follow-up prompt",
    )
    .unwrap();

    let messages = conversations::get_messages(
        &crate::core::runtime::DispatchCtx::ambient(),
        dir.clone(),
        &id,
    )
    .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].sender, "user");
    assert_eq!(messages[1].content, "Follow-up prompt");
    let _ = std::fs::remove_dir_all(dir);
}

fn profile_ctx(name: &str, workspace: &std::path::Path) -> crate::core::runtime::DispatchCtx {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    let config = crate::config::Config {
        workspace_dir: workspace.to_path_buf(),
        ..crate::config::Config::default()
    };
    DispatchCtx::new(
        CoreContext::for_test(DomainSet::full(), None).derive_with(
            ContextOverlay::new(
                config,
                DomainSet::kernel(),
                crate::tools::toolpacks::ToolGroups::none(),
            )
            .profile(name)
            .session_agent(name),
        ),
    )
}

#[tokio::test]
async fn worker_threads_follow_the_explicit_context_across_awaits() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let tmp = |tag: &str| {
        let dir = std::env::temp_dir().join(format!("wt-ctx-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    };
    let (dir_a, dir_b) = (tmp("a"), tmp("b"));
    let alice = profile_ctx(
        &format!("u-alice-{}", uuid::Uuid::new_v4().simple()),
        &dir_a,
    );
    let bob = profile_ctx(&format!("u-bob-{}", uuid::Uuid::new_v4().simple()), &dir_b);

    // No ambient scope here: each call acts under the context it was handed,
    // including after the task has yielded.
    let id_a = create_worker_thread(
        &alice,
        dir_a.clone(),
        "parent",
        "researcher",
        "A",
        "A prompt",
    )
    .unwrap();
    tokio::task::yield_now().await;
    let id_b =
        create_worker_thread(&bob, dir_b.clone(), "parent", "researcher", "B", "B prompt").unwrap();

    let ids = |ctx: &crate::core::runtime::DispatchCtx, dir: &std::path::Path| -> Vec<String> {
        conversations::list_threads(ctx, dir.to_path_buf())
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect()
    };
    assert!(ids(&alice, &dir_a).contains(&id_a));
    assert!(!ids(&alice, &dir_a).contains(&id_b));
    assert!(ids(&bob, &dir_b).contains(&id_b));
    assert!(!ids(&bob, &dir_b).contains(&id_a));
    let _ = std::fs::remove_dir_all(dir_a);
    let _ = std::fs::remove_dir_all(dir_b);
}
