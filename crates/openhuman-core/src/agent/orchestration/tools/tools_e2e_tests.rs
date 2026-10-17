use super::{ArchetypeDelegationTool, DelegationTarget, SpawnSubagentTool, SpawnWorkerThreadTool};
use crate::agent::harness::host_definition::AgentDefinitionRegistry;
use crate::agent::harness::ParentExecutionContext;
use crate::agent::prompts::ConnectedIntegration;
use crate::threads::store as conversations;
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use tinyagents_harness::context::RunConfig;
use tinyagents_session::transcript::TranscriptMessage;
use tinyinference_llm::message::Message;
use tinyinference_llm::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};
use tinytools_agent::dialect::ToolCallFormat;

const SPAWN_SUBAGENT_CANARY: &str = "tool-e2e-spawn-subagent-canary";
const ARCHETYPE_DELEGATION_CANARY: &str = "tool-e2e-archetype-delegation-canary";
const WORKER_THREAD_CANARY: &str = "tool-e2e-worker-thread-canary";

#[tokio::test]
async fn spawn_subagent_tool_runs_child_agent_e2e() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let provider = Arc::new(ScriptedModel::new(vec![(
        SPAWN_SUBAGENT_CANARY,
        "spawn-subagent-child-answer",
    )]));

    let live = super::live_parent_run_context(
        "spawn-subagent-e2e",
        parent_context(workspace.path(), provider.clone(), vec![]),
    );
    // Boxed: the inline spawn future is too large for a test thread's stack.
    let result = Box::pin(SpawnSubagentTool::new().execute_with_live_parent_context(
        json!({
            "agent_id": "task_manager_agent",
            "prompt": format!("Investigate {SPAWN_SUBAGENT_CANARY}"),
            "context": "parent supplied context",
            "model": "test-model",
            "blocking": true
        }),
        None,
        live.data.child(),
        Some(&live),
    ))
    .await
    .expect("tool execution");

    assert!(!result.is_error, "{}", result.output());
    assert_eq!(result.output(), "spawn-subagent-child-answer");
    assert!(provider.saw(SPAWN_SUBAGENT_CANARY));
    assert!(provider.saw("parent supplied context"));
}

/// A child the harness stops early (failure breaker / iteration cap) is not a
/// finished delegation: the parent's tool result must be a failure, so its own
/// failure policy and its trace see it, and the `SubagentCompleted` progress
/// event must say how the child was stopped.
#[tokio::test]
async fn spawn_subagent_reports_a_stopped_child_as_a_failed_tool_result() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(512);
    let parent = parent_context(workspace.path(), Arc::new(LoopingToolModel), vec![]);
    // The live parent run a real turn hands the tool, with the turn's
    // progress channel on the child's run context.
    let live = super::live_parent_run_context("stopped-child-test", parent);
    let mut run_context = live.data.child();
    run_context.progress = Some(progress_tx);
    // Boxed: the inline spawn future is too large for a test thread's
    // stack (the trait `execute` path boxes it the same way).
    let tool = SpawnSubagentTool::new();
    let result = Box::pin(tool.execute_with_live_parent_context(
        json!({
            "agent_id": "task_manager_agent",
            "prompt": "List the task sources",
            "model": "test-model",
            "blocking": true
        }),
        None,
        run_context,
        Some(&live),
    ))
    .await
    .expect("tool execution");

    assert!(
        result.is_error,
        "a stopped child must fail the parent's tool call: {}",
        result.output()
    );
    assert!(
        result.output().contains("[SUBAGENT_INCOMPLETE]"),
        "{}",
        result.output()
    );

    let mut stop = None;
    while let Ok(event) = progress_rx.try_recv() {
        if let crate::agent::progress::AgentProgress::SubagentCompleted { stop: s, .. } = event {
            stop = Some(s);
        }
    }
    let stop = stop
        .expect("SubagentCompleted was emitted")
        .expect("SubagentCompleted carries the child's stop");
    assert!(matches!(
        stop.kind,
        tinyagents_harness::turn_stop::TurnStopKind::Breaker
            | tinyagents_harness::turn_stop::TurnStopKind::IterationCap
    ));
}

#[tokio::test]
async fn archetype_delegation_tool_runs_child_agent_e2e() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let provider = Arc::new(ScriptedModel::new(vec![(
        ARCHETYPE_DELEGATION_CANARY,
        "archetype-delegation-child-answer",
    )]));
    let tool = ArchetypeDelegationTool {
        tool_name: "delegate_task_manager_agent".to_string(),
        agent_id: DelegationTarget("task_manager_agent".to_string()),
        tool_description: "Delegate research work.".to_string(),
    };

    let live = super::live_parent_run_context(
        "archetype-delegation-e2e",
        parent_context(workspace.path(), provider.clone(), vec![]),
    );
    let result = super::archetype_delegation::execute_archetype_delegation_with_live_parent(
        &tool.agent_id.0,
        &tool.tool_name,
        json!({
            "prompt": format!("Research {ARCHETYPE_DELEGATION_CANARY}"),
            "model": "test-model"
        }),
        None,
        live.data.child(),
        Some(&live),
    )
    .await
    .expect("tool execution");

    assert!(!result.is_error, "{}", result.output());
    // This archetype delegation asks for async but has no delivery thread in
    // a test, so it falls back to running inline — and a fallback registers
    // no durable worker either, so the result says so (#6033).
    let output = result.output();
    assert!(
        output.starts_with("archetype-delegation-child-answer"),
        "the child's answer must lead the result: {output}"
    );
    assert!(
        output.contains("[INLINE_RESULT]"),
        "an async delegation that fell back to blocking must disclose that no worker exists: {output}"
    );
    assert!(provider.saw(ARCHETYPE_DELEGATION_CANARY));
}

#[tokio::test]
async fn archetype_delegation_defaults_to_async_with_durable_session_e2e() {
    // The continuity contract: with a parent turn AND a chat thread to
    // deliver into, a delegate_* call must NOT run inline — it returns an
    // [async_subagent_ref] immediately (task_id + subagent_session_id the
    // orchestrator can steer/wait/continue by) and persists a durable
    // session in the workspace store. The finished result is then queued
    // for background delivery as a new chat turn.
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let provider = Arc::new(ScriptedModel::new(vec![(
        ARCHETYPE_DELEGATION_CANARY,
        "async-delegation-child-answer",
    )]));
    let tool = ArchetypeDelegationTool {
        tool_name: "delegate_task_manager_agent".to_string(),
        agent_id: DelegationTarget("task_manager_agent".to_string()),
        tool_description: "Delegate research work.".to_string(),
    };

    let mut ctx = parent_context(workspace.path(), provider.clone(), vec![]);
    ctx.session_id = "tools-e2e-async-session".into();
    let mut parent_data = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    parent_data.thread_id = Some("thread-async-parent".into());
    let parent_run = parent_data
        .with_parent(ctx.clone())
        .into_tinyagents(RunConfig::new("archetype-async-e2e").with_thread("thread-async-parent"));
    let result = super::archetype_delegation::execute_archetype_delegation_with_live_parent(
        &tool.agent_id.0,
        &tool.tool_name,
        json!({
            "prompt": format!("Research {ARCHETYPE_DELEGATION_CANARY} in the background"),
            "model": "test-model"
        }),
        None,
        parent_run.data.child(),
        Some(&parent_run),
    )
    .await
    .expect("tool execution");

    assert!(!result.is_error, "{}", result.output());
    let out = result.output();
    assert!(
        out.contains("[async_subagent_ref]"),
        "async ref returned immediately, not the child's answer: {out}"
    );
    assert!(out.contains("\"task_id\":\"sub-"), "carries task_id: {out}");
    assert!(
        out.contains("subagent_session_id"),
        "carries durable id: {out}"
    );

    // The durable session must exist for this parent, and reach a terminal
    // reusable state once the (scripted, near-instant) child finishes.
    use crate::agent::orchestration::subagent_sessions::{
        self, DurableSubagentStatus, SubagentSessionStore,
    };
    let store = SubagentSessionStore::new(workspace.path().to_path_buf());
    let mut finished = None;
    for _ in 0..600 {
        let sessions = subagent_sessions::list_for_parent(&store, "tools-e2e-async-session", None)
            .expect("durable store readable");
        if let Some(session) = sessions.first() {
            if session.status == DurableSubagentStatus::Idle {
                finished = Some(session.clone());
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let session = finished.expect("durable session reached Idle after background completion");
    assert_eq!(session.agent_id, "task_manager_agent");
    assert!(
        session
            .latest_history
            .as_ref()
            .is_some_and(|h| !h.is_empty()),
        "resumable history persisted"
    );
    // Completion queued for delivery back into the parent chat as a new turn.
    assert!(
        !crate::agent::orchestration::background_completions::pending_for(
            workspace.path(),
            "thread-async-parent",
        )
        .is_empty(),
        "finished result queued for background delivery"
    );
    crate::agent::orchestration::background_completions::forget_workspace_for_test(
        workspace.path(),
    );
}

#[tokio::test]
async fn continue_subagent_resumes_idle_durable_session_e2e() {
    // The "looks good" flow: a workflow_builder-style worker finished in an
    // EARLIER turn (durable session Idle, history persisted, no pause
    // checkpoint on disk). continue_subagent must resume that same session
    // with the follow-up — seeding the persisted history — instead of
    // erroring "no checkpoint" or spawning a stateless fresh worker.
    use super::ContinueSubagentTool;
    use crate::agent::orchestration::subagent_sessions::{
        self, SubagentSessionSelector, SubagentSessionStore, SubagentSessionUpsert,
    };

    let _ = env_logger::builder().is_test(true).try_init();
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let registry = AgentDefinitionRegistry::global().expect("registry");
    let definition = registry
        .get("task_manager_agent")
        .expect("task_manager_agent definition");
    let workspace = tempfile::TempDir::new().expect("workspace");
    let provider = Arc::new(ScriptedModel::new(vec![(
        "continue-durable-canary",
        "resumed-child-answer",
    )]));

    // Seed the durable session exactly as an earlier async run would have
    // left it, with a selector matching what the resume path will compute.
    let store = SubagentSessionStore::new(workspace.path().to_path_buf());
    let action_root = subagent_sessions::action_root_key(
        crate::security::live_policy::current()
            .map(|policy| policy.action_dir.clone())
            .as_deref(),
    );
    let session = subagent_sessions::upsert_running(
        &store,
        SubagentSessionUpsert {
            selector: SubagentSessionSelector {
                parent_session: "tools-e2e-continue-session".into(),
                parent_thread_id: Some("thread-continue-parent".into()),
                agent_id: "task_manager_agent".into(),
                // Pin the seeded session to the parent's scripted provider —
                // continue_subagent forwards session.model into the resume, so
                // without this the child would resolve the definition's managed
                // tier and dial a REAL provider instead of the test double.
                model: Some("test-model".into()),
                sandbox_mode: format!("{:?}", definition.sandbox_mode),
                action_root,
                task_key: "durable-resume-task".into(),
            },
            display_name: Some("Researcher".into()),
            task_title: "Durable resume task".into(),
            worker_thread_id: None,
            task_id: "sub-earlier-task".into(),
        },
        None,
    )
    .expect("seed durable session");
    subagent_sessions::mark_finished(
        &store,
        &session.subagent_session_id,
        "sub-earlier-task",
        &crate::agent::subagent_host::SubagentRunStatus::Completed,
        vec![
            TranscriptMessage::user("original task from an earlier turn"),
            TranscriptMessage::assistant("earlier proposal result"),
        ],
    )
    .expect("mark idle with history");

    let mut ctx = parent_context(workspace.path(), provider.clone(), vec![]);
    ctx.session_id = "tools-e2e-continue-session".into();
    let mut parent_data = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    parent_data.thread_id = Some("thread-continue-parent".into());
    let parent_run = parent_data.with_parent(ctx.clone()).into_tinyagents(
        RunConfig::new("continue-async-e2e").with_thread("thread-continue-parent"),
    );
    let session_id = session.subagent_session_id.clone();
    let (wrong_agent, result) = (async {
        let tool = ContinueSubagentTool::new();
        let wrong_agent = tool
            .execute_with_live_parent_context(
                json!({
                    "task_id": session_id,
                    "agent_id": "unrelated_agent",
                    "message": "this must not resume the worker"
                }),
                None,
                parent_run.data.child(),
                Some(&parent_run),
            )
            .await?;
        let result = tool
            .execute_with_live_parent_context(
                json!({
                    "task_id": session_id,
                    // Production callers sometimes copy the roster's session
                    // id into both fields; this is the regression case.
                    "agent_id": session.subagent_session_id,
                    "message": "looks good — proceed with continue-durable-canary"
                }),
                None,
                parent_run.data.child(),
                Some(&parent_run),
            )
            .await?;
        Ok::<_, anyhow::Error>((wrong_agent, result))
    })
    .await
    .expect("tool execution");

    assert!(wrong_agent.is_error, "unrelated agent id must be rejected");
    assert!(!result.is_error, "{}", result.output());
    let out = result.output();
    assert!(
        out.contains("[async_subagent_ref]"),
        "resume goes async with a durable ref: {out}"
    );
    assert!(
        out.contains(&session.subagent_session_id),
        "resume keeps the SAME durable session id: {out}"
    );

    // The background resume must reach the provider carrying BOTH the
    // persisted prior history and the new follow-up.
    for _ in 0..600 {
        if provider.saw("continue-durable-canary") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        provider.saw("continue-durable-canary"),
        "follow-up reached the resumed child"
    );
    assert!(
        provider.saw("original task from an earlier turn"),
        "persisted history was replayed into the resumed run"
    );
    crate::agent::orchestration::background_completions::forget_workspace_for_test(
        workspace.path(),
    );
}

#[tokio::test]
async fn continue_subagent_without_checkpoint_or_durable_session_names_the_roster() {
    use super::ContinueSubagentTool;
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let provider = Arc::new(ScriptedModel::new(vec![]));

    let mut ctx = parent_context(workspace.path(), provider, vec![]);
    ctx.session_id = "tools-e2e-continue-missing".into();
    let live = super::live_parent_run_context("continue-missing-e2e", ctx);
    let result = ContinueSubagentTool::new()
        .execute_with_live_parent_context(
            json!({
                "task_id": "sub-does-not-exist",
                "agent_id": "task_manager_agent",
                "message": "hello?"
            }),
            None,
            live.data.child(),
            Some(&live),
        )
        .await
        .expect("tool execution");

    assert!(result.is_error);
    let out = result.output();
    assert!(
        out.contains("no checkpoint and no durable session"),
        "explains both lookups failed: {out}"
    );
    assert!(
        out.contains("[active_subagents]"),
        "points the model at the roster: {out}"
    );
}

#[tokio::test]
async fn spawn_worker_thread_tool_persists_worker_thread_e2e() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let provider = Arc::new(ScriptedModel::new(vec![(
        WORKER_THREAD_CANARY,
        "worker-thread-child-answer",
    )]));

    let live = super::live_parent_run_context(
        "worker-thread-e2e",
        parent_context(workspace.path(), provider.clone(), vec![]),
    );
    let result = SpawnWorkerThreadTool::new()
        .execute_with_live_parent_context(
            json!({
                "agent_id": "task_manager_agent",
                "prompt": format!("Handle long task {WORKER_THREAD_CANARY}"),
                "task_title": "Long delegated task",
                "model": "test-model"
            }),
            None,
            live.data.child(),
            Some(&live),
        )
        .await
        .expect("tool execution");

    assert!(!result.is_error, "{}", result.output());
    assert!(result.output().contains("[worker_thread_ref]"));
    assert!(result.output().contains("\"status\":\"completed\""));
    assert!(provider.saw(WORKER_THREAD_CANARY));

    let threads = conversations::list_threads(
        &crate::core::runtime::DispatchCtx::ambient(),
        workspace.path().to_path_buf(),
    )
    .expect("worker threads");
    let worker = threads
        .iter()
        .find(|thread| thread.labels.contains(&"tasks".to_string()))
        .expect("worker thread was persisted");
    assert_eq!(worker.title, "Long delegated task");

    let messages = conversations::get_messages(
        &crate::core::runtime::DispatchCtx::ambient(),
        workspace.path().to_path_buf(),
        &worker.id,
    )
    .expect("worker messages");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].sender, "user");
    assert!(messages[0].content.contains(WORKER_THREAD_CANARY));
    assert_eq!(messages[1].sender, "agent");
    assert_eq!(messages[1].content, "worker-thread-child-answer");
}

fn parent_context(
    workspace_dir: &Path,
    model: Arc<dyn ChatModel<()>>,
    connected_integrations: Vec<ConnectedIntegration>,
) -> ParentExecutionContext {
    ParentExecutionContext {
        workspace_descriptor: None,
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: ["task_manager_agent".to_string()].into_iter().collect(),
        turn_model_source: crate::agent::tinyagents::TurnModelSource::from_model(model),
        all_tools: Arc::new(Vec::new()),
        all_tool_specs: Arc::new(Vec::new()),
        visible_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: std::collections::HashSet::new(),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        model_name: "test-model".into(),
        temperature: 0.2,
        workspace_dir: workspace_dir.to_path_buf(),
        agent_config: Default::default(),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(None),
        session_id: "tools-e2e-session".into(),
        channel: "test".into(),
        connected_integrations,
        tool_call_format: ToolCallFormat::Native,
        session_key: "tools-e2e".into(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
        host_env: Default::default(),
    }
}

struct ScriptedModel {
    responses: Vec<(&'static str, &'static str)>,
    seen: Mutex<Vec<String>>,
}

impl ScriptedModel {
    fn new(responses: Vec<(&'static str, &'static str)>) -> Self {
        Self {
            responses,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn saw(&self, needle: &str) -> bool {
        self.seen
            .lock()
            .iter()
            .any(|payload| payload.contains(needle))
    }
}

#[async_trait]
impl ChatModel<()> for ScriptedModel {
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
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        let flattened = flatten_messages(&request.messages);
        self.seen.lock().push(flattened.clone());
        for (needle, answer) in &self.responses {
            if flattened.contains(needle) {
                return Ok(ModelResponse::assistant(*answer));
            }
        }
        Err(tinyinference_llm::Error::Model(format!(
            "unexpected model request: {flattened}"
        )))
    }
}

fn flatten_messages(messages: &[Message]) -> String {
    messages
        .iter()
        .map(Message::text)
        .collect::<Vec<_>>()
        .join("\n")
}

/// A model that never stops asking for the same unavailable tool, so the child
/// run can only end by being stopped.
struct LoopingToolModel;

#[async_trait]
impl ChatModel<()> for LoopingToolModel {
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
        let mut response = ModelResponse::assistant("");
        response.message.tool_calls = vec![tinyinference_llm::tool::ToolCall::new(
            "call-loop",
            "task_source_list",
            json!({}),
        )];
        response.finish_reason = Some("tool_calls".to_string());
        Ok(response)
    }
}
