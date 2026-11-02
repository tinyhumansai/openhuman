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

    for name in ["goal_get", "goal_set", "goal_complete", "goal_resume"] {
        assert!(
            candidates.iter().any(|candidate| candidate == name),
            "{name} must stay a candidate on a threaded turn: {candidates:?}"
        );
    }
}

struct NamedTopologyTool(&'static str);

#[async_trait]
impl Tool for NamedTopologyTool {
    fn name(&self) -> &str {
        self.0
    }

    fn description(&self) -> &str {
        "Topology projection test tool."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }
}

#[test]
fn capability_diagnostics_omit_retired_graphs() {
    let names = [
        "agent_teams:member",
        "delegation",
        "workflow_runs:scheduler_preview",
    ];
    let tools: SharedToolSet = Arc::new(
        names
            .iter()
            .map(|name| Box::new(NamedTopologyTool(name)) as Box<dyn Tool>)
            .collect(),
    );
    let diagnostics = capability_diagnostics(&[], &[tools], &names.map(str::to_owned));

    let reused_names: Vec<_> = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.name.as_str())
        .collect();
    assert!(
        reused_names.is_empty(),
        "retired graph diagnostics: {reused_names:?}"
    );
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
    let dispatch = typed_dispatch_for("memory", Arc::new(adapter))
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
                Some(crate::core::runtime::turn_origin::AgentTurnOrigin::ExternalChannel { .. })
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
        crate::core::runtime::turn_origin::AgentTurnOrigin::ExternalChannel {
            sender_name: None,
            channel: "test".into(),
            sender: None,
            reply_target: "room".into(),
            message_id: "message".into(),
            history_key: None,
            channel_session: None,
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
