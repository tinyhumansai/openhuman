use super::*;

#[tokio::test]
async fn driver_store_imports_legacy_rows_without_losing_host_epoch() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    use crate::tools::toolpacks::ToolGroups;

    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let scratch = tempfile::tempdir().expect("isolated session-store fixture");
    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.clone();
    let context = CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(config, DomainSet::kernel(), ToolGroups::none())
            .profile("session-owner")
            .session_agent("session-owner"),
    );
    let dispatch = DispatchCtx::new(context);
    let legacy = SubagentSessionStore::new_in_context(&dispatch, workspace.clone()).unwrap();
    let row: DurableSubagentSession = serde_json::from_value(serde_json::json!({
        "subagentSessionId": "legacy-session",
        "parentSession": "owner",
        "parentThreadId": "thread",
        "workerThreadId": null,
        "agentId": "researcher",
        "displayName": null,
        "model": null,
        "sandboxMode": "workspace",
        "actionRoot": null,
        "taskKey": "analysis",
        "taskTitle": "Analyze",
        "currentTaskId": "legacy-task",
        "status": "idle",
        "reusable": true,
        "latestHistory": [{"role": "user", "content": "legacy history"}],
        "latestError": null,
        "createdAt": "2026-01-01T00:00:00Z",
        "updatedAt": "2026-01-01T00:00:00Z",
        "lastUsedAt": "2026-01-01T00:00:00Z",
        "parentEpoch": 7
    }))
    .unwrap();
    legacy.save(&[row]).unwrap();

    let driver = open_for_context(
        &dispatch,
        workspace.clone(),
        "owner",
        Some("thread".into()),
        None,
    )
    .expect("open explicitly scoped driver store");
    let imported = driver
        .get("legacy-task")
        .await
        .expect("read imported task alias")
        .expect("legacy row is visible to the driver store");
    assert_eq!(imported.subagent_session_id, "legacy-session");
    assert_eq!(imported.current_task_id.as_deref(), Some("legacy-task"));
    assert_eq!(imported.status, TinyStatus::Idle);
    assert!(serde_json::to_string(&imported.latest_history)
        .unwrap()
        .contains("legacy history"));

    let retained = legacy.load().unwrap();
    assert_eq!(retained[0].parent_epoch, Some(7));
    let context = dispatch.resolved().unwrap();
    let documents = crate::core::runtime::CoreContext::sync_scope(context, || {
        crate::storage::local::open("subagent_sessions", &workspace)
            .unwrap()
            .scoped()
            .unwrap()
            .documents()
            .clone()
    });
    assert_eq!(
        parent_epoch_from_sidecar(Arc::clone(&documents), "legacy-session".into()).unwrap(),
        Some(7)
    );
    import_parent_epoch(Arc::clone(&documents), "legacy-session".into(), None).unwrap();
    assert_eq!(
        parent_epoch_from_sidecar(documents, "legacy-session".into()).unwrap(),
        None
    );
}

#[tokio::test]
async fn driver_session_store_uses_the_explicit_profile_scope() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    use crate::tools::toolpacks::ToolGroups;
    use tinyagents_orchestration::subagent::{
        SubagentSessionSelector as TinySelector, SubagentSessionUpsert as TinyUpsert,
        SubagentTaskKey,
    };

    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let scratch = tempfile::tempdir().expect("isolated session-store fixture");
    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let make_context = |profile: &str| {
        let mut config = crate::config::Config::default();
        config.workspace_dir = workspace.clone();
        CoreContext::for_test(DomainSet::full(), None).derive_with(
            ContextOverlay::new(config, DomainSet::kernel(), ToolGroups::none())
                .profile(profile)
                .session_agent(profile),
        )
    };
    let alice = open_for_context(
        &DispatchCtx::new(make_context("session-owner-alice")),
        workspace.clone(),
        "alice-session",
        Some("shared-thread".into()),
        None,
    )
    .expect("open Alice's explicitly scoped store");
    let bob = open_for_context(
        &DispatchCtx::new(make_context("session-owner-bob")),
        workspace.clone(),
        "bob-session",
        Some("shared-thread".into()),
        None,
    )
    .expect("open Bob's explicitly scoped store");
    let upsert = |parent_session: &str, task_id: &str| TinyUpsert {
        selector: TinySelector {
            parent_session: parent_session.into(),
            parent_thread_id: Some("shared-thread".into()),
            agent_id: "researcher".into(),
            model: None,
            sandbox_mode: "workspace".into(),
            action_root: None,
            task_key: "analysis".into(),
        },
        lifecycle_key: SubagentTaskKey {
            root_run_id: format!("{parent_session}-root"),
            parent_run_id: format!("{parent_session}-parent"),
            thread_id: Some("shared-thread".into()),
            task_id: task_id.into(),
        },
        display_name: None,
        task_title: "Analyze".into(),
        worker_thread_id: None,
        task_id: task_id.into(),
        force_fresh: false,
    };
    alice
        .upsert_running(upsert("alice-session", "alice-task"))
        .await
        .expect("write Alice's row");
    bob.upsert_running(upsert("bob-session", "bob-task"))
        .await
        .expect("write Bob's row");

    let alice_rows = alice
        .list_for_parent("alice-session", Some("shared-thread"))
        .await
        .expect("read Alice's row");
    let bob_rows = bob
        .list_for_parent("bob-session", Some("shared-thread"))
        .await
        .expect("read Bob's row");
    assert_eq!(alice_rows.len(), 1);
    assert_eq!(alice_rows[0].current_task_id.as_deref(), Some("alice-task"));
    assert_eq!(bob_rows.len(), 1);
    assert_eq!(bob_rows[0].current_task_id.as_deref(), Some("bob-task"));
}

#[tokio::test]
async fn old_rows_without_checkpoint_identity_remain_readable_without_fabricated_keys() {
    let workspace = tempfile::tempdir().unwrap();
    let store = SubagentSessionStore::new(workspace.path().to_path_buf());
    let row: DurableSubagentSession = serde_json::from_value(serde_json::json!({
        "subagentSessionId": "legacy-session",
        "parentSession": "owner",
        "parentThreadId": "thread",
        "workerThreadId": null,
        "agentId": "researcher",
        "displayName": null,
        "model": null,
        "sandboxMode": "workspace",
        "actionRoot": null,
        "taskKey": "analysis",
        "taskTitle": "Analyze",
        "currentTaskId": "legacy-task",
        "status": "idle",
        "reusable": true,
        "latestHistory": null,
        "latestError": null,
        "createdAt": "2026-01-01T00:00:00Z",
        "updatedAt": "2026-01-01T00:00:00Z",
        "lastUsedAt": "2026-01-01T00:00:00Z"
    }))
    .unwrap();
    assert!(row.lifecycle_key.is_none());
    store.save(&[row]).unwrap();

    let port = OpenHumanTinySubagentSessionStore::new(
        workspace.path().to_path_buf(),
        "owner",
        Some("thread".into()),
    );
    let sessions = port.list_for_parent("owner", Some("thread")).await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].subagent_session_id, "legacy-session");
    assert_eq!(sessions[0].current_task_id.as_deref(), Some("legacy-task"));
    assert!(sessions[0].lifecycle_key.is_none());
    assert!(port
        .list_for_parent("foreign-owner", Some("thread"))
        .await
        .unwrap()
        .is_empty());
    assert!(port
        .list_for_parent("owner", Some("other-thread"))
        .await
        .unwrap()
        .is_empty());
    assert_eq!(port.list_for_parent("owner", None).await.unwrap().len(), 1);
    assert!(port.get("legacy-session").await.unwrap().is_some());
    let other_thread = OpenHumanTinySubagentSessionStore::new(
        workspace.path().to_path_buf(),
        "owner",
        Some("other-thread".into()),
    );
    assert!(other_thread.get("legacy-session").await.unwrap().is_none());
}

#[tokio::test]
async fn unthreaded_port_lists_only_unthreaded_rows() {
    let workspace = tempfile::tempdir().unwrap();
    let store = SubagentSessionStore::new(workspace.path().to_path_buf());
    let unthreaded: DurableSubagentSession = serde_json::from_value(serde_json::json!({
        "subagentSessionId": "unthreaded-session",
        "parentSession": "owner",
        "parentThreadId": null,
        "workerThreadId": null,
        "agentId": "researcher",
        "displayName": null,
        "model": null,
        "sandboxMode": "workspace",
        "actionRoot": null,
        "taskKey": "analysis",
        "taskTitle": "No thread",
        "currentTaskId": "task-unthreaded",
        "status": "idle",
        "reusable": true,
        "latestHistory": null,
        "latestError": null,
        "createdAt": "2026-01-01T00:00:00Z",
        "updatedAt": "2026-01-01T00:00:00Z",
        "lastUsedAt": "2026-01-01T00:00:00Z"
    }))
    .unwrap();
    let mut threaded = unthreaded.clone();
    threaded.subagent_session_id = "threaded-session".into();
    threaded.parent_thread_id = Some("thread-a".into());
    store.save(&[unthreaded, threaded]).unwrap();

    let port =
        OpenHumanTinySubagentSessionStore::new(workspace.path().to_path_buf(), "owner", None);
    let sessions = port.list_for_parent("owner", None).await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].subagent_session_id, "unthreaded-session");
    assert!(port
        .list_for_parent("owner", Some("thread-a"))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn raw_task_id_lookup_filters_owner_before_matching_and_rejects_ambiguity() {
    fn row(session: &str, owner: &str, thread: &str) -> DurableSubagentSession {
        serde_json::from_value(serde_json::json!({
            "subagentSessionId": session,
            "parentSession": owner,
            "parentThreadId": thread,
            "workerThreadId": null,
            "agentId": "researcher",
            "displayName": null,
            "model": null,
            "sandboxMode": "workspace",
            "actionRoot": null,
            "taskKey": "analysis",
            "taskTitle": "Analyze",
            "currentTaskId": "shared-task-id",
            "status": "idle",
            "reusable": true,
            "latestHistory": null,
            "latestError": null,
            "createdAt": "2026-01-01T00:00:00Z",
            "updatedAt": "2026-01-01T00:00:00Z",
            "lastUsedAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap()
    }

    let workspace = tempfile::tempdir().unwrap();
    let store = SubagentSessionStore::new(workspace.path().to_path_buf());
    store
        .save(&[
            row("foreign-first", "foreign-owner", "same-thread"),
            row("owned-second", "owner", "same-thread"),
        ])
        .unwrap();

    let owned = OpenHumanTinySubagentSessionStore::new(
        workspace.path().to_path_buf(),
        "owner",
        Some("same-thread".into()),
    );
    assert_eq!(
        owned
            .get("shared-task-id")
            .await
            .unwrap()
            .unwrap()
            .subagent_session_id,
        "owned-second",
        "a foreign row with the same raw task ID cannot shadow an owned child"
    );

    store
        .save(&[
            row("owned-second", "owner", "same-thread"),
            row("owned-duplicate", "owner", "same-thread"),
        ])
        .unwrap();
    let error = owned.get("shared-task-id").await.unwrap_err();
    assert!(error.to_string().contains("ambiguous"));
}

#[tokio::test]
async fn upsert_rejects_a_selector_or_lifecycle_key_outside_bound_scope() {
    let workspace = tempfile::tempdir().unwrap();
    let store = SubagentSessionStore::new(workspace.path().to_path_buf());
    let port = OpenHumanTinySubagentSessionStore::new(
        workspace.path().to_path_buf(),
        "owner",
        Some("thread-a".into()),
    );
    let key = |thread_id: Option<&str>, task_id: &str| {
        tinyagents_orchestration::subagent::SubagentTaskKey {
            root_run_id: "root".into(),
            parent_run_id: "parent".into(),
            thread_id: thread_id.map(str::to_owned),
            task_id: task_id.into(),
        }
    };
    let upsert =
        |parent_session: &str,
         parent_thread_id: Option<&str>,
         lifecycle_key: tinyagents_orchestration::subagent::SubagentTaskKey| {
            TinyUpsert {
                selector: TinySelector {
                    parent_session: parent_session.into(),
                    parent_thread_id: parent_thread_id.map(str::to_owned),
                    agent_id: "researcher".into(),
                    model: None,
                    sandbox_mode: "workspace".into(),
                    action_root: None,
                    task_key: "analysis".into(),
                },
                lifecycle_key,
                display_name: None,
                task_title: "Task".into(),
                worker_thread_id: None,
                task_id: "task-1".into(),
                force_fresh: true,
            }
        };

    assert!(port
        .upsert_running(upsert(
            "foreign",
            Some("thread-a"),
            key(Some("thread-a"), "task-1")
        ))
        .await
        .is_err());
    assert!(port
        .upsert_running(upsert(
            "owner",
            Some("thread-b"),
            key(Some("thread-b"), "task-1")
        ))
        .await
        .is_err());
    assert!(port
        .upsert_running(upsert(
            "owner",
            Some("thread-a"),
            key(Some("thread-b"), "task-1")
        ))
        .await
        .is_err());
    assert!(port
        .upsert_running(upsert(
            "owner",
            Some("thread-a"),
            key(Some("thread-a"), "other-task")
        ))
        .await
        .is_err());
    assert!(store.load().unwrap().is_empty());
}
