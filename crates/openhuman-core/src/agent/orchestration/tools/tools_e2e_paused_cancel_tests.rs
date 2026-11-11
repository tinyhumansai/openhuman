use super::*;

#[tokio::test]
async fn cancelling_a_paused_driver_settles_once_with_its_reason() {
    use crate::agent::orchestration::background_completions::{
        forget_workspace_for_test, pending_for_ctx,
    };
    use crate::agent::orchestration::subagent_sessions::DurableSubagentStatus;
    use crate::agent::orchestration::tools::SpawnSubagentTool;
    use tinyagents_orchestration::subagent::SubAgentJobRegistry;
    use tinyagents_orchestration::subagent::SubAgentJobStatus;
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
    parent.session_id = "paused-cancel-rpc-parent".into();
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(64);
    parent.on_progress = Some(progress_tx);
    parent.all_tools = Arc::new(vec![ask_tool]);
    parent.all_tool_specs = Arc::new(vec![ask_spec.clone()]);
    parent.visible_tool_specs = Arc::new(vec![ask_spec]);
    parent
        .visible_tool_names
        .insert("ask_user_clarification".into());
    parent
        .subagent_tool_ceiling_names
        .insert("ask_user_clarification".into());

    let (live, _core) = super::continuation_tests::core_backed_live_parent(
        workspace.path(),
        parent,
        "paused-cancel-rpc-parent",
        "paused-cancel-rpc-thread",
    );
    assert!(
        live.data.dispatch_ctx().resolved().is_some(),
        "the parent carrier retains its core context"
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
            crate::agent::subagent_host::detached_subagent_parent(&live, "paused-cancel-child")
                .expect("detached parent"),
        )
        .await
        .expect("initial spawn");
    assert!(!paused.is_error, "{}", paused.output());
    let paused_ref = paused
        .output()
        .split("[async_subagent_ref]\n")
        .nth(1)
        .and_then(|value| value.split("\n[/async_subagent_ref]").next())
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .unwrap_or_else(|| {
            panic!(
                "accepted envelope carries task reference: {}",
                paused.output()
            )
        });
    let task_id = paused_ref["task_id"]
        .as_str()
        .expect("accepted task id")
        .to_owned();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match progress_rx.recv().await {
                Some(crate::progress::AgentProgress::SubagentAwaitingUser {
                    task_id: event_task_id,
                    ..
                }) if event_task_id == task_id => break,
                Some(_) => {}
                None => panic!("parent progress stream closed before the pause was projected"),
            }
        }
    })
    .await
    .expect("host pause projection follows durable checkpoint persistence");
    assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
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
    let queued = pending_for_ctx(
        &live.data.dispatch_ctx(),
        workspace.path(),
        "paused-cancel-rpc-thread",
    );
    assert!(
        queued.iter().any(|record| {
            record.label.as_deref()
                == Some(crate::agent::orchestration::background_completions::AWAITING_INPUT_LABEL)
        }),
        "the asynchronous pause notice is queued independently"
    );
    let parent_session = live
        .data
        .parent
        .as_ref()
        .expect("live parent execution context")
        .session_id
        .clone();
    let before_row = super::driver_sessions_for_parent(
        &live.data.dispatch_ctx(),
        workspace.path(),
        &parent_session,
        Some("paused-cancel-rpc-thread"),
    )
    .await
    .expect("paused driver session list")
    .into_iter()
    .find(|row| row.current_task_id.as_deref() == Some(task_id.as_str()))
    .expect("actual paused durable session remains available");
    assert_eq!(before_row.status, DurableSubagentStatus::AwaitingUser);
    assert_eq!(before_row.parent_session, parent_session);
    assert_eq!(
        before_row.parent_thread_id.as_deref(),
        Some("paused-cancel-rpc-thread")
    );
    assert!(before_row.lifecycle_key.is_some());
    assert!(
        crate::agent::orchestration::completion_owners::driver_task_owner(
            &live.data.dispatch_ctx(),
            &task_id,
        )
        .is_some(),
        "spawn records the owner scope needed by the cancel RPC"
    );
    let owner_scope = crate::agent::orchestration::completion_owners::driver_task_owner(
        &live.data.dispatch_ctx(),
        &task_id,
    )
    .expect("spawn owner metadata remains available");
    let registry_owner = crate::agent::tinyagents::tool_registration::subagent_owner_key(
        &owner_scope.workspace_dir,
        &owner_scope.parent_session,
        owner_scope.parent_thread_id.as_deref(),
    );
    let registry_scope = tinyagents_orchestration::subagent::ParentJobScope::new(
        owner_scope.workspace.clone(),
        owner_scope.parent_thread_id.clone().expect("parent thread"),
    );
    let scoped_jobs = jobs
        .list_for_parent_scope(&registry_owner, &registry_scope)
        .expect("list exact owner-scoped driver jobs");
    assert!(
        scoped_jobs.iter().any(|job| {
            job.extensions
                .get("task_id")
                .and_then(serde_json::Value::as_str)
                == Some(task_id.as_str())
        }),
        "exact owner/workspace/thread scope must include the paused job; owner={registry_owner}, workspace={:?}, thread={:?}, jobs={:?}",
        owner_scope.workspace,
        owner_scope.parent_thread_id,
        scoped_jobs
            .iter()
            .map(|job| (&job.id, &job.status, &job.extensions))
            .collect::<Vec<_>>(),
    );
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
    let row = super::driver_sessions_for_parent(
        &live.data.dispatch_ctx(),
        workspace.path(),
        &parent_session,
        Some("paused-cancel-rpc-thread"),
    )
    .await
    .expect("driver session")
    .into_iter()
    .find(|row| row.current_task_id.as_deref() == Some(task_id.as_str()))
    .expect("cancelled durable session remains available");
    assert_eq!(row.status, DurableSubagentStatus::Failed);
    assert!(row
        .latest_error
        .as_deref()
        .is_some_and(|error| error.contains("user requested stop")));
    let deliveries = pending_for_ctx(
        &live.data.dispatch_ctx(),
        workspace.path(),
        "paused-cancel-rpc-thread",
    );
    let failures: Vec<_> = deliveries
        .iter()
        .filter(|record| record.task_id == task_id && record.status == CompletionStatus::Failed)
        .collect();
    assert_eq!(failures.len(), 1, "cancellation settles once for this task");
    assert!(failures[0].result.text.contains("[SUBAGENT_FAILED]"));
    assert!(failures[0].result.text.contains("user requested stop"));

    let mut repeated_params = serde_json::Map::new();
    repeated_params.insert("taskId".into(), json!(task_id));
    let repeated = (controller.handler)(live.data.dispatch_ctx(), repeated_params)
        .await
        .expect("repeated cancel RPC result");
    assert_eq!(response_field(&repeated, "cancelled"), Some(json!(false)));
    assert_eq!(response_field(&repeated, "outcome"), Some(json!("unknown")));
    let deliveries_after_repeat = pending_for_ctx(
        &live.data.dispatch_ctx(),
        workspace.path(),
        "paused-cancel-rpc-thread",
    );
    assert_eq!(
        deliveries_after_repeat
            .iter()
            .filter(|record| record.task_id == task_id && record.status == CompletionStatus::Failed)
            .count(),
        1,
        "repeated cancellation does not enqueue another terminal failure"
    );
    forget_workspace_for_test(workspace.path());
    assert!(!parent_cancellation.is_cancelled());
}

#[tokio::test]
async fn stopping_a_paused_driver_releases_its_active_owner() {
    use crate::agent::orchestration::tools::SpawnSubagentTool;
    use tinyagents_orchestration::subagent::SubAgentJobRegistry;
    use tinyagents_orchestration::subagent::SubAgentJobStatus;

    let _lock = crate::config::TEST_ENV_LOCK.lock().await;
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::tempdir().expect("workspace");
    let parent_session = "paused-stop-owner-parent";
    let thread_id = "paused-stop-owner-thread";
    let ask_tool: Box<dyn tinytools::Tool> =
        Box::new(tinyagents_harness_tools::AskClarificationTool::new());
    let ask_spec = Arc::new(ask_tool.spec());
    let mut parent = parent_context(
        workspace.path(),
        Arc::new(PauseThenAnswerModel {
            calls: std::sync::atomic::AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        }),
        vec![],
    );
    parent.all_tools = Arc::new(vec![ask_tool]);
    parent.all_tool_specs = Arc::new(vec![ask_spec.clone()]);
    parent.visible_tool_specs = Arc::new(vec![ask_spec]);
    parent
        .visible_tool_names
        .insert("ask_user_clarification".into());
    parent
        .subagent_tool_ceiling_names
        .insert("ask_user_clarification".into());
    let (live, _core) = super::continuation_tests::core_backed_live_parent(
        workspace.path(),
        parent,
        parent_session,
        thread_id,
    );
    let jobs =
        (*crate::core::runtime::slot_for::<SubAgentJobRegistry>(&live.data.dispatch_ctx())).clone();
    let result = SpawnSubagentTool::new()
        .execute_with_live_parent_context(
            json!({
                "agent_id": "task_manager_agent",
                "prompt": "pause before stop",
                "model": "test-model",
                "blocking": true
            }),
            None,
            live.data.child(),
            crate::agent::subagent_host::detached_subagent_parent(&live, "paused-stop-child")
                .expect("detached parent"),
        )
        .await
        .expect("spawn paused child");
    assert!(!result.is_error, "{}", result.output());
    let task_id = result
        .output()
        .split("[async_subagent_ref]\n")
        .nth(1)
        .and_then(|value| value.split("\n[/async_subagent_ref]").next())
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .and_then(|value| value["task_id"].as_str().map(str::to_owned))
        .expect("accepted task id");
    let dispatch = live.data.dispatch_ctx();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let paused = jobs.list().into_iter().any(|job| {
                job.extensions
                    .get("task_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(task_id.as_str())
                    && job.status == SubAgentJobStatus::AwaitingInput
            });
            if paused {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("selected child reaches AwaitingInput in its driver job");
    assert_eq!(
        crate::agent::orchestration::running_subagents::stop_for_thread(&dispatch, thread_id),
        vec![task_id.clone()]
    );
    assert_eq!(
        jobs.list()
            .into_iter()
            .find(|job| job
                .extensions
                .get("task_id")
                .and_then(serde_json::Value::as_str)
                == Some(task_id.as_str()))
            .map(|job| job.status),
        Some(SubAgentJobStatus::Cancelled)
    );
    assert!(
        crate::agent::orchestration::completion_owners::driver_tasks_for_thread(
            &dispatch,
            workspace.path(),
            thread_id,
        )
        .is_empty(),
        "a cancelled paused child must release its active owner"
    );
}
