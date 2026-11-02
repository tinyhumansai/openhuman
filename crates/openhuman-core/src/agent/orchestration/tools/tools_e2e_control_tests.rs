use super::*;

struct StopGateModel {
    started: tokio::sync::mpsc::Sender<()>,
    release: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl ChatModel<()> for StopGateModel {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: std::sync::OnceLock<ModelProfile> = std::sync::OnceLock::new();
        Some(PROFILE.get_or_init(|| {
            let mut profile = ModelProfile::default();
            profile.tool_calling = true;
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
        Ok(ModelResponse::assistant("released"))
    }
}

#[tokio::test]
async fn unscoped_web_stop_cancels_a_running_driver_subagent() {
    let workspace = tempfile::TempDir::new().expect("workspace");
    let thread_id = "stop-detached-subagent-thread";
    let (started_tx, mut started_rx) = tokio::sync::mpsc::channel(1);
    let release = Arc::new(tokio::sync::Notify::new());
    let model = Arc::new(StopGateModel {
        started: started_tx,
        release: Arc::clone(&release),
    });
    let mut parent = parent_context(workspace.path(), model, vec![]);
    parent.session_id = "stop-detached-subagent-session".into();
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    config.components.juice = false;
    let definitions = Arc::new(AgentDefinitionRegistry::builtins_only());
    let core = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config.clone(),
    )
    .derive_with(
        crate::core::runtime::ContextOverlay::new(
            config,
            crate::core::runtime::DomainSet::full(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .definitions(definitions),
    );
    let mut host = crate::agent::tinyagents::host::OpenHumanRunContext::new()
        .with_core(Some(Arc::clone(&core)))
        .with_parent(parent);
    host.thread_id = Some(thread_id.into());
    let live = tinyagents_harness::context::RunContext::new(
        tinyagents_harness::context::RunConfig::new("stop-detached-subagent-session")
            .with_thread(thread_id),
        host,
    );
    let accepted = crate::core::runtime::CoreContext::scope(
        Arc::clone(&core),
        Box::pin(
            SpawnSubagentTool::new().execute_with_live_parent_context(
                json!({
                    "agent_id": "task_manager_agent",
                    "prompt": "wait for the thread stop",
                    "model": "test-model",
                    "task_key": "stop detached child"
                }),
                None,
                live.data.child(),
                crate::agent::subagent_host::detached_subagent_parent(&live, "stop-child")
                    .expect("detached parent"),
            ),
        ),
    )
    .await
    .expect("spawn child");
    assert!(!accepted.is_error, "{}", accepted.output());
    let task_id = accepted
        .output()
        .split("[async_subagent_ref]\n")
        .nth(1)
        .and_then(|value| value.split("\n[/async_subagent_ref]").next())
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .and_then(|value| value["task_id"].as_str().map(str::to_owned))
        .expect("accepted child includes its task ID");
    tokio::time::timeout(std::time::Duration::from_secs(5), started_rx.recv())
        .await
        .expect("driver child reached provider gate")
        .expect("provider start");
    let dispatch = live.data.dispatch_ctx();
    assert!(
        dispatch.resolved().is_some(),
        "stop uses the live core context"
    );
    let jobs = crate::core::runtime::slot_for::<
        tinyagents_orchestration::subagent::SubAgentJobRegistry,
    >(&dispatch);
    let before_scoped = super::driver_sessions_for_parent(
        &dispatch,
        workspace.path(),
        "stop-detached-subagent-session",
        Some(thread_id),
    )
    .await
    .expect("load live driver row");
    let row_before = before_scoped
        .iter()
        .find(|row| row.current_task_id.as_deref() == Some(task_id.as_str()))
        .expect("driver persistence includes the live child");
    assert_eq!(row_before.parent_thread_id.as_deref(), Some(thread_id));
    let lifecycle_key = row_before
        .lifecycle_key
        .clone()
        .expect("persisted child row carries its lifecycle key");
    let owner = crate::agent::tinyagents::tool_registration::subagent_owner_key(
        workspace.path(),
        "stop-detached-subagent-session",
        Some(thread_id),
    );
    let job_id = jobs
        .driver_job_for_key(&lifecycle_key, &owner)
        .expect("driver owns a live child job for this lifecycle key");
    let job = jobs
        .get(job_id.as_str())
        .expect("registered driver job remains available");
    assert_eq!(
        job.status,
        tinyagents_orchestration::subagent::SubAgentJobStatus::Running
    );
    assert!(
        before_scoped.iter().any(|row| {
            row.current_task_id.as_deref() == Some(task_id.as_str())
                && row.parent_thread_id.as_deref() == Some(thread_id)
        }),
        "driver persistence is scoped to the stopped thread"
    );
    let before_scoped = serde_json::to_string(&before_scoped).expect("encode live row");

    let scoped = crate::core::runtime::CoreContext::scope(
        Arc::clone(&core),
        crate::web_chat::channel_web_cancel(
            &dispatch,
            "stop-client",
            thread_id,
            Some("req-unrelated"),
        ),
    )
    .await
    .expect("scoped cancel");
    assert_eq!(scoped.value["cancelled"], serde_json::json!(false));
    assert_eq!(scoped.value["subagents_cancelled"], serde_json::json!(0));
    assert_eq!(
        jobs.get(job.id.as_str()).unwrap().status,
        tinyagents_orchestration::subagent::SubAgentJobStatus::Running
    );
    let after_scoped = super::driver_sessions_for_parent(
        &dispatch,
        workspace.path(),
        "stop-detached-subagent-session",
        Some(thread_id),
    )
    .await
    .expect("scoped cancel leaves driver row readable");
    assert_eq!(
        serde_json::to_string(&after_scoped).expect("encode row after scoped cancel"),
        before_scoped,
        "scoped cancellation leaves the sub-agent session unchanged"
    );

    let stop = crate::core::runtime::CoreContext::scope(
        Arc::clone(&core),
        crate::web_chat::channel_web_cancel(&dispatch, "stop-client", thread_id, None),
    )
    .await
    .expect("unscoped stop");
    assert_eq!(stop.value["cancelled"], serde_json::json!(true));
    assert_eq!(stop.value["request_id"], serde_json::Value::Null);
    assert_eq!(stop.value["subagents_cancelled"], serde_json::json!(1));
    release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if jobs.get(job.id.as_str()).is_some_and(|job| {
                job.status == tinyagents_orchestration::subagent::SubAgentJobStatus::Cancelled
            }) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("driver child cancellation settles");
    let stopped_snapshot = super::driver_sessions_for_parent(
        &dispatch,
        workspace.path(),
        "stop-detached-subagent-session",
        Some(thread_id),
    )
    .await
    .expect("stopped driver row readable");
    let stopped_snapshot = serde_json::to_string(&stopped_snapshot).expect("encode stopped row");

    let repeated = crate::core::runtime::CoreContext::scope(
        Arc::clone(&core),
        crate::web_chat::channel_web_cancel(&dispatch, "stop-client", thread_id, None),
    )
    .await
    .expect("repeated stop");
    assert_eq!(repeated.value["subagents_cancelled"], serde_json::json!(0));
    let after_repeat = super::driver_sessions_for_parent(
        &dispatch,
        workspace.path(),
        "stop-detached-subagent-session",
        Some(thread_id),
    )
    .await
    .expect("repeated stop leaves driver row readable");
    assert_eq!(
        serde_json::to_string(&after_repeat).expect("encode repeated stop row"),
        stopped_snapshot,
        "repeated stop does not settle the same child twice"
    );
}

#[tokio::test]
async fn cancel_of_a_completed_driver_child_reports_completed_without_rewriting_its_session() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};

    let workspace = tempfile::TempDir::new().expect("workspace");
    let provider = Arc::new(ScriptedModel::new(vec![(
        "completed-cancel-canary",
        "child completed before late cancel",
    )]));
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(16);
    let mut parent = parent_context(workspace.path(), provider, vec![]);
    parent.session_id = "completed-cancel-parent-session".into();
    parent.on_progress = Some(progress_tx);
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    config.components.juice = false;
    let definitions = Arc::new(AgentDefinitionRegistry::builtins_only());
    let core = CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(
            config,
            DomainSet::full(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .definitions(definitions),
    );
    let thread_id = "completed-cancel-parent-thread";
    let mut host = crate::agent::tinyagents::host::OpenHumanRunContext::new()
        .with_core(Some(Arc::clone(&core)))
        .with_parent(parent);
    host.thread_id = Some(thread_id.into());
    let live = tinyagents_harness::context::RunContext::new(
        tinyagents_harness::context::RunConfig::new("completed-cancel-parent-session")
            .with_thread(thread_id),
        host,
    );
    let result = CoreContext::scope(
        Arc::clone(&core),
        Box::pin(
            SpawnSubagentTool::new().execute_with_live_parent_context(
                json!({
                    "agent_id": "task_manager_agent",
                    "prompt": "Investigate completed-cancel-canary",
                    "model": "test-model",
                    "task_key": "completed child late cancel"
                }),
                None,
                live.data.child(),
                crate::agent::subagent_host::detached_subagent_parent(
                    &live,
                    "completed-cancel-child",
                )
                .expect("detached parent"),
            ),
        ),
    )
    .await
    .expect("spawn tool execution");
    assert!(!result.is_error, "{}", result.output());
    let task_id = result
        .output()
        .split("[async_subagent_ref]\n")
        .nth(1)
        .and_then(|value| value.split("\n[/async_subagent_ref]").next())
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .and_then(|value| value["task_id"].as_str().map(str::to_owned))
        .expect("accepted child task reference");
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match progress_rx.recv().await {
                Some(crate::progress::AgentProgress::SubagentCompleted {
                    task_id: event_task_id,
                    ..
                }) if event_task_id == task_id => break,
                Some(crate::progress::AgentProgress::SubagentFailed {
                    task_id: event_task_id,
                    error,
                    ..
                }) if event_task_id == task_id => panic!("child failed: {error}"),
                Some(_) => {}
                None => panic!("child progress stream closed"),
            }
        }
    })
    .await
    .expect("child completed before the late cancel");

    let dispatch = DispatchCtx::new(Arc::clone(&core));
    let before = super::driver_sessions_for_parent(
        &dispatch,
        workspace.path(),
        "completed-cancel-parent-session",
        Some(thread_id),
    )
    .await
    .expect("load completed driver row");
    assert!(before.iter().any(|row| {
        row.current_task_id.as_deref() == Some(task_id.as_str())
            && row.parent_session == "completed-cancel-parent-session"
            && row.parent_thread_id.as_deref() == Some(thread_id)
    }));
    let before_snapshot = serde_json::to_string(&before).expect("encode completed rows");
    let controller = crate::agent_runs::all_subagent_control_registered_controllers()
        .into_iter()
        .find(|controller| controller.schema.function == "cancel")
        .expect("subagent cancel RPC registered");
    let mut params = serde_json::Map::new();
    params.insert("taskId".into(), json!(task_id));
    let output = (controller.handler)(DispatchCtx::new(Arc::clone(&core)), params)
        .await
        .expect("late cancel handler");
    assert_eq!(output["cancelled"], serde_json::json!(false));
    assert_eq!(output["outcome"], serde_json::json!("completed"));
    let after_snapshot = serde_json::to_string(
        &super::driver_sessions_for_parent(
            &dispatch,
            workspace.path(),
            "completed-cancel-parent-session",
            Some(thread_id),
        )
        .await
        .expect("load driver row after cancel"),
    )
    .expect("encode rows after cancel");
    assert_eq!(
        after_snapshot, before_snapshot,
        "late cancel must not rewrite completed state"
    );
}
