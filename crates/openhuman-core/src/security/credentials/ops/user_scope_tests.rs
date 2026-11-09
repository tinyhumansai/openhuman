use super::*;

#[test]
fn explicit_scratch_workspace_cannot_activate_operator_user() {
    let _guard = crate::config::TEST_ENV_LOCK.blocking_lock();
    let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
    let scratch = tempfile::tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", scratch.path());
    }
    assert!(!operator_user_activation_allowed());
    match previous {
        Some(value) => unsafe { std::env::set_var("OPENHUMAN_WORKSPACE", value) },
        None => unsafe { std::env::remove_var("OPENHUMAN_WORKSPACE") },
    }
}

#[test]
fn first_user_activation_queues_prelogin_purge_in_the_removed_workspace() {
    use crate::memory::deletion::{pending, PendingDeletion};
    use crate::threads::store::{ensure_thread, list_threads, CreateConversationThread};

    let tmp = tempfile::tempdir().unwrap();
    let env = crate::config::test_env::EnvVarGuard::locked().with("HOME", tmp.path());

    let root = crate::config::default_root_openhuman_dir().unwrap();
    let _env = env.with("OPENHUMAN_WORKSPACE", &root);
    let prelogin_workspace = crate::config::pre_login_user_dir(&root).join("workspace");
    let active_workspace = crate::config::user_openhuman_dir(&root, "new-user").join("workspace");
    crate::storage::local::use_persistent_default_for_test(&prelogin_workspace);
    let ctx = crate::core::runtime::DispatchCtx::unbound();
    assert!(operator_user_activation_allowed());
    assert!(crate::config::read_active_user_id(&root).is_none());
    ensure_thread(
        &ctx,
        prelogin_workspace.clone(),
        CreateConversationThread {
            id: "prelogin-channel-thread".to_string(),
            title: "Pre-login thread".to_string(),
            created_at: "2026-10-18T00:00:00Z".to_string(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .unwrap();
    assert_eq!(
        list_threads(&ctx, prelogin_workspace.clone())
            .unwrap()
            .iter()
            .map(|thread| thread.id.as_str())
            .collect::<Vec<_>>(),
        vec!["prelogin-channel-thread"]
    );

    activate_user_scope(&ctx, "new-user").unwrap();

    assert!(list_threads(&ctx, prelogin_workspace.clone())
        .unwrap()
        .is_empty());
    assert_eq!(
        pending(&prelogin_workspace),
        vec![PendingDeletion::Thread {
            thread_id: "prelogin-channel-thread".to_string(),
        }]
    );
    assert!(pending(&active_workspace).is_empty());
}
