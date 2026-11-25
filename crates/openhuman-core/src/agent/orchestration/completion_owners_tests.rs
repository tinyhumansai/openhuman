use super::*;

fn owner(task_id: &str) -> DriverTaskOwner {
    let workspace_dir = PathBuf::from("/workspace");
    DriverTaskOwner {
        task_id: task_id.to_owned(),
        profile: None,
        workspace: tinytools::WorkspaceDescriptor::new(workspace_dir.clone()),
        workspace_dir,
        parent_session: "parent".into(),
        parent_thread_id: Some("thread".into()),
    }
}

#[test]
fn terminal_tasks_leave_the_non_evictable_live_owner_set() {
    let mut st = Owners::default();
    st.driver_tasks.insert("task".into(), owner("task"));

    terminalize_driver_task(&mut st, "task", 2);

    assert!(!st.driver_tasks.contains_key("task"));
    assert!(st.terminal_driver_tasks.contains_key("task"));
}

#[test]
fn active_owner_capacity_refuses_new_tasks_without_evicting_live_scope() {
    let mut st = Owners::default();
    for index in 0..ACTIVE_DRIVER_TASKS_CAP {
        let key = format!("task-{index}");
        st.driver_tasks.insert(key.clone(), owner(&key));
    }

    let result = remember_active_driver_task(
        &mut st,
        "new-task".into(),
        owner("new-task"),
        ACTIVE_DRIVER_TASKS_CAP,
    );

    assert!(result.is_err());
    assert!(st.driver_tasks.contains_key("task-0"));
    assert!(!st.driver_tasks.contains_key("new-task"));
}

#[test]
fn deleting_thread_refuses_late_owner_registration_but_keeps_prior_owner() {
    let mut st = Owners::default();
    let existing = owner("existing");
    let existing_key = "existing".to_owned();
    remember_active_driver_task(
        &mut st,
        existing_key.clone(),
        existing.clone(),
        ACTIVE_DRIVER_TASKS_CAP,
    )
    .expect("register live child before deletion starts");
    begin_thread_deletion_in(
        &mut st,
        driver_thread_scope(
            existing.profile.clone(),
            existing.workspace_dir.clone(),
            existing.parent_thread_id.clone().expect("thread owner"),
        ),
    );

    let late = owner("late");
    let result = remember_active_driver_task(&mut st, "late".into(), late, ACTIVE_DRIVER_TASKS_CAP);

    assert!(
        result.is_err(),
        "a child cannot enter after deletion fences admission"
    );
    assert!(
        st.driver_tasks.contains_key(&existing_key),
        "deletion still sees owners admitted before its fence"
    );
    assert!(!st.driver_tasks.contains_key("late"));
}

#[test]
fn failed_overlapping_delete_release_keeps_thread_owner_fenced() {
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let workspace = tempfile::tempdir().expect("workspace");
    let thread = "overlapping-delete-release";
    begin_thread_deletion(&ctx, workspace.path(), thread).expect("first delete begins");
    begin_thread_deletion(&ctx, workspace.path(), thread).expect("overlapping delete begins");

    release_thread_deletion(&ctx, workspace.path(), thread);
    let late_owner = note_driver_task(
        &ctx,
        "overlapping-delete-late-task",
        workspace.path().to_path_buf(),
        tinytools::WorkspaceDescriptor::new(workspace.path()),
        "overlapping-delete-parent".into(),
        Some(thread.into()),
    );
    release_thread_deletion(&ctx, workspace.path(), thread);
    if late_owner.is_ok() {
        finish_driver_task(&ctx, "overlapping-delete-late-task");
    }

    assert!(
        late_owner.is_err(),
        "one failed attempt must not clear another's fence"
    );
}

#[test]
fn retry_release_cannot_remove_committed_thread_deletion_fence() {
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let workspace = tempfile::tempdir().expect("workspace");
    let thread = "committed-delete-retry";
    begin_thread_deletion(&ctx, workspace.path(), thread).expect("delete begins");
    commit_thread_deletion(&ctx, workspace.path(), thread).expect("durable close commits");
    begin_thread_deletion(&ctx, workspace.path(), thread).expect("delete retry begins");

    release_thread_deletion(&ctx, workspace.path(), thread);
    let late_owner = note_driver_task(
        &ctx,
        "committed-delete-late-task",
        workspace.path().to_path_buf(),
        tinytools::WorkspaceDescriptor::new(workspace.path()),
        "committed-delete-parent".into(),
        Some(thread.into()),
    );
    if late_owner.is_ok() {
        finish_driver_task(&ctx, "committed-delete-late-task");
    }

    assert!(
        late_owner.is_err(),
        "a retry release cannot reopen a deleted parent"
    );
}
