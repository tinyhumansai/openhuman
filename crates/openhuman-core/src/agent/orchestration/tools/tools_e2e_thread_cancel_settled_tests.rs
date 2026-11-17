use super::*;

/// A driver child listed as live by thread deletion/purge, which then finishes
/// on its own before `cancel_owned` runs, is settled, not a cancellation
/// failure. `cancel_owned` answers `Terminal` for it; thread deletion and purge
/// both settle children through `cancel_for_thread`, which used to report that
/// as "could not safely settle" and fail the whole operation.
#[tokio::test]
async fn child_settling_between_listing_and_cancel_counts_as_settled() {
    use crate::agent::orchestration::running_subagents::cancel_for_thread_with_hook;
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
            Ok(ModelResponse::assistant("child finished before cancel"))
        }
    }

    let scratch = tempfile::tempdir().expect("isolated cancel-window fixture");
    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let thread_id = "cancel-window-parent-thread";
    let parent_session = "cancel-window-parent-session";
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
        tinyagents_harness::context::RunConfig::new("cancel-window-parent").with_thread(thread_id),
        host_context,
    )
    .with_workspace(workspace_descriptor.clone());
    let dispatch = live.data.dispatch_ctx();
    blocking::ensure_thread(
        dispatch.clone(),
        workspace.clone(),
        CreateConversationThread {
            id: thread_id.into(),
            title: "Cancel window parent".into(),
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
    crate::agent::orchestration::background_completions::install_store_for_test(
        &workspace,
        completion_store,
    );
    let detached_data = live.data.detached_child();
    let detached = live
        .child(
            tinyagents_harness::context::RunConfig::new("cancel-window-child"),
            detached_data.clone(),
        )
        .expect("detached child context")
        .with_cancellation(detached_data.cancellation.clone());
    let spawned = crate::agent::orchestration::tools::SpawnSubagentTool::new()
        .execute_with_live_parent_context(
            json!({
                "agent_id": "task_manager_agent",
                "prompt": "finish while the parent thread is being deleted",
                "model": "test-model"
            }),
            None,
            live.data.child(),
            detached,
        )
        .await
        .expect("spawn tool execution");
    assert!(!spawned.is_error, "{}", spawned.output());
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

    // The hook runs after cancel_for_thread has listed the job as live and
    // before it calls cancel_owned. Let the child finish and wait until the
    // registry reports it terminal, so cancel_owned meets a settled job.
    let settle_before_cancel = {
        let release = release.clone();
        let jobs = jobs.clone();
        let job_id = job.id.clone();
        move || {
            let release = release.clone();
            let jobs = jobs.clone();
            let job_id = job_id.clone();
            async move {
                release.notify_one();
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    while !jobs
                        .get(job_id.as_str())
                        .is_some_and(|job| job.status.is_terminal())
                    {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                })
                .await
                .expect("released child settles on its own");
            }
        }
    };

    let cancelled = cancel_for_thread_with_hook(&dispatch, thread_id, settle_before_cancel)
        .await
        .expect("a child that settled before cancel is not a cancellation failure");

    assert_eq!(
        cancelled, 0,
        "the child finished on its own, nothing was cancelled"
    );
    assert_eq!(
        jobs.get(job.id.as_str()).map(|job| job.status),
        Some(SubAgentJobStatus::Completed),
        "the child's own completion stands"
    );
    assert!(
        crate::agent::orchestration::completion_owners::driver_tasks_for_thread(
            &dispatch, &workspace, thread_id,
        )
        .is_empty(),
        "the settled child's owner slot is released"
    );
}
