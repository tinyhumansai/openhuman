use super::*;

#[tokio::test]
async fn failed_steer_of_running_reused_session_does_not_create_second_child() {
    use crate::agent::orchestration::subagent_sessions::{
        DurableSubagentStatus, SubagentSessionSelector, SubagentSessionStore, SubagentSessionUpsert,
    };
    use crate::agent::tinyagents::host::OpenHumanRunContext;
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::tools::toolpacks::ToolGroups;
    use tinyagents_harness::context::RunContext;

    let scratch = tempfile::tempdir().expect("isolated failed-steer fixture");
    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let provider = Arc::new(ScriptedModel::new(vec![]));
    let mut parent = parent_context(workspace.as_path(), provider.clone(), vec![]);
    parent.workspace_dir = workspace.clone();
    parent.session_id = "failed-steer-parent-session".into();
    let definitions = Arc::new(AgentDefinitionRegistry::builtins_only());
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.clone();
    config.components.juice = false;
    let core = CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(config, DomainSet::full(), ToolGroups::none()).definitions(definitions),
    );
    let thread_id = "failed-steer-parent-thread";
    let mut host_context = OpenHumanRunContext::new()
        .with_core(Some(Arc::clone(&core)))
        .with_parent(parent);
    host_context.thread_id = Some(thread_id.into());
    let live = RunContext::<OpenHumanRunContext>::new(
        tinyagents_harness::context::RunConfig::new("failed-steer-parent").with_thread(thread_id),
        host_context,
    );

    // Model a durable Running row whose in-memory job has disappeared. The
    // production reuse branch tries to steer this exact task; the real
    // SubAgentMessageTool then refuses the missing job. Before the fix this
    // refusal fell through and admitted a second worker on the same session.
    let store = SubagentSessionStore::new(workspace.clone());
    let selector = SubagentSessionSelector {
        parent_session: "failed-steer-parent-session".into(),
        parent_thread_id: Some(thread_id.into()),
        agent_id: "task_manager_agent".into(),
        model: Some("test-model".into()),
        sandbox_mode: format!(
            "{:?}",
            AgentDefinitionRegistry::builtins_only()
                .get("task_manager_agent")
                .expect("task manager definition")
                .sandbox_mode
        ),
        action_root: None,
        task_key: "failed-steer-canary".into(),
    };
    let existing = crate::core::runtime::CoreContext::sync_scope(core.clone(), || {
        crate::agent::orchestration::subagent_sessions::upsert_running(
            &store,
            SubagentSessionUpsert {
                selector,
                display_name: None,
                task_title: "existing child".into(),
                worker_thread_id: None,
                task_id: "existing-running-task".into(),
                lifecycle_key: None,
                parent_epoch: None,
            },
            None,
        )
    })
    .expect("seed running reusable session");
    assert_eq!(existing.status, DurableSubagentStatus::Running);

    let detached_data = live.data.detached_child();
    let detached = live
        .child(
            tinyagents_harness::context::RunConfig::new("failed-steer-child"),
            detached_data.clone(),
        )
        .expect("detached child context")
        .with_cancellation(detached_data.cancellation.clone());
    let jobs = crate::core::runtime::slot_for::<
        tinyagents_orchestration::subagent::SubAgentJobRegistry,
    >(&live.data.dispatch_ctx());
    let jobs_before = jobs.list().len();
    let result = crate::agent::orchestration::tools::SpawnSubagentTool::new()
        .execute_with_live_parent_context(
            json!({
                "agent_id": "task_manager_agent",
                "prompt": "continue the same task",
                "model": "test-model",
                "task_key": "failed-steer-canary"
            }),
            None,
            live.data.child(),
            detached,
        )
        .await
        .expect("spawn tool executes");

    assert!(
        result.is_error,
        "failed steer must be reported, got: {}",
        result.output()
    );
    assert!(
        result.output().contains("refusing to start a duplicate")
            && result.output().contains("existing-running-task"),
        "the refusal identifies the failed reusable task: {}",
        result.output()
    );
    assert!(
        provider.seen.lock().is_empty(),
        "no second child model run may start"
    );
    assert_eq!(
        jobs.list().len(),
        jobs_before,
        "no second child job is admitted"
    );
    let after = store
        .load()
        .expect("session store remains readable")
        .into_iter()
        .find(|row| row.subagent_session_id == existing.subagent_session_id)
        .expect("the original session remains");
    assert_eq!(after.status, DurableSubagentStatus::Running);
    assert_eq!(
        after.current_task_id.as_deref(),
        Some("existing-running-task")
    );
}
