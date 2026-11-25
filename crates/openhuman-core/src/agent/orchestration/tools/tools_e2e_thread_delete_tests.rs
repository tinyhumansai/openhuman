use super::*;

#[tokio::test]
async fn deleting_parent_thread_cancels_driver_child_before_discarding_completions() {
    use crate::agent::orchestration::background_completions::{self, BackgroundAgentOutcome};
    use crate::agent::tinyagents::host::OpenHumanRunContext;
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::threads::store::{blocking, CreateConversationThread};
    use crate::tools::toolpacks::ToolGroups;
    use tinyagents_harness::context::RunContext;
    use tinyagents_orchestration::subagent::SubAgentJobRegistry;
    use tinyagents_orchestration::subagent::SubAgentJobStatus;

    struct GatedModel {
        started: tokio::sync::mpsc::Sender<()>,
        release: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl ChatModel<()> for GatedModel {
        fn profile(&self) -> Option<&ModelProfile> {
            static PROFILE: std::sync::OnceLock<ModelProfile> = std::sync::OnceLock::new();
            Some(PROFILE.get_or_init(|| {
                let mut profile = ModelProfile::default();
                profile.tool_calling = true;
                profile.parallel_tool_calls = true;
                profile
            }))
        }

        async fn invoke(
            &self,
            _state: &(),
            _request: ModelRequest,
        ) -> tinyinference_llm::Result<ModelResponse> {
            let _ = self.started.send(()).await;
            self.release.notified().await;
            Ok(ModelResponse::assistant("child released after delete"))
        }
    }

    let scratch = tempfile::tempdir().expect("isolated thread-delete fixture");
    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let thread_id = "delete-live-driver-parent-thread";
    let parent_session = "delete-live-driver-parent-session";
    let (started_tx, mut started_rx) = tokio::sync::mpsc::channel(1);
    let release = Arc::new(tokio::sync::Notify::new());
    let provider = Arc::new(GatedModel {
        started: started_tx,
        release: release.clone(),
    });
    let mut parent = parent_context(&workspace, provider, vec![]);
    let workspace_descriptor = tinytools::WorkspaceDescriptor::new(workspace.clone());
    parent.workspace_descriptor = Some(workspace_descriptor.clone());
    parent.workspace_dir = workspace.clone();
    parent.session_id = parent_session.into();

    let definitions = Arc::new(AgentDefinitionRegistry::builtins_only());
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.clone();
    config.components.juice = false;
    let core = CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(config, DomainSet::full(), ToolGroups::none()).definitions(definitions),
    );
    let mut host_context = OpenHumanRunContext::new()
        .with_core(Some(Arc::clone(&core)))
        .with_parent(parent);
    host_context.thread_id = Some(thread_id.into());
    let live = RunContext::<OpenHumanRunContext>::new(
        tinyagents_harness::context::RunConfig::new("delete-live-driver-parent")
            .with_thread(thread_id),
        host_context,
    )
    .with_workspace(workspace_descriptor.clone());
    let dispatch = live.data.dispatch_ctx();
    blocking::ensure_thread(
        dispatch.clone(),
        workspace.clone(),
        CreateConversationThread {
            id: thread_id.into(),
            title: "Delete live driver parent".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .await
    .expect("persist parent thread before spawning child");

    let completion_store: Arc<dyn tinyagents_tasks::CompletionStore> =
        Arc::new(tinyagents_tasks::InMemoryCompletionStore::new());
    background_completions::install_store_for_test(&workspace, completion_store);
    let detached_data = live.data.detached_child();
    let detached = live
        .child(
            tinyagents_harness::context::RunConfig::new("delete-live-driver-child"),
            detached_data.clone(),
        )
        .expect("detached child context")
        .with_cancellation(detached_data.cancellation.clone());
    let spawned = crate::agent::orchestration::tools::SpawnSubagentTool::new()
        .execute_with_live_parent_context(
            json!({
                "agent_id": "task_manager_agent",
                "prompt": "wait for the parent thread to be deleted",
                "model": "test-model"
            }),
            None,
            live.data.child(),
            detached,
        )
        .await
        .expect("spawn tool execution");
    assert!(!spawned.is_error, "{}", spawned.output());
    let accepted_ref = spawned
        .output()
        .split("[async_subagent_ref]\n")
        .nth(1)
        .and_then(|value| value.split("\n[/async_subagent_ref]").next())
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .expect("accepted child envelope contains its task ID");
    let task_id = accepted_ref["task_id"]
        .as_str()
        .expect("accepted task ID")
        .to_owned();
    tokio::time::timeout(std::time::Duration::from_secs(5), started_rx.recv())
        .await
        .expect("child reaches its gated provider call")
        .expect("provider start notification");

    let jobs = crate::core::runtime::slot_for::<SubAgentJobRegistry>(&dispatch);
    let job = jobs
        .list()
        .into_iter()
        .find(|job| job.status == SubAgentJobStatus::Running && job.agent == "task_manager_agent")
        .expect("spawned child is live in the TinyAgents job registry");
    let owner =
        crate::agent::orchestration::completion_owners::driver_task_owner(&dispatch, &task_id)
            .expect("spawn registers the exact task owner");
    assert_eq!(owner.workspace_dir, workspace);
    assert_eq!(owner.parent_thread_id.as_deref(), Some(thread_id));
    assert_eq!(
        crate::agent::orchestration::completion_owners::driver_tasks_for_thread(
            &dispatch, &workspace, thread_id,
        )
        .len(),
        1,
        "owner index selects the job in its exact profile/workspace/thread scope"
    );
    let scoped_jobs = jobs
        .list_for_parent_scope(
            &crate::agent::tinyagents::tool_registration::subagent_owner_key(
                &owner.workspace_dir,
                &owner.parent_session,
                owner.parent_thread_id.as_deref(),
            ),
            &tinyagents_orchestration::subagent::ParentJobScope::new(
                owner.workspace.clone(),
                thread_id,
            ),
        )
        .expect("list exact parent scope");
    assert!(
        scoped_jobs.iter().any(|scoped| scoped.id == job.id),
        "the driver's own scope query sees the job selected by the owner index"
    );
    background_completions::record_outcome(
        &dispatch,
        &workspace,
        parent_session,
        task_id.clone(),
        "task_manager_agent",
        "queued child result",
        Some(thread_id.into()),
        BackgroundAgentOutcome::Completed,
    )
    .await;
    assert_eq!(
        background_completions::pending_for(&workspace, thread_id).len(),
        1,
        "the deletion starts with a queued child completion"
    );
    let release_on_cancel = {
        let release = release.clone();
        tokio::spawn(async move {
            // Give thread deletion time to request cancellation and enter its
            // settlement wait. The running snapshot stays `Running` until the
            // provider unwinds, so the cancellation request itself is not
            // observable through the public job snapshot.
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            release.notify_one();
        })
    };

    tokio::time::timeout(
        std::time::Duration::from_secs(6),
        crate::threads::ops::thread_delete(
            &dispatch,
            crate::threads::DeleteConversationThreadRequest {
                thread_id: thread_id.into(),
                deleted_at: "2026-01-01T00:01:00Z".into(),
            },
        ),
    )
    .await
    .expect("thread deletion waits for cancellation-aware child settlement")
    .expect("delete parent thread");

    assert_eq!(
        jobs.get(job.id.as_str()).map(|job| job.status),
        Some(SubAgentJobStatus::Cancelled),
        "deleting a parent thread cancels its live TinyAgents child"
    );
    assert!(
        crate::agent::orchestration::completion_owners::driver_tasks_for_thread(
            &dispatch, &workspace, thread_id,
        )
        .is_empty(),
        "settled thread deletion releases active child owner slots"
    );
    assert!(
        crate::agent::orchestration::completion_owners::note_driver_task(
            &dispatch,
            "late-child-after-parent-delete",
            workspace.clone(),
            workspace_descriptor.clone(),
            parent_session.into(),
            Some(thread_id.into()),
        )
        .is_err(),
        "the permanent deletion fence refuses children that race after the sweep"
    );
    assert!(
        background_completions::pending_for(&workspace, thread_id).is_empty(),
        "queued child completions are discarded after cancellation"
    );
    background_completions::record_outcome(
        &dispatch,
        &workspace,
        parent_session,
        task_id.clone(),
        "task_manager_agent",
        "late terminal child completion",
        Some(thread_id.into()),
        BackgroundAgentOutcome::Completed,
    )
    .await;
    crate::threads::ops::thread_delete(
        &dispatch,
        crate::threads::DeleteConversationThreadRequest {
            thread_id: thread_id.into(),
            deleted_at: "2026-01-01T00:02:00Z".into(),
        },
    )
    .await
    .expect("a retained terminal owner does not block deletion cleanup");
    assert!(
        background_completions::pending_for(&workspace, thread_id).is_empty(),
        "terminal driver owners do not prevent queued completions from being discarded"
    );
    release_on_cancel
        .await
        .expect("cancellation-aware child release task completes");
}

#[tokio::test]
async fn thread_creation_cannot_enter_the_snapshot_to_purge_window() {
    use crate::config::test_env::EnvVarGuard;
    use crate::threads::store::{blocking, ConversationStore, CreateConversationThread};

    let scratch = tempfile::tempdir().expect("isolated purge race fixture");
    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let _env_lock = EnvVarGuard::locked_async().await;
    let _workspace_env = EnvVarGuard::workspace_unlocked(&workspace);
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let make_thread = |id: &str| CreateConversationThread {
        id: id.into(),
        title: format!("Purge race {id}"),
        created_at: "2026-01-01T00:00:00Z".into(),
        parent_thread_id: None,
        labels: None,
        personality_id: None,
        working_dir: None,
    };
    blocking::ensure_thread(
        ctx.clone(),
        workspace.clone(),
        make_thread("purge-race-existing"),
    )
    .await
    .expect("seed a thread for the purge snapshot");

    let (snapshot_tx, snapshot_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let purge_ctx = ctx.clone();
    let purge_workspace = workspace.clone();
    let purge = tokio::spawn(async move {
        crate::threads::ops::threads_purge_inner_with_snapshot_hook(
            purge_ctx,
            purge_workspace,
            || async move {
                let _ = snapshot_tx.send(());
                let _ = release_rx.await;
            },
        )
        .await
    });
    snapshot_rx
        .await
        .expect("purge has captured its thread snapshot");

    // Exercise both the synchronous facade directly (which many non-RPC
    // callers use) and the request path used by thread_upsert. They must not
    // create a row after the purge snapshot but before the store-wide delete.
    let alias_workspace = workspace.join(".").join("..").join("workspace");
    let direct_result = ConversationStore::new(ctx.clone(), alias_workspace)
        .ensure_thread(make_thread("purge-race-direct-facade"));
    let request_ctx = ctx.clone();
    let request_result = tokio::spawn(async move {
        crate::threads::ops::thread_upsert(
            &request_ctx,
            crate::threads::UpsertConversationThreadRequest {
                id: "purge-race-rpc-upsert".into(),
                title: "Purge race RPC upsert".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                parent_thread_id: None,
                labels: None,
                personality_id: None,
            },
        )
        .await
    });
    let rpc_result = tokio::time::timeout(std::time::Duration::from_secs(3), request_result)
        .await
        .expect("thread_upsert returns while purge is active")
        .expect("thread_upsert task joins");

    let _ = release_tx.send(());
    purge
        .await
        .expect("purge task joins")
        .expect("purge succeeds");

    assert!(
        direct_result.is_err(),
        "direct ConversationStore ensures must be refused during workspace purge"
    );
    assert!(
        rpc_result.is_err(),
        "RPC upserts must be refused during workspace purge"
    );
    assert!(
        blocking::list_threads(ctx, workspace)
            .await
            .expect("list threads after purge")
            .is_empty(),
        "no creator can add an un-fenced parent during the snapshot/purge window"
    );
}

#[tokio::test]
async fn thread_delete_closes_parent_when_driver_task_scope_cannot_be_resolved() {
    use crate::agent::orchestration::background_completions::{self, BackgroundAgentOutcome};
    use crate::agent::tinyagents::host::OpenHumanRunContext;
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::threads::store::{blocking, CreateConversationThread};
    use crate::tools::toolpacks::ToolGroups;
    use tinyagents_harness::context::RunContext;

    let scratch = tempfile::tempdir().expect("isolated unresolved-driver fixture");
    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let thread_id = "delete-unresolved-driver-parent-thread";
    let parent_session = "delete-unresolved-driver-parent-session";
    let mut parent = parent_context(&workspace, Arc::new(ScriptedModel::new(vec![])), vec![]);
    let workspace_descriptor = tinytools::WorkspaceDescriptor::new(workspace.clone());
    parent.workspace_descriptor = Some(workspace_descriptor.clone());
    parent.session_id = parent_session.into();

    let definitions = Arc::new(AgentDefinitionRegistry::builtins_only());
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.clone();
    config.components.juice = false;
    let core = CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(config, DomainSet::full(), ToolGroups::none()).definitions(definitions),
    );
    let mut host_context = OpenHumanRunContext::new()
        .with_core(Some(Arc::clone(&core)))
        .with_parent(parent);
    host_context.thread_id = Some(thread_id.into());
    let live = RunContext::<OpenHumanRunContext>::new(
        tinyagents_harness::context::RunConfig::new("delete-unresolved-driver-parent")
            .with_thread(thread_id),
        host_context,
    )
    .with_workspace(workspace_descriptor.clone());
    let dispatch = live.data.dispatch_ctx();
    blocking::ensure_thread(
        dispatch.clone(),
        workspace.clone(),
        CreateConversationThread {
            id: thread_id.into(),
            title: "Delete unresolved driver parent".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .await
    .expect("persist parent thread before deletion");

    let completion_store: Arc<dyn tinyagents_tasks::CompletionStore> =
        Arc::new(tinyagents_tasks::InMemoryCompletionStore::new());
    background_completions::install_store_for_test(&workspace, completion_store);
    background_completions::record_outcome(
        &dispatch,
        &workspace,
        parent_session,
        "stale-driver-task",
        "task_manager_agent",
        "queued child result",
        Some(thread_id.into()),
        BackgroundAgentOutcome::Completed,
    )
    .await;
    crate::agent::orchestration::completion_owners::note_driver_task(
        &dispatch,
        "stale-driver-task",
        workspace.clone(),
        workspace_descriptor,
        parent_session.into(),
        Some(thread_id.into()),
    )
    .expect("record active owner for fail-closed regression");

    let result = crate::threads::ops::thread_delete(
        &dispatch,
        crate::threads::DeleteConversationThreadRequest {
            thread_id: thread_id.into(),
            deleted_at: "2026-01-01T00:01:00Z".into(),
        },
    )
    .await;

    assert!(result
        .expect_err("an indexed task without its driver job must fail closed")
        .contains("no live driver job or terminal durable session"));
    assert!(
        background_completions::pending_for(&workspace, thread_id).is_empty(),
        "failed child settlement still durably closes a deleted parent and withdraws its queue"
    );

    let terminal_legacy_row = serde_json::from_value(serde_json::json!({
        "subagentSessionId": "pruned-terminal-session",
        "parentSession": parent_session,
        "parentThreadId": thread_id,
        "workerThreadId": null,
        "agentId": "task_manager_agent",
        "displayName": null,
        "model": null,
        "sandboxMode": "workspace",
        "actionRoot": null,
        "taskKey": "pruned-terminal-task",
        "taskTitle": "Completed before registry pruning",
        "currentTaskId": "stale-driver-task",
        "status": "idle",
        "reusable": true,
        "latestHistory": [],
        "latestError": null,
        "createdAt": "2026-01-01T00:00:00Z",
        "updatedAt": "2026-01-01T00:00:00Z",
        "lastUsedAt": "2026-01-01T00:00:00Z",
        "parentEpoch": null,
        "lifecycleKey": null
    }))
    .expect("deserialize terminal legacy session");
    crate::agent::orchestration::subagent_sessions::SubagentSessionStore::new_in_context(
        &dispatch,
        workspace.clone(),
    )
    .expect("open explicit legacy session scope")
    .save(&[terminal_legacy_row])
    .expect("save terminal legacy session for import");

    crate::threads::ops::thread_delete(
        &dispatch,
        crate::threads::DeleteConversationThreadRequest {
            thread_id: thread_id.into(),
            deleted_at: "2026-01-01T00:02:00Z".into(),
        },
    )
    .await
    .expect("terminal durable session proves the missing registry snapshot is settled");
    assert!(
        background_completions::pending_for(&workspace, thread_id).is_empty(),
        "terminal durable session permits queued completion cleanup after registry pruning"
    );
    crate::agent::orchestration::completion_owners::finish_driver_task(
        &dispatch,
        "stale-driver-task",
    );
}

#[tokio::test]
async fn purging_threads_cancels_driver_children_and_discards_completions() {
    use crate::agent::orchestration::background_completions::{self, BackgroundAgentOutcome};
    use crate::agent::tinyagents::host::OpenHumanRunContext;
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::threads::store::{blocking, CreateConversationThread};
    use crate::tools::toolpacks::ToolGroups;
    use tinyagents_harness::context::RunContext;
    use tinyagents_orchestration::subagent::SubAgentJobRegistry;
    use tinyagents_orchestration::subagent::SubAgentJobStatus;

    struct GatedModel {
        started: tokio::sync::mpsc::Sender<()>,
        release: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl ChatModel<()> for GatedModel {
        fn profile(&self) -> Option<&ModelProfile> {
            static PROFILE: std::sync::OnceLock<ModelProfile> = std::sync::OnceLock::new();
            Some(PROFILE.get_or_init(|| {
                let mut profile = ModelProfile::default();
                profile.tool_calling = true;
                profile.parallel_tool_calls = true;
                profile
            }))
        }

        async fn invoke(
            &self,
            _state: &(),
            _request: ModelRequest,
        ) -> tinyinference_llm::Result<ModelResponse> {
            let _ = self.started.send(()).await;
            self.release.notified().await;
            Ok(ModelResponse::assistant("child released after purge"))
        }
    }

    let scratch = tempfile::tempdir().expect("isolated thread-purge fixture");
    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let _env =
        crate::config::test_env::EnvVarGuard::locked_set_async("OPENHUMAN_WORKSPACE", &workspace)
            .await;
    let thread_id = "purge-live-driver-parent-thread";
    let parent_session = "purge-live-driver-parent-session";
    let (started_tx, mut started_rx) = tokio::sync::mpsc::channel(1);
    let release = Arc::new(tokio::sync::Notify::new());
    let provider = Arc::new(GatedModel {
        started: started_tx,
        release: release.clone(),
    });
    let mut parent = parent_context(&workspace, provider, vec![]);
    let workspace_descriptor = tinytools::WorkspaceDescriptor::new(workspace.clone());
    parent.workspace_descriptor = Some(workspace_descriptor.clone());
    parent.workspace_dir = workspace.clone();
    parent.session_id = parent_session.into();

    let definitions = Arc::new(AgentDefinitionRegistry::builtins_only());
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.clone();
    config.components.juice = false;
    let core = CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(config, DomainSet::full(), ToolGroups::none()).definitions(definitions),
    );
    let mut host_context = OpenHumanRunContext::new()
        .with_core(Some(Arc::clone(&core)))
        .with_parent(parent);
    host_context.thread_id = Some(thread_id.into());
    let live = RunContext::<OpenHumanRunContext>::new(
        tinyagents_harness::context::RunConfig::new("purge-live-driver-parent")
            .with_thread(thread_id),
        host_context,
    )
    .with_workspace(workspace_descriptor.clone());
    let dispatch = live.data.dispatch_ctx();
    blocking::ensure_thread(
        dispatch.clone(),
        workspace.clone(),
        CreateConversationThread {
            id: thread_id.into(),
            title: "Purge live driver parent".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .await
    .expect("persist parent thread before spawning child");

    let completion_store: Arc<dyn tinyagents_tasks::CompletionStore> =
        Arc::new(tinyagents_tasks::InMemoryCompletionStore::new());
    background_completions::install_store_for_test(&workspace, completion_store);
    let detached_data = live.data.detached_child();
    let detached = live
        .child(
            tinyagents_harness::context::RunConfig::new("purge-live-driver-child"),
            detached_data.clone(),
        )
        .expect("detached child context")
        .with_cancellation(detached_data.cancellation.clone());
    let spawned = crate::agent::orchestration::tools::SpawnSubagentTool::new()
        .execute_with_live_parent_context(
            json!({
                "agent_id": "task_manager_agent",
                "prompt": "wait for the parent thread to be purged",
                "model": "test-model"
            }),
            None,
            live.data.child(),
            detached,
        )
        .await
        .expect("spawn tool execution");
    assert!(!spawned.is_error, "{}", spawned.output());
    let task_id = spawned
        .output()
        .split("[async_subagent_ref]\n")
        .nth(1)
        .and_then(|value| value.split("\n[/async_subagent_ref]").next())
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .and_then(|value| value["task_id"].as_str().map(str::to_owned))
        .expect("accepted child envelope contains its task ID");
    tokio::time::timeout(std::time::Duration::from_secs(5), started_rx.recv())
        .await
        .expect("child reaches its gated provider call")
        .expect("provider start notification");

    let jobs = crate::core::runtime::slot_for::<SubAgentJobRegistry>(&dispatch);
    let job = jobs
        .list()
        .into_iter()
        .find(|job| job.status == SubAgentJobStatus::Running && job.agent == "task_manager_agent")
        .expect("spawned child is live in the TinyAgents job registry");
    let owner =
        crate::agent::orchestration::completion_owners::driver_task_owner(&dispatch, &task_id)
            .expect("spawn registers the exact task owner");
    assert_eq!(owner.workspace_dir, workspace);
    assert_eq!(owner.parent_thread_id.as_deref(), Some(thread_id));
    background_completions::record_outcome(
        &dispatch,
        &workspace,
        parent_session,
        task_id.clone(),
        "task_manager_agent",
        "queued child result",
        Some(thread_id.into()),
        BackgroundAgentOutcome::Completed,
    )
    .await;
    assert_eq!(
        background_completions::pending_for(&workspace, thread_id).len(),
        1,
        "the purge starts with a queued child completion"
    );
    // Keep the child gated until purge has settled it. The harness races the
    // in-flight model call against the job's cancellation token, so purge's
    // cancel unwinds the child without a release. Releasing on a timer let the
    // child complete on its own before purge cancelled it whenever purge's
    // admission and fencing took longer than the timer.
    tokio::time::timeout(
        std::time::Duration::from_secs(6),
        crate::threads::ops::threads_purge(&dispatch, crate::threads::EmptyRequest {}),
    )
    .await
    .expect("purge waits for cancellation-aware child settlement")
    .expect("purge all threads");

    assert_eq!(
        jobs.get(job.id.as_str()).map(|job| job.status),
        Some(SubAgentJobStatus::Cancelled),
        "purging the parent thread cancels its live TinyAgents child"
    );
    assert!(
        background_completions::pending_for(&workspace, thread_id).is_empty(),
        "purge discards queued child completions"
    );
    background_completions::record_outcome(
        &dispatch,
        &workspace,
        parent_session,
        task_id,
        "task_manager_agent",
        "late child result after purge",
        Some(thread_id.into()),
        BackgroundAgentOutcome::Completed,
    )
    .await;
    assert!(
        background_completions::pending_for(&workspace, thread_id).is_empty(),
        "a child completion racing after purge remains tombstoned"
    );
    assert!(
        crate::agent::orchestration::completion_owners::driver_tasks_for_thread(
            &dispatch, &workspace, thread_id,
        )
        .is_empty(),
        "settled purge releases active child owner slots"
    );
    release.notify_one();
}
