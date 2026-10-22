//! Coverage for the host's tool registration over the runtime builder.

use super::*;
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use tinyagents_harness::runtime::AgentHarness;
use tinytools::{Tool, ToolCallOptions, ToolResult};

#[cfg(unix)]
#[test]
fn subagent_owner_scope_preserves_non_utf8_workspace_identity() {
    use std::os::unix::ffi::OsStringExt;

    let path_a = std::path::PathBuf::from(std::ffi::OsString::from_vec(vec![b'w', 0xff]));
    let path_b = std::path::PathBuf::from(std::ffi::OsString::from_vec(vec![b'w', 0xfe]));
    let owner_a = super::subagent_owner_key(&path_a, "parent", Some("thread"));
    let owner_b = super::subagent_owner_key(&path_b, "parent", Some("thread"));

    assert_ne!(owner_a, owner_b);
}

struct BlockingLiveContextTool {
    started: tokio::sync::mpsc::Sender<()>,
}

#[async_trait]
impl Tool for BlockingLiveContextTool {
    fn name(&self) -> &str {
        "memory"
    }

    fn description(&self) -> &str {
        "Blocks until its parent turn is cancelled."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let _ = self.started.send(()).await;
        std::future::pending::<()>().await;
        unreachable!("the live-parent cancellation must win")
    }
}

/// Register the real goal tools on a fresh harness and return the candidate
/// names handed on to the exposure layer.
fn candidate_names(has_thread: bool) -> Vec<String> {
    let workspace = tempfile::TempDir::new().expect("workspace");
    let tools = crate::agent::goals::goal_tools(workspace.path());
    let mut harness: AgentHarness<(), OpenHumanRunContext> = AgentHarness::new();

    let report = host_tool_set_builder(&[Arc::new(tools)], &None, has_thread, &HashSet::new())
        .register(&mut harness, None);
    assert_eq!(report.registered.len(), harness.tools().names().len());
    report.plan.candidates
}

#[test]
fn thread_less_turn_drops_goal_tools_from_the_exposure_candidates() {
    // The exposure layer diffs these against the registered tools; a
    // candidate that is never registered would log a false divergence warn.
    let candidates = candidate_names(false);

    assert!(
        candidates.iter().all(|name| !is_thread_goal_tool(name)),
        "goal tools must not be exposure candidates without a thread: {candidates:?}"
    );
}

#[test]
fn threaded_turn_keeps_goal_tools_as_exposure_candidates() {
    let candidates = candidate_names(true);

    for name in ["goal_get", "goal_set", "goal_complete"] {
        assert!(
            candidates.iter().any(|candidate| candidate == name),
            "{name} must stay a candidate on a threaded turn: {candidates:?}"
        );
    }
}

#[tokio::test]
async fn static_live_harness_builder_resolves_control_aliases_from_the_execution_parent() {
    use crate::agent::harness::fork_context::ParentExecutionContext;
    use crate::agent::orchestration::subagent_sessions::{
        DurableSubagentSession, DurableSubagentStatus, SubagentSessionStore,
    };
    use tinyagents_harness::{
        agent_loop::phases::execute_tool_batch, context::RunConfig, ComponentId,
    };

    let workspace = tempfile::tempdir().expect("live harness workspace");
    let config = crate::config::Config::default();
    let core = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config,
    );
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    let parent = ParentExecutionContext {
        runtime_config: None,
        agent_definition_id: "live-orchestrator".into(),
        allowed_subagent_ids: HashSet::new(),
        turn_model_source: crate::agent::tinyagents::TurnModelSource::from_model(model),
        all_tools: Arc::new(Vec::new()),
        all_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: HashSet::new(),
        visible_tool_specs: Arc::new(Vec::new()),
        subagent_tool_ceiling_names: HashSet::new(),
        model_name: "test-model".into(),
        temperature: 0.0,
        workspace_dir: workspace.path().to_path_buf(),
        workspace_descriptor: None,
        agent_config: Default::default(),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(None),
        session_id: "live-parent-session".into(),
        channel: "voice-live-test".into(),
        connected_integrations: Vec::new(),
        tool_call_format: tinytools_agent::dialect::ToolCallFormat::Native,
        session_key: "live-parent-session".into(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
        host_env: Default::default(),
    };
    let store = SubagentSessionStore::new(workspace.path().to_path_buf());
    store
        .save(&[DurableSubagentSession {
            subagent_session_id: "live-owned-session".into(),
            parent_session: "live-parent-session".into(),
            parent_thread_id: Some("voice-thread".into()),
            worker_thread_id: None,
            agent_id: "researcher".into(),
            display_name: None,
            model: None,
            sandbox_mode: "workspace".into(),
            action_root: None,
            task_key: "research".into(),
            task_title: "Research task".into(),
            current_task_id: Some("live-task".into()),
            lifecycle_key: None,
            status: DurableSubagentStatus::Idle,
            reusable: true,
            latest_history: None,
            latest_error: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            last_used_at: "2026-01-01T00:00:00Z".into(),
        }])
        .expect("seed one scoped durable row");

    let tools: SharedToolSet = Arc::new(vec![Box::new(
        crate::agent::orchestration::tools::ListSubagentsTool::new(),
    )]);
    let sets = vec![tools];
    let mut harness: AgentHarness<(), OpenHumanRunContext> = AgentHarness::new();
    host_tool_set_builder(&sets, &None, true, &HashSet::new()).register(&mut harness, None);
    let mut live_context = OpenHumanRunContext::new()
        .with_core(Some(Arc::clone(&core)))
        .with_parent(parent);
    live_context.thread_id = Some("voice-thread".into());
    let mut run = live_context
        .into_tinyagents(RunConfig::new("live-harness-parent").with_thread("voice-thread"));
    let mut status = tinyagents_harness::events::HarnessRunStatus::new(
        run.run_id().clone(),
        ComponentId::new("live-harness-test".to_owned()),
    );
    let mut agent_run = tinyagents_harness::middleware::AgentRun::new();
    let mut messages = Vec::new();
    let result = execute_tool_batch(
        &harness,
        &(),
        &mut run,
        &mut agent_run,
        &mut status,
        &mut messages,
        vec![tinyinference_llm::tool::ToolCall::new(
            "live-list-call",
            "list_subagents",
            serde_json::json!({}),
        )],
    )
    .await
    .expect("the live control resolves its execution parent");
    assert_eq!(result.results.len(), 1);
    assert!(
        result.results[0].text().contains("live-owned-session"),
        "live control query should project its scoped session, got: {}",
        result.results[0].text()
    );
}

#[tokio::test]
async fn live_close_alias_enforces_parent_thread_ownership() {
    use crate::agent::harness::fork_context::ParentExecutionContext;
    use crate::agent::orchestration::subagent_sessions::{
        DurableSubagentSession, DurableSubagentStatus, SubagentSessionStore,
    };
    use tinyagents_harness::{
        agent_loop::phases::execute_tool_batch, context::RunConfig, ComponentId,
    };

    let workspace = tempfile::tempdir().expect("live harness workspace");
    let config = crate::config::Config::default();
    let core = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config,
    );
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    let parent = ParentExecutionContext {
        runtime_config: None,
        agent_definition_id: "live-orchestrator".into(),
        allowed_subagent_ids: HashSet::new(),
        turn_model_source: crate::agent::tinyagents::TurnModelSource::from_model(model),
        all_tools: Arc::new(Vec::new()),
        all_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: HashSet::new(),
        visible_tool_specs: Arc::new(Vec::new()),
        subagent_tool_ceiling_names: HashSet::new(),
        model_name: "test-model".into(),
        temperature: 0.0,
        workspace_dir: workspace.path().to_path_buf(),
        workspace_descriptor: None,
        agent_config: Default::default(),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(None),
        session_id: "live-close-parent-session".into(),
        channel: "voice-live-test".into(),
        connected_integrations: Vec::new(),
        tool_call_format: tinytools_agent::dialect::ToolCallFormat::Native,
        session_key: "live-close-parent-session".into(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
        host_env: Default::default(),
    };
    let sessions = SubagentSessionStore::new(workspace.path().to_path_buf());
    let row = |id: &str, thread: &str| DurableSubagentSession {
        subagent_session_id: id.into(),
        parent_session: "live-close-parent-session".into(),
        parent_thread_id: Some(thread.into()),
        worker_thread_id: None,
        agent_id: "researcher".into(),
        display_name: None,
        model: None,
        sandbox_mode: "workspace".into(),
        action_root: None,
        task_key: id.into(),
        task_title: "Research task".into(),
        current_task_id: Some(format!("task-{id}")),
        lifecycle_key: None,
        status: DurableSubagentStatus::Idle,
        reusable: true,
        latest_history: None,
        latest_error: None,
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
        last_used_at: "2026-01-01T00:00:00Z".into(),
    };
    sessions
        .save(&[
            row("owned-session", "voice-thread"),
            row("foreign-session", "other-thread"),
        ])
        .expect("seed scoped rows");

    let tools: SharedToolSet = Arc::new(vec![Box::new(
        crate::agent::orchestration::tools::CloseSubagentTool::new(),
    )]);
    let sets = vec![tools];
    let mut harness: AgentHarness<(), OpenHumanRunContext> = AgentHarness::new();
    host_tool_set_builder(&sets, &None, true, &HashSet::new()).register(&mut harness, None);
    let mut live_context = OpenHumanRunContext::new()
        .with_core(Some(Arc::clone(&core)))
        .with_parent(parent);
    live_context.thread_id = Some("voice-thread".into());
    let mut run = live_context
        .into_tinyagents(RunConfig::new("live-close-parent").with_thread("voice-thread"));
    let mut status = tinyagents_harness::events::HarnessRunStatus::new(
        run.run_id().clone(),
        ComponentId::new("live-close-test".to_owned()),
    );
    let mut agent_run = tinyagents_harness::middleware::AgentRun::new();
    let mut messages = Vec::new();
    let foreign = execute_tool_batch(
        &harness,
        &(),
        &mut run,
        &mut agent_run,
        &mut status,
        &mut messages,
        vec![tinyinference_llm::tool::ToolCall::new(
            "close-foreign-call",
            "close_subagent",
            serde_json::json!({"subagent_session_id":"foreign-session"}),
        )],
    )
    .await
    .expect("a foreign selector is returned as a recoverable model-facing tool error");
    assert!(
        foreign.results.iter().any(|message| {
            message.text().contains("session") || message.text().contains("subagent")
        }),
        "the registered alias must return a selector denial to the model"
    );
    assert!(sessions.load().unwrap().iter().any(|row| {
        row.subagent_session_id == "foreign-session" && row.status == DurableSubagentStatus::Idle
    }));

    let owned = execute_tool_batch(
        &harness,
        &(),
        &mut run,
        &mut agent_run,
        &mut status,
        &mut messages,
        vec![tinyinference_llm::tool::ToolCall::new(
            "close-owned-call",
            "close_subagent",
            serde_json::json!({"subagent_session_id":"owned-session"}),
        )],
    )
    .await
    .expect("owned close tool call");
    assert!(
        !owned.results.is_empty(),
        "owned live close should complete through the registered alias"
    );
    assert!(sessions.load().unwrap().iter().any(|row| {
        row.subagent_session_id == "owned-session" && row.status == DurableSubagentStatus::Closed
    }));
}

#[tokio::test]
async fn registered_memory_dispatch_enforces_one_budget_across_distinct_calls() {
    use crate::memory::test_fixtures::{bind_reference, config_in};
    use tinyagents_harness::{context::RunConfig, CallId};
    use tinytools::ToolCallOptions;
    let workspace = tempfile::tempdir().unwrap();
    let config = config_in(&workspace);
    let _engine = bind_reference(&config);
    let tools: Arc<Vec<Box<dyn tinytools::Tool>>> = Arc::new(vec![Box::new(
        crate::memory::tools::MemoryTool::new(Arc::new(config.clone())),
    )]);
    let adapter = shared_tool_adapter(vec![tools], "memory").unwrap();
    let dispatch = typed_dispatch_for("memory", Arc::new(adapter), None)
        .expect("memory registration must preserve the harness run context");
    let parent = OpenHumanRunContext::new().into_tinyagents(RunConfig::new("memory-dispatch"));
    for index in 0..9 {
        let result = dispatch
            .execute(
                &(),
                CallId::new(format!("call-{index}")),
                serde_json::json!({"action":"fetch","query":format!("fact {index}")}),
                ToolCallOptions::default(),
                &parent,
            )
            .await
            .unwrap();
        assert_eq!(result.is_error, index == 8, "call {index}");
    }
}

#[tokio::test]
async fn memory_live_context_dispatch_cancels_with_parent_without_creating_a_child_job() {
    use tinyagents_harness::context::RunConfig;
    use tinyagents_harness::tool::ToolDispatch;

    let (started_tx, mut started_rx) = tokio::sync::mpsc::channel(1);
    let tool: Arc<dyn Tool> = Arc::new(BlockingLiveContextTool {
        started: started_tx,
    });
    let dispatch = LiveToolContextDispatch::new(tool);
    let cancellation = tinyagents_harness::CancellationToken::new();
    let config = crate::config::Config::default();
    let core = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config,
    );
    let parent = OpenHumanRunContext::new()
        .with_core(Some(Arc::clone(&core)))
        .with_cancellation(cancellation.clone())
        .into_tinyagents(RunConfig::new("memory-live-context-cancel"));
    let jobs = crate::core::runtime::slot_for::<
        tinyagents_orchestration::subagent::SubAgentJobRegistry,
    >(&parent.data.dispatch_ctx());
    assert!(jobs.list().is_empty(), "fresh parent has no child jobs");

    let execution = dispatch.execute(
        &(),
        tinyagents_harness::CallId::new("memory-cancel-call"),
        serde_json::json!({}),
        ToolCallOptions::default(),
        &parent,
    );
    tokio::pin!(execution);
    tokio::select! {
        _ = started_rx.recv() => cancellation.cancel(),
        result = &mut execution => panic!("blocking memory tool returned before cancellation: {result:?}"),
    }
    let result = execution
        .await
        .expect("parent cancellation returns a tool result");
    assert!(result.is_error);
    assert!(result.output().contains("cancelled"));
    assert!(
        jobs.list().is_empty(),
        "memory remains a tool on the parent run"
    );
}

struct OriginObservingTool(Arc<AtomicBool>);

#[async_trait]
impl Tool for OriginObservingTool {
    fn name(&self) -> &str {
        "origin_observer"
    }
    fn description(&self) -> &str {
        "Observes the per-turn core authority."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("executed"))
    }
    async fn execute_with_context(
        &self,
        _args: serde_json::Value,
        _options: ToolCallOptions,
        _context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.0.store(
            matches!(
                crate::core::runtime::CoreContext::current_turn_origin(),
                Some(crate::security::turn_origin::AgentTurnOrigin::ExternalChannel { .. })
            ),
            Ordering::SeqCst,
        );
        Ok(ToolResult::success("executed"))
    }
}

#[tokio::test]
async fn shared_adapter_scopes_tool_execution_to_the_run_origin() {
    use tinyagents_harness::context::RunConfig;

    let context =
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::full(), None);
    let observed = Arc::new(AtomicBool::new(false));
    let tools: Vec<Arc<Vec<Box<dyn Tool>>>> = vec![Arc::new(vec![Box::new(OriginObservingTool(
        observed.clone(),
    ))])];
    let adapter = shared_tool_adapter(tools, "origin_observer").unwrap();
    let mut host = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    host.origin = Some(
        crate::security::turn_origin::AgentTurnOrigin::ExternalChannel {
            sender_name: None,
            channel: "test".into(),
            sender: None,
            reply_target: "room".into(),
            message_id: "message".into(),
            history_key: None,
        },
    );
    let run = host.into_tinyagents(RunConfig::new("origin-observer"));
    let tool_context = tinyagents_harness::tool::ToolExecutionContext::from_run_context(
        &run,
        tinyagents_harness::ids::CallId::new("call"),
    );

    crate::core::runtime::CoreContext::scope(context, async {
        adapter
            .execute_with_context(
                serde_json::json!({}),
                ToolCallOptions::default(),
                Some(&tool_context),
            )
            .await
            .expect("tool execution succeeds");
        assert!(observed.load(Ordering::SeqCst));
        assert!(crate::core::runtime::CoreContext::current_turn_origin().is_none());
    })
    .await;
}
