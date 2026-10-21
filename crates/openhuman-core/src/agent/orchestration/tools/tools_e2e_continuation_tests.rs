use super::*;

#[tokio::test]
async fn continue_subagent_resumes_paused_job_through_shared_driver() {
    use crate::agent::orchestration::tools::{ContinueSubagentTool, SpawnSubagentTool};
    use tinyagents_orchestration::subagent::{SubAgentJobRegistry, SubAgentJobStatus};

    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let provider = Arc::new(PauseThenAnswerModel {
        calls: std::sync::atomic::AtomicUsize::new(0),
        seen: Mutex::new(Vec::new()),
    });
    let ask_tool: Box<dyn tinytools::Tool> =
        Box::new(tinyagents_harness_tools::AskClarificationTool::new());
    let ask_spec = Arc::new(ask_tool.spec());
    let mut parent = parent_context(workspace.path(), provider.clone(), vec![]);
    parent.all_tools = Arc::new(vec![ask_tool]);
    parent.all_tool_specs = Arc::new(vec![ask_spec.clone()]);
    parent.visible_tool_specs = Arc::new(vec![ask_spec]);
    parent
        .visible_tool_names
        .insert("ask_user_clarification".into());
    parent
        .subagent_tool_ceiling_names
        .insert("ask_user_clarification".into());
    let live =
        crate::agent::orchestration::tools::live_parent_run_context("continue-paused-e2e", parent);
    let paused = SpawnSubagentTool::new()
        .execute_with_live_parent_context(
            json!({
                "agent_id": "task_manager_agent",
                "prompt": "Investigate the pause-resume fixture",
                "model": "test-model",
                "blocking": true
            }),
            None,
            live.data.child(),
            Some(&live),
        )
        .await
        .expect("initial spawn");
    assert!(!paused.is_error, "{}", paused.output());
    assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let task_id = paused
        .output()
        .lines()
        .find_map(|line| line.strip_prefix("task_id: "))
        .unwrap_or_else(|| panic!("pause envelope carries task id: {}", paused.output()))
        .to_owned();
    assert!(paused.output().contains("which file should I inspect?"));

    let jobs = crate::core::runtime::slot_for::<SubAgentJobRegistry>(&live.data.dispatch_ctx());
    let paused_job = jobs
        .list()
        .into_iter()
        .find(|job| {
            job.extensions
                .get("task_id")
                .and_then(serde_json::Value::as_str)
                == Some(task_id.as_str())
        })
        .expect("initial pause is registered in shared Jobs");
    assert_eq!(paused_job.status, SubAgentJobStatus::AwaitingInput);

    let resumed = ContinueSubagentTool::new()
        .execute_with_live_parent_context(
            json!({
                "task_id": task_id,
                "agent_id": "task_manager_agent",
                "message": "Inspect the README and continue."
            }),
            None,
            live.data.child(),
            Some(&live),
        )
        .await
        .expect("continue");
    assert!(!resumed.is_error, "{}", resumed.output());
    assert!(provider.saw("Inspect the README and continue."));
    assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    let requests = provider.seen.lock();
    assert_eq!(requests.len(), 2);
    let resumed_request = &requests[1];
    assert!(resumed_request.contains("Investigate the pause-resume fixture"));
    assert!(resumed_request.contains("which file should I inspect?"));
    assert_eq!(
        resumed_request
            .matches("Inspect the README and continue.")
            .count(),
        1,
        "the answer is appended to the resumed history exactly once"
    );
    drop(requests);

    let completed_job = jobs
        .get(paused_job.id.as_str())
        .expect("continuation reuses the same registered job");
    assert_eq!(completed_job.agent, "task_manager_agent");
    assert_eq!(completed_job.status, SubAgentJobStatus::Completed);
    assert_eq!(
        completed_job.output.as_deref(),
        Some("paused-child-resumed-answer")
    );
}

#[tokio::test]
async fn cancelling_a_paused_driver_through_rpc_settles_once_with_its_reason() {
    use crate::agent::orchestration::background_completions::{
        forget_workspace_for_test, pending_for,
    };
    use crate::agent::orchestration::subagent_sessions::{
        DurableSubagentStatus, SubagentSessionStore,
    };
    use crate::agent::orchestration::tools::SpawnSubagentTool;
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    use crate::tools::toolpacks::ToolGroups;
    use tinyagents_orchestration::subagent::{SubAgentJobRegistry, SubAgentJobStatus};
    use tinyagents_tasks::CompletionStatus;

    let _lock = crate::config::TEST_ENV_LOCK.lock().await;
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::tempdir().expect("workspace");
    let provider = Arc::new(PauseThenAnswerModel {
        calls: std::sync::atomic::AtomicUsize::new(0),
        seen: Mutex::new(Vec::new()),
    });
    let ask_tool: Box<dyn tinytools::Tool> =
        Box::new(tinyagents_harness_tools::AskClarificationTool::new());
    let ask_spec = Arc::new(ask_tool.spec());
    let mut parent = parent_context(workspace.path(), provider.clone(), vec![]);
    parent.workspace_dir = workspace.path().to_path_buf();
    parent.session_id = "paused-cancel-rpc-parent".into();
    parent.all_tools = Arc::new(vec![ask_tool]);
    parent.all_tool_specs = Arc::new(vec![ask_spec.clone()]);
    parent.visible_tool_specs = Arc::new(vec![ask_spec]);
    parent
        .visible_tool_names
        .insert("ask_user_clarification".into());
    parent
        .subagent_tool_ceiling_names
        .insert("ask_user_clarification".into());

    let mut definition = AgentDefinitionRegistry::builtins_only()
        .get("task_manager_agent")
        .expect("task manager definition")
        .clone();
    definition.model = tinyagents_definition::ModelSpec::Inherit;
    let definitions =
        Arc::new(AgentDefinitionRegistry::builtins_only().with_definitions([definition]));
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    config.components.juice = false;
    let core = CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(config, DomainSet::full(), ToolGroups::none()).definitions(definitions),
    );
    let mut host_context = crate::agent::tinyagents::host::OpenHumanRunContext::new()
        .with_core(Some(Arc::clone(&core)))
        .with_parent(parent);
    host_context.thread_id = Some("paused-cancel-rpc-thread".into());
    let live = tinyagents_harness::context::RunContext::new(
        tinyagents_harness::context::RunConfig::new("paused-cancel-rpc-parent")
            .with_thread("paused-cancel-rpc-thread"),
        host_context,
    );
    let parent_cancellation = live.data.child().cancellation;
    let jobs =
        (*crate::core::runtime::slot_for::<SubAgentJobRegistry>(&live.data.dispatch_ctx())).clone();
    let paused = SpawnSubagentTool::new()
        .execute_with_live_parent_context(
            json!({
                "agent_id": "task_manager_agent",
                "prompt": "Investigate the paused RPC cancellation fixture",
                "model": "test-model",
                "blocking": true
            }),
            None,
            live.data.child(),
            Some(&live),
        )
        .await
        .expect("initial spawn");
    assert!(!paused.is_error, "{}", paused.output());
    assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let task_id = paused
        .output()
        .lines()
        .find_map(|line| line.strip_prefix("task_id: "))
        .unwrap_or_else(|| panic!("pause envelope carries task id: {}", paused.output()))
        .to_owned();
    let paused_job = jobs
        .list()
        .into_iter()
        .find(|job| {
            job.extensions
                .get("task_id")
                .and_then(serde_json::Value::as_str)
                == Some(task_id.as_str())
        })
        .expect("the paused child is registered in shared Jobs");
    assert_eq!(paused_job.status, SubAgentJobStatus::AwaitingInput);
    assert!(pending_for(workspace.path(), "paused-cancel-rpc-thread").is_empty());
    let sessions = SubagentSessionStore::new(workspace.path().to_path_buf());
    let parent_session = live
        .data
        .parent
        .as_ref()
        .expect("live parent execution context")
        .session_id
        .clone();
    let lifecycle_key = tinyagents_orchestration::subagent::SubagentTaskKey::from_context(
        &live,
        task_id.clone(),
        Some("paused-cancel-rpc-thread".into()),
    );
    // Blocking spawn has no product session row in the legacy contract. Seed
    // the row that a host-owned session surface would expose, using the real
    // paused job's public id, full lifecycle key, and authenticated owner.
    let session = crate::agent::orchestration::subagent_sessions::upsert_running(
        &sessions,
        crate::agent::orchestration::subagent_sessions::SubagentSessionUpsert {
            selector: crate::agent::orchestration::subagent_sessions::SubagentSessionSelector {
                parent_session: parent_session.clone(),
                parent_thread_id: Some("paused-cancel-rpc-thread".into()),
                agent_id: "task_manager_agent".into(),
                model: Some("test-model".into()),
                sandbox_mode: "workspace".into(),
                action_root: None,
                task_key: "paused-cancel-rpc-task".into(),
            },
            display_name: Some("Task Manager".into()),
            task_title: "Pause cancellation fixture".into(),
            worker_thread_id: None,
            task_id: task_id.clone(),
            lifecycle_key: Some(lifecycle_key),
        },
        None,
    )
    .expect("seed durable row for the actual paused job");
    crate::agent::orchestration::subagent_sessions::mark_finished(
        &sessions,
        &session.subagent_session_id,
        &task_id,
        &crate::agent::subagent_host::SubagentRunStatus::AwaitingUser {
            question: "which file should I inspect?".into(),
            options: None,
            checkpoint: None,
        },
        Vec::new(),
    )
    .expect("mark the seeded product row paused");
    let before_row = sessions
        .load()
        .expect("paused durable session list")
        .into_iter()
        .find(|row| row.subagent_session_id == session.subagent_session_id)
        .expect("seeded durable row remains available");
    assert_eq!(before_row.status, DurableSubagentStatus::AwaitingUser);
    assert_eq!(before_row.parent_session, parent_session);
    assert_eq!(
        before_row.parent_thread_id.as_deref(),
        Some("paused-cancel-rpc-thread")
    );
    assert!(before_row.lifecycle_key.is_some());
    assert_eq!(
        live.data
            .dispatch_ctx()
            .resolved()
            .expect("RPC uses the spawned parent's core")
            .workspace_dir()
            .expect("RPC resolves the parent's workspace"),
        workspace.path()
    );

    let controller = crate::agent_runs::subagent_control::all_registered_controllers()
        .into_iter()
        .find(|controller| {
            controller.schema.namespace == "subagent" && controller.schema.function == "cancel"
        })
        .expect("subagent cancel RPC is registered");
    let mut params = serde_json::Map::new();
    params.insert("taskId".into(), json!(task_id));
    params.insert("reason".into(), json!("user requested stop"));
    let cancelled = (controller.handler)(live.data.dispatch_ctx(), params)
        .await
        .expect("cancel RPC result");
    let response_field = |value: &serde_json::Value, name: &str| {
        value
            .get("data")
            .and_then(|data| data.get(name))
            .or_else(|| value.get(name))
            .cloned()
    };
    assert_eq!(response_field(&cancelled, "cancelled"), Some(json!(true)));
    assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(matches!(
        jobs.get(paused_job.id.as_str())
            .expect("paused job remains inspectable")
            .status,
        SubAgentJobStatus::Cancelled
    ));
    let row = SubagentSessionStore::new(workspace.path().to_path_buf())
        .load()
        .expect("durable session")
        .into_iter()
        .find(|row| row.current_task_id.as_deref() == Some(task_id.as_str()))
        .expect("cancelled durable session remains available");
    assert_eq!(row.status, DurableSubagentStatus::Failed);
    assert!(row
        .latest_error
        .as_deref()
        .is_some_and(|error| error.contains("user requested stop")));
    let deliveries = pending_for(workspace.path(), "paused-cancel-rpc-thread");
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].status, CompletionStatus::Failed);
    assert!(deliveries[0].result.text.contains("[SUBAGENT_FAILED]"));
    assert!(deliveries[0].result.text.contains("user requested stop"));

    let mut repeated_params = serde_json::Map::new();
    repeated_params.insert("taskId".into(), json!(task_id));
    let repeated = (controller.handler)(DispatchCtx::new(Arc::clone(&core)), repeated_params)
        .await
        .expect("repeated cancel RPC result");
    assert_eq!(response_field(&repeated, "cancelled"), Some(json!(false)));
    assert_eq!(response_field(&repeated, "outcome"), Some(json!("unknown")));
    forget_workspace_for_test(workspace.path());
    assert!(!parent_cancellation.is_cancelled());
}

#[tokio::test]
async fn delegate_graph_runs_every_stage_through_shared_driver_jobs() {
    use crate::agent::orchestration::tools::DelegateGraphTool;
    use crate::agent_registry::host_definition::AgentDefinitionRegistry;
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::tools::toolpacks::ToolGroups;
    use tinyagents_orchestration::subagent::{SubAgentJobRegistry, SubAgentJobStatus};

    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let provider = Arc::new(ScriptedModel::new(vec![
        ("Produce a short, concrete", "1. inspect the fixture"),
        ("Carry out the plan below", "graph-stage-execution-canary"),
        ("Review the result below", "APPROVE\nThe result is correct."),
    ]));
    let mut parent = parent_context(workspace.path(), provider.clone(), vec![]);
    parent.session_id = "delegate-graph-jobs-e2e".into();
    let mut test_definition = AgentDefinitionRegistry::builtins_only()
        .get("task_manager_agent")
        .expect("task manager definition")
        .clone();
    test_definition.model = tinyagents_definition::ModelSpec::Exact("test-model".into());
    let definitions =
        Arc::new(AgentDefinitionRegistry::builtins_only().with_definitions([test_definition]));
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    let core = CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(config, DomainSet::full(), ToolGroups::none()).definitions(definitions),
    );
    // `OpenHumanRunContext::with_core` retains a weak handle; keep this strong
    // owner alive through the graph execution so the scoped definition
    // catalogue is the one the live parent carries.
    let live = Arc::new(
        crate::agent::tinyagents::host::OpenHumanRunContext::new()
            .with_core(Some(Arc::clone(&core)))
            .with_parent(parent)
            .into_tinyagents(tinyagents_harness::context::RunConfig::new(
                "delegate-graph-jobs-e2e",
            )),
    );
    let result = DelegateGraphTool::new()
        .execute_with_live_parent_context(
            json!({
                "agent_id": "task_manager_agent",
                "task": "Verify the driver-backed graph stage fixture",
                "max_revisions": 1
            }),
            None,
            live.data.child(),
            Some(Arc::clone(&live)),
        )
        .await
        .expect("graph delegation returns a tool result");

    assert!(!result.is_error, "{}", result.output());
    assert!(result.output().contains("graph-stage-execution-canary"));
    assert_eq!(provider.seen.lock().len(), 3);

    let jobs = crate::core::runtime::slot_for::<SubAgentJobRegistry>(&live.data.dispatch_ctx());
    let completed_stages = jobs
        .list()
        .into_iter()
        .filter(|job| {
            job.agent == "task_manager_agent"
                && job.status == SubAgentJobStatus::Completed
                && job.output.as_deref().is_some_and(|output| {
                    output.contains("1. inspect the fixture")
                        || output.contains("graph-stage-execution-canary")
                        || output.contains("APPROVE")
                })
        })
        .count();
    assert_eq!(
        completed_stages, 3,
        "each graph stage is registered in Jobs"
    );
}
