use super::{
    ArchetypeDelegationTool, SkillDelegationTool, SpawnSubagentTool, SpawnWorkerThreadTool,
};
use crate::openhuman::agent::context::prompt::{ConnectedIntegration, ToolCallFormat};
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::{with_parent_context, ParentExecutionContext};
use crate::openhuman::agent::messages::ChatMessage;
use crate::openhuman::memory::conversations;
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts};
use crate::openhuman::tools::Tool;
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use tinyinference::message::Message;
use tinyinference::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};

const SPAWN_SUBAGENT_CANARY: &str = "tool-e2e-spawn-subagent-canary";
const ARCHETYPE_DELEGATION_CANARY: &str = "tool-e2e-archetype-delegation-canary";
const SKILL_DELEGATION_CANARY: &str = "tool-e2e-skill-delegation-canary";
const WORKER_THREAD_CANARY: &str = "tool-e2e-worker-thread-canary";

#[tokio::test]
async fn spawn_subagent_tool_runs_child_agent_e2e() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let provider = Arc::new(ScriptedModel::new(vec![(
        SPAWN_SUBAGENT_CANARY,
        "spawn-subagent-child-answer",
    )]));

    let result = with_parent_context(
        parent_context(workspace.path(), provider.clone(), vec![]),
        async {
            SpawnSubagentTool::new()
                .execute(json!({
                    "agent_id": "researcher",
                    "prompt": format!("Investigate {SPAWN_SUBAGENT_CANARY}"),
                    "context": "parent supplied context",
                    "model": "test-model",
                    "blocking": true
                }))
                .await
        },
    )
    .await
    .expect("tool execution");

    assert!(!result.is_error, "{}", result.output());
    assert_eq!(result.output(), "spawn-subagent-child-answer");
    assert!(provider.saw(SPAWN_SUBAGENT_CANARY));
    assert!(provider.saw("parent supplied context"));
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
        tool_name: "delegate_researcher".to_string(),
        agent_id: "researcher".to_string(),
        tool_description: "Delegate research work.".to_string(),
    };

    let result = with_parent_context(
        parent_context(workspace.path(), provider.clone(), vec![]),
        async {
            tool.execute(json!({
                "prompt": format!("Research {ARCHETYPE_DELEGATION_CANARY}"),
                "model": "test-model"
            }))
            .await
        },
    )
    .await
    .expect("tool execution");

    assert!(!result.is_error, "{}", result.output());
    assert_eq!(result.output(), "archetype-delegation-child-answer");
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
        tool_name: "delegate_researcher".to_string(),
        agent_id: "researcher".to_string(),
        tool_description: "Delegate research work.".to_string(),
    };

    let mut ctx = parent_context(workspace.path(), provider.clone(), vec![]);
    ctx.session_id = "tools-e2e-async-session".into();
    let result = with_parent_context(ctx, async {
        crate::openhuman::agent::tinyagents::thread_context::with_thread_id(
            "thread-async-parent",
            async {
                tool.execute(json!({
                    "prompt": format!("Research {ARCHETYPE_DELEGATION_CANARY} in the background"),
                    "model": "test-model"
                }))
                .await
            },
        )
        .await
    })
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
    use crate::openhuman::agent::orchestration::subagent_sessions::{
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
    assert_eq!(session.agent_id, "researcher");
    assert!(
        session
            .latest_history
            .as_ref()
            .is_some_and(|h| !h.is_empty()),
        "resumable history persisted"
    );
    // Completion queued for delivery back into the parent chat as a new turn.
    assert!(
        crate::openhuman::agent::orchestration::background_completions::has_pending(
            "tools-e2e-async-session"
        ),
        "finished result queued for background delivery"
    );
    let _ = crate::openhuman::agent::orchestration::background_completions::take_pending(
        "tools-e2e-async-session",
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
    use crate::openhuman::agent::orchestration::subagent_sessions::{
        self, SubagentSessionSelector, SubagentSessionStore, SubagentSessionUpsert,
    };

    let _ = env_logger::builder().is_test(true).try_init();
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let registry = AgentDefinitionRegistry::global().expect("registry");
    let definition = registry.get("researcher").expect("researcher definition");
    let workspace = tempfile::TempDir::new().expect("workspace");
    let provider = Arc::new(ScriptedModel::new(vec![(
        "continue-durable-canary",
        "resumed-child-answer",
    )]));

    // Seed the durable session exactly as an earlier async run would have
    // left it, with a selector matching what the resume path will compute.
    let store = SubagentSessionStore::new(workspace.path().to_path_buf());
    let action_root = subagent_sessions::action_root_key(
        crate::openhuman::security::live_policy::current()
            .map(|policy| policy.action_dir.clone())
            .as_deref(),
    );
    let session = subagent_sessions::upsert_running(
        &store,
        SubagentSessionUpsert {
            selector: SubagentSessionSelector {
                parent_session: "tools-e2e-continue-session".into(),
                parent_thread_id: Some("thread-continue-parent".into()),
                agent_id: "researcher".into(),
                toolkit: None,
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
        &crate::openhuman::agent::harness::subagent_runner::SubagentRunStatus::Completed,
        vec![
            ChatMessage::user("original task from an earlier turn"),
            ChatMessage::assistant("earlier proposal result"),
        ],
    )
    .expect("mark idle with history");

    let mut ctx = parent_context(workspace.path(), provider.clone(), vec![]);
    ctx.session_id = "tools-e2e-continue-session".into();
    let session_id = session.subagent_session_id.clone();
    let result = with_parent_context(ctx, async {
        crate::openhuman::agent::tinyagents::thread_context::with_thread_id(
            "thread-continue-parent",
            async {
                ContinueSubagentTool::new()
                    .execute(json!({
                        "task_id": session_id,
                        "agent_id": "researcher",
                        "message": "looks good — proceed with continue-durable-canary"
                    }))
                    .await
            },
        )
        .await
    })
    .await
    .expect("tool execution");

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
    let _ = crate::openhuman::agent::orchestration::background_completions::take_pending(
        "tools-e2e-continue-session",
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
    let result = with_parent_context(ctx, async {
        ContinueSubagentTool::new()
            .execute(json!({
                "task_id": "sub-does-not-exist",
                "agent_id": "researcher",
                "message": "hello?"
            }))
            .await
    })
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
async fn skill_delegation_tool_runs_integrations_agent_e2e() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let provider = Arc::new(ScriptedModel::new(vec![(
        SKILL_DELEGATION_CANARY,
        "skill-delegation-child-answer",
    )]));
    let tool = SkillDelegationTool::for_connected(vec![(
        "gmail".to_string(),
        "Email access.".to_string(),
    )])
    .expect("delegation tool");

    let result = with_parent_context(
        parent_context(
            workspace.path(),
            provider.clone(),
            vec![ConnectedIntegration {
                toolkit: "gmail".to_string(),
                description: "Email access.".to_string(),
                tools: Vec::new(),
                gated_tools: Vec::new(),
                connected: true,
                connections: Vec::new(),
                non_active_status: None,
            }],
        ),
        async {
            tool.execute(json!({
                "toolkit": "gmail",
                "prompt": format!("Summarize inbox state for {SKILL_DELEGATION_CANARY}"),
                "model": "test-model"
            }))
            .await
        },
    )
    .await
    .expect("tool execution");

    assert!(!result.is_error, "{}", result.output());
    assert_eq!(result.output(), "skill-delegation-child-answer");
    assert!(provider.saw(SKILL_DELEGATION_CANARY));
    assert!(provider.saw("gmail"));
}

#[tokio::test]
async fn spawn_worker_thread_tool_persists_worker_thread_e2e() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let provider = Arc::new(ScriptedModel::new(vec![(
        WORKER_THREAD_CANARY,
        "worker-thread-child-answer",
    )]));

    let result = with_parent_context(
        parent_context(workspace.path(), provider.clone(), vec![]),
        async {
            SpawnWorkerThreadTool::new()
                .execute(json!({
                    "agent_id": "researcher",
                    "prompt": format!("Handle long task {WORKER_THREAD_CANARY}"),
                    "task_title": "Long delegated task",
                    "model": "test-model"
                }))
                .await
        },
    )
    .await
    .expect("tool execution");

    assert!(!result.is_error, "{}", result.output());
    assert!(result.output().contains("[worker_thread_ref]"));
    assert!(result.output().contains("\"status\":\"completed\""));
    assert!(provider.saw(WORKER_THREAD_CANARY));

    let threads =
        conversations::list_threads(workspace.path().to_path_buf()).expect("worker threads");
    let worker = threads
        .iter()
        .find(|thread| thread.labels.contains(&"tasks".to_string()))
        .expect("worker thread was persisted");
    assert_eq!(worker.title, "Long delegated task");

    let messages = conversations::get_messages(workspace.path().to_path_buf(), &worker.id)
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
        allowed_subagent_ids: ["researcher".to_string(), "integrations_agent".to_string()]
            .into_iter()
            .collect(),
        turn_model_source: crate::openhuman::agent::tinyagents::TurnModelSource::from_model(model),
        all_tools: Arc::new(Vec::new()),
        all_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: std::collections::HashSet::new(),
        subagent_tool_ceiling_names: std::collections::HashSet::new(),
        model_name: "test-model".into(),
        temperature: 0.2,
        workspace_dir: workspace_dir.to_path_buf(),
        memory: Arc::new(NoopMemory),
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
    ) -> tinyinference::Result<ModelResponse> {
        let flattened = flatten_messages(&request.messages);
        self.seen.lock().push(flattened.clone());
        for (needle, answer) in &self.responses {
            if flattened.contains(needle) {
                return Ok(ModelResponse::assistant(*answer));
            }
        }
        Err(tinyinference::Error::Model(format!(
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

struct NoopMemory;

#[async_trait]
impl Memory for NoopMemory {
    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        _value: &str,
        _category: MemoryCategory,
        _source: Option<&str>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn recall(
        &self,
        _query: &str,
        _limit: usize,
        _opts: RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(None)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _source: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(Vec::new())
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
        Ok(false)
    }

    async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(0)
    }

    async fn health_check(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "noop"
    }
}
