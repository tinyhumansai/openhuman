use super::*;

#[tokio::test]
async fn spawn_subagent_with_same_task_key_resumes_paused_job_through_shared_driver() {
    use crate::agent::orchestration::subagent_sessions::DurableSubagentStatus;
    use crate::agent::orchestration::tools::SpawnSubagentTool;
    use tinyagents_orchestration::subagent::SubAgentJobRegistry;
    use tinyagents_orchestration::subagent::SubAgentJobStatus;

    let _lock = crate::config::TEST_ENV_LOCK.lock().await;
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
    parent.session_id = "continue-paused-e2e".into();
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
    let (live, _core) = core_backed_live_parent(
        workspace.path(),
        parent,
        "continue-paused-e2e",
        "continue-paused-e2e-thread",
    );
    assert!(
        live.data.dispatch_ctx().resolved().is_some(),
        "the parent carrier retains its core context"
    );
    let paused = crate::core::runtime::CoreContext::scope(
        Arc::clone(&_core),
        SpawnSubagentTool::new().execute_with_live_parent_context(
            json!({
                "agent_id": "task_manager_agent",
                "prompt": "Investigate the pause-resume fixture",
                "model": "test-model",
                "task_key": "pause-resume-fixture",
                "blocking": true
            }),
            None,
            live.data.child(),
            crate::agent::subagent_host::detached_subagent_parent(&live, "pause-resume-child")
                .expect("detached parent"),
        ),
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

    let jobs = crate::core::runtime::slot_for::<SubAgentJobRegistry>(&live.data.dispatch_ctx());
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
    let paused_job = jobs
        .list()
        .into_iter()
        .find(|job| {
            job.extensions
                .get("task_id")
                .and_then(serde_json::Value::as_str)
                == Some(task_id.as_str())
        })
        .expect("initial child is registered in shared Jobs");
    let paused_job = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        jobs.wait_owned(
            paused_job.id.as_str(),
            crate::agent::tinyagents::tool_registration::subagent_owner_key(
                workspace.path(),
                "continue-paused-e2e",
                Some("continue-paused-e2e-thread"),
            ),
        ),
    )
    .await
    .expect("child reaches its clarification checkpoint")
    .expect("registered child remains available");
    assert_eq!(paused_job.status, SubAgentJobStatus::AwaitingInput);
    assert_eq!(
        provider.calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the initial child must reach and persist its clarification before resume"
    );

    let before_session = super::driver_sessions_for_parent(
        &live.data.dispatch_ctx(),
        workspace.path(),
        "continue-paused-e2e",
        Some("continue-paused-e2e-thread"),
    )
    .await
    .expect("initial session persisted")
    .into_iter()
    .find(|session| session.current_task_id.as_deref() == Some(task_id.as_str()))
    .expect("initial durable session points at the paused child");
    assert_eq!(before_session.status, DurableSubagentStatus::AwaitingUser);
    let original_lifecycle_key = before_session
        .lifecycle_key
        .clone()
        .expect("paused session retains its original TinyAgents task key");
    assert_eq!(original_lifecycle_key.task_id, task_id);
    assert_eq!(
        original_lifecycle_key.thread_id.as_deref(),
        Some("continue-paused-e2e-thread")
    );
    let before_job_count = jobs.list().len();

    let driver_documents =
        crate::core::runtime::CoreContext::sync_scope(Arc::clone(&_core), || {
            let opened = crate::storage::local::open("subagent_sessions", workspace.path())
                .map_err(|error| format!("open driver session documents: {error:#}"))?;
            let scoped = opened
                .scoped()
                .map_err(|error| format!("scope driver session documents: {error:#}"))?;
            Ok::<_, String>(Arc::clone(scoped.documents()))
        })
        .expect("open scoped driver session documents");

    // A durable host row and the checkpoint must agree on the full owner key.
    // Corrupt only its root run identity and drive the real spawn path: resume
    // must refuse before mutating the paused row or admitting another child.
    let definitions = AgentDefinitionRegistry::builtins_only();
    let definition = definitions
        .get("task_manager_agent")
        .expect("task manager definition");
    let effective_action_root = live
        .data
        .workspace
        .as_ref()
        .map(|workspace| workspace.root.clone())
        .or_else(|| {
            crate::security::live_policy::effective_in(&live.data.dispatch_ctx())
                .map(|policy| policy.action_dir.clone())
        });
    let selector = crate::agent::orchestration::subagent_sessions::SubagentSessionSelector {
        parent_session: "continue-paused-e2e".into(),
        parent_thread_id: Some("continue-paused-e2e-thread".into()),
        agent_id: "task_manager_agent".into(),
        model: Some("test-model".into()),
        sandbox_mode: format!("{:?}", definition.sandbox_mode),
        action_root: crate::agent::orchestration::subagent_sessions::action_root_key(
            effective_action_root.as_deref(),
        ),
        task_key: "pause-resume-fixture".into(),
    };
    let driver_documents_for_read = Arc::clone(&driver_documents);
    let session_id = before_session.subagent_session_id.clone();
    let versioned = crate::storage::block_on(async move {
        driver_documents_for_read
            .get("subagent_sessions", &session_id)
            .await
    })
    .expect("read paused driver document")
    .expect("paused driver document exists");
    let precondition = versioned.unchanged();
    let mut persisted: tinyagents_orchestration::subagent::DurableSubagentSession =
        serde_json::from_value(versioned.doc).expect("decode paused driver document");
    persisted
        .lifecycle_key
        .as_mut()
        .expect("paused driver session retains lifecycle key")
        .root_run_id
        .push_str("-mismatch");
    let updated_at = chrono::DateTime::parse_from_rfc3339(&persisted.updated_at)
        .expect("persisted update timestamp")
        .with_timezone(&chrono::Utc)
        + chrono::Duration::milliseconds(1);
    persisted.updated_at = updated_at.to_rfc3339();
    let saved_key = persisted
        .lifecycle_key
        .clone()
        .expect("corrupted paused driver session retains lifecycle key");
    let driver_documents_for_write = Arc::clone(&driver_documents);
    let session_id = before_session.subagent_session_id.clone();
    let persisted_doc = serde_json::to_value(&persisted).expect("encode corrupted driver session");
    crate::storage::block_on(async move {
        driver_documents_for_write
            .put(
                "subagent_sessions",
                &session_id,
                persisted_doc,
                precondition,
            )
            .await
    })
    .expect("corrupt only the persisted driver lifecycle owner key");
    assert_eq!(persisted.parent_session, selector.parent_session);
    assert_eq!(
        persisted.parent_thread_id.as_deref(),
        selector.parent_thread_id.as_deref()
    );
    assert_eq!(persisted.agent_id, selector.agent_id);
    assert_eq!(persisted.task_key, selector.task_key);
    let reusable_session_id = before_session.subagent_session_id.clone();
    assert_ne!(saved_key, original_lifecycle_key);
    assert_eq!(
        reusable_session_id, before_session.subagent_session_id,
        "the production reuse selector sees the corrupted paused row"
    );
    let refused_resume = crate::core::runtime::CoreContext::scope(
        Arc::clone(&_core),
        SpawnSubagentTool::new().execute_with_live_parent_context(
            json!({
                "agent_id": "task_manager_agent",
                "prompt": "This must not resume against a different lifecycle key.",
                "model": "test-model",
                "task_key": "pause-resume-fixture"
            }),
            None,
            live.data.child(),
            crate::agent::subagent_host::detached_subagent_parent(&live, "pause-resume-child")
                .expect("detached parent"),
        ),
    )
    .await
    .expect("mismatched checkpoint refusal is returned as a tool error");
    assert!(refused_resume.is_error);
    assert!(
        refused_resume
            .output()
            .contains("paused checkpoint does not match the reusable session"),
        "refusal identifies the mismatched checkpoint: {}",
        refused_resume.output()
    );
    assert_eq!(
        jobs.list().len(),
        before_job_count,
        "refusal admits no child"
    );
    assert_eq!(
        provider.calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "refusal invokes no child model"
    );
    let refused_session = super::driver_sessions_for_parent(
        &live.data.dispatch_ctx(),
        workspace.path(),
        "continue-paused-e2e",
        Some("continue-paused-e2e-thread"),
    )
    .await
    .expect("refused row remains persisted")
    .into_iter()
    .find(|session| session.subagent_session_id == before_session.subagent_session_id)
    .expect("paused row remains present");
    assert_eq!(refused_session.status, DurableSubagentStatus::AwaitingUser);
    assert_eq!(
        refused_session.lifecycle_key.as_ref(),
        Some(&saved_key),
        "refusal leaves the mismatched paused lifecycle key untouched"
    );
    let driver_documents_for_read = Arc::clone(&driver_documents);
    let session_id = before_session.subagent_session_id.clone();
    let versioned = crate::storage::block_on(async move {
        driver_documents_for_read
            .get("subagent_sessions", &session_id)
            .await
    })
    .expect("read mismatched driver session")
    .expect("mismatched driver session exists");
    let precondition = versioned.unchanged();
    let mut restored: tinyagents_orchestration::subagent::DurableSubagentSession =
        serde_json::from_value(versioned.doc).expect("decode mismatched driver session");
    restored.lifecycle_key = Some(original_lifecycle_key.clone());
    let updated_at = chrono::DateTime::parse_from_rfc3339(&restored.updated_at)
        .expect("corrupted update timestamp")
        .with_timezone(&chrono::Utc)
        + chrono::Duration::milliseconds(1);
    restored.updated_at = updated_at.to_rfc3339();
    let driver_documents_for_write = Arc::clone(&driver_documents);
    let session_id = before_session.subagent_session_id.clone();
    let restored_doc = serde_json::to_value(&restored).expect("encode restored driver session");
    crate::storage::block_on(async move {
        driver_documents_for_write
            .put("subagent_sessions", &session_id, restored_doc, precondition)
            .await
    })
    .expect("restore persisted lifecycle key for valid continuation");
    assert_eq!(
        restored.lifecycle_key.as_ref(),
        Some(&original_lifecycle_key),
        "valid continuation starts only after the original lifecycle key is restored"
    );

    let resumed = crate::core::runtime::CoreContext::scope(
        Arc::clone(&_core),
        SpawnSubagentTool::new().execute_with_live_parent_context(
            json!({
                "agent_id": "task_manager_agent",
                "prompt": "Inspect the README and continue.",
                "model": "test-model",
                "task_key": "pause-resume-fixture"
            }),
            None,
            live.data.child(),
            crate::agent::subagent_host::detached_subagent_parent(&live, "pause-resume-child")
                .expect("detached parent"),
        ),
    )
    .await
    .expect("continue");
    assert!(!resumed.is_error, "{}", resumed.output());
    let resumed_ref = resumed
        .output()
        .split("[async_subagent_ref]\n")
        .nth(1)
        .and_then(|value| value.split("\n[/async_subagent_ref]").next())
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .unwrap_or_else(|| {
            panic!(
                "resumed result carries task reference: {}",
                resumed.output()
            )
        });
    assert_eq!(
        resumed_ref["task_id"].as_str(),
        Some(task_id.as_str()),
        "resuming a checkpoint must keep its registered task identity"
    );
    assert_eq!(
        jobs.list().len(),
        before_job_count,
        "resume must not spawn a second child"
    );
    assert_eq!(
        resumed_ref["subagent_session_id"].as_str(),
        Some(before_session.subagent_session_id.as_str()),
        "resume must keep the durable session identity"
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match progress_rx.recv().await {
                Some(crate::progress::AgentProgress::SubagentCompleted {
                    task_id: event_task_id,
                    ..
                }) if event_task_id == task_id => break,
                Some(_) => {}
                None => panic!("parent progress stream closed before child completion"),
            }
        }
    })
    .await
    .expect("resume completion projection follows durable settlement");
    assert!(provider.saw("Inspect the README and continue."));
    assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    let requests = provider.seen.lock().clone();
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
    let completed_job = jobs
        .get(paused_job.id.as_str())
        .expect("continuation reuses the same registered job");
    assert_eq!(completed_job.agent, "task_manager_agent");
    assert_eq!(completed_job.status, SubAgentJobStatus::Completed);
    assert_eq!(
        completed_job.output.as_deref(),
        Some("paused-child-resumed-answer")
    );
    let completed_session = super::driver_sessions_for_parent(
        &live.data.dispatch_ctx(),
        workspace.path(),
        "continue-paused-e2e",
        Some("continue-paused-e2e-thread"),
    )
    .await
    .expect("completed session list")
    .into_iter()
    .find(|session| session.current_task_id.as_deref() == Some(task_id.as_str()))
    .expect("completed row still owns the original task");
    assert_eq!(
        completed_session.lifecycle_key.as_ref(),
        Some(&original_lifecycle_key),
        "resume preserves the original lifecycle identity"
    );
}

#[tokio::test]
async fn spawn_subagent_reuses_idle_task_key_with_the_saved_history() {
    use crate::agent::orchestration::subagent_sessions::open_for_context;
    use crate::agent::orchestration::tools::SpawnSubagentTool;
    use tinyagents_orchestration::subagent::SubAgentJobRegistry;
    use tinyagents_orchestration::subagent::{DurableSubagentStatus, SubAgentJobStatus};

    let _lock = crate::config::TEST_ENV_LOCK.lock().await;
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::TempDir::new().expect("workspace");
    let provider = Arc::new(TwoTurnModel::default());
    let mut parent = parent_context(workspace.path(), provider.clone(), vec![]);
    parent.session_id = "idle-reuse-e2e".into();
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(64);
    parent.on_progress = Some(progress_tx);
    let (mut live, _core) = core_backed_live_parent(
        workspace.path(),
        parent,
        "idle-reuse-e2e",
        "idle-reuse-e2e-thread",
    );
    let workspace_descriptor = tinytools::WorkspaceDescriptor::new(workspace.path())
        .with_trusted_root(workspace.path().join("shared-worktree"))
        .with_policy_id("idle-roster-policy");
    live.workspace = Some(workspace_descriptor.clone());
    assert!(live.data.dispatch_ctx().resolved().is_some());
    let tool_context = tinyagents_harness::tool::ToolExecutionContext::from_run_context(
        &live,
        tinyagents_harness::ids::CallId::new("idle-roster-spawn"),
    );
    let tool = SpawnSubagentTool::new();
    let first = tool
        .execute_with_live_parent_context(
            json!({
                "agent_id": "task_manager_agent",
                "prompt": "idle-history-first-prompt",
                "model": "test-model",
                "task_key": "idle-history-fixture"
            }),
            Some(&tool_context),
            live.data.child(),
            crate::agent::subagent_host::detached_subagent_parent(&live, "idle-reuse-child")
                .expect("detached parent"),
        )
        .await
        .expect("first spawn");
    assert!(!first.is_error, "{}", first.output());
    wait_completed(&mut progress_rx).await;

    let sessions = open_for_context(
        &live.data.dispatch_ctx(),
        workspace.path().to_path_buf(),
        "idle-reuse-e2e",
        Some("idle-reuse-e2e-thread".into()),
        None,
    )
    .expect("open driver sessions");
    let first_session = sessions
        .list_for_parent("idle-reuse-e2e", Some("idle-reuse-e2e-thread"))
        .await
        .expect("first session list")
        .into_iter()
        .find(|session| session.task_key == "idle-history-fixture")
        .expect("first durable session");
    assert_eq!(first_session.status, DurableSubagentStatus::Idle);
    let roster = crate::agent::orchestration::running_subagents::active_subagents_context_block(
        &live.data.dispatch_ctx(),
        "idle-reuse-e2e",
        workspace.path(),
        Some(&workspace_descriptor),
    )
    .await
    .expect("driver-backed child appears in the parent roster");
    assert!(roster.contains(&first_session.subagent_session_id));
    assert!(roster.contains(first_session.current_task_id.as_deref().unwrap()));
    let original_session_id = first_session.subagent_session_id.clone();
    let first_task_id = first_session
        .current_task_id
        .clone()
        .expect("first task id");
    let cached_owner = crate::agent::orchestration::completion_owners::driver_task_owner(
        &live.data.dispatch_ctx(),
        &first_task_id,
    )
    .expect("task owner scope is retained for the roster");
    assert_eq!(cached_owner.workspace, workspace_descriptor);
    let owner = crate::agent::tinyagents::tool_registration::subagent_owner_key(
        workspace.path(),
        "idle-reuse-e2e",
        Some("idle-reuse-e2e-thread"),
    );
    let scoped_jobs =
        crate::core::runtime::slot_for::<SubAgentJobRegistry>(&live.data.dispatch_ctx())
            .list_for_parent_scope(
                &owner,
                &tinyagents_orchestration::subagent::ParentJobScope::new(
                    workspace_descriptor.clone(),
                    "idle-reuse-e2e-thread",
                ),
            )
            .expect("custom workspace/thread job scope");
    assert!(scoped_jobs.iter().any(|job| {
        job.extensions
            .get("task_id")
            .and_then(serde_json::Value::as_str)
            == Some(first_task_id.as_str())
    }));
    let first_job =
        crate::core::runtime::slot_for::<SubAgentJobRegistry>(&live.data.dispatch_ctx())
            .list()
            .into_iter()
            .find(|job| {
                job.extensions
                    .get("task_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(first_task_id.as_str())
            })
            .expect("first child is registered");
    assert_eq!(first_job.status, SubAgentJobStatus::Completed);

    let second = tool
        .execute_with_live_parent_context(
            json!({
                "agent_id": "task_manager_agent",
                "prompt": "idle-history-follow-up-prompt",
                "model": "test-model",
                "task_key": "idle-history-fixture"
            }),
            Some(&tool_context),
            live.data.child(),
            crate::agent::subagent_host::detached_subagent_parent(&live, "idle-reuse-child")
                .expect("detached parent"),
        )
        .await
        .expect("follow-up spawn");
    assert!(!second.is_error, "{}", second.output());
    wait_completed(&mut progress_rx).await;
    let second_session = sessions
        .list_for_parent("idle-reuse-e2e", Some("idle-reuse-e2e-thread"))
        .await
        .expect("follow-up session list")
        .into_iter()
        .find(|session| session.subagent_session_id == original_session_id)
        .expect("same durable child session is reused");
    assert_eq!(second_session.status, DurableSubagentStatus::Idle);
    assert_ne!(
        second_session.current_task_id.as_deref(),
        Some(first_task_id.as_str())
    );
    assert!(provider.saw("idle-history-first-prompt"));
    assert!(provider.saw("idle-child-first-result"));
    assert!(provider.saw("idle-history-follow-up-prompt"));
    let requests = provider.seen.lock();
    assert_eq!(requests.len(), 2, "both spawns use the injected model");
    assert_eq!(
        requests[1].matches("idle-history-follow-up-prompt").count(),
        1
    );
}

async fn wait_completed(
    progress: &mut tokio::sync::mpsc::Receiver<crate::progress::AgentProgress>,
) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match progress.recv().await {
                Some(crate::progress::AgentProgress::SubagentCompleted { agent_id, .. })
                    if agent_id == "task_manager_agent" =>
                {
                    break;
                }
                Some(crate::progress::AgentProgress::SubagentFailed { error, .. }) => {
                    panic!("child failed before completion: {error}")
                }
                Some(_) => {}
                None => panic!("progress stream closed before child completion"),
            }
        }
    })
    .await
    .expect("completion is projected after durable settlement");
}

#[derive(Default)]
struct TwoTurnModel {
    calls: std::sync::atomic::AtomicUsize,
    seen: Mutex<Vec<String>>,
}

impl TwoTurnModel {
    fn saw(&self, needle: &str) -> bool {
        self.seen
            .lock()
            .iter()
            .any(|payload| payload.contains(needle))
    }
}

#[async_trait::async_trait]
impl tinyinference_llm::model::ChatModel<()> for TwoTurnModel {
    fn profile(&self) -> Option<&tinyinference_llm::model::ModelProfile> {
        static PROFILE: std::sync::OnceLock<tinyinference_llm::model::ModelProfile> =
            std::sync::OnceLock::new();
        Some(PROFILE.get_or_init(|| {
            let mut profile = tinyinference_llm::model::ModelProfile::default();
            profile.tool_calling = true;
            profile.parallel_tool_calls = true;
            profile
        }))
    }

    async fn invoke(
        &self,
        _state: &(),
        request: tinyinference_llm::model::ModelRequest,
    ) -> tinyinference_llm::Result<tinyinference_llm::model::ModelResponse> {
        let seen = request
            .messages
            .iter()
            .map(tinyinference_llm::message::Message::text)
            .collect::<Vec<_>>()
            .join("\n");
        self.seen.lock().push(seen);
        let answer = match self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
            0 => "idle-child-first-result",
            1 => "idle-child-follow-up-result",
            call => {
                return Err(tinyinference_llm::Error::Model(format!(
                    "unexpected call {call}"
                )));
            }
        };
        Ok(tinyinference_llm::model::ModelResponse::assistant(answer))
    }
}

pub(super) fn core_backed_live_parent(
    workspace: &std::path::Path,
    parent: crate::agent::subagent_host::ParentExecutionContext,
    session_id: &str,
    thread_id: &str,
) -> (
    tinyagents_harness::context::RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    Arc<crate::core::runtime::CoreContext>,
) {
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.to_path_buf();
    core_backed_live_parent_with_config(workspace, parent, session_id, thread_id, config)
}

pub(super) fn core_backed_live_parent_with_config(
    workspace: &std::path::Path,
    parent: crate::agent::subagent_host::ParentExecutionContext,
    session_id: &str,
    thread_id: &str,
    mut config: crate::config::Config,
) -> (
    tinyagents_harness::context::RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    Arc<crate::core::runtime::CoreContext>,
) {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::tools::toolpacks::ToolGroups;

    let mut definition = AgentDefinitionRegistry::builtins_only()
        .get("task_manager_agent")
        .expect("task manager definition")
        .clone();
    definition.model = tinyagents_definition::ModelSpec::Inherit;
    let definitions =
        Arc::new(AgentDefinitionRegistry::builtins_only().with_definitions([definition]));
    config.workspace_dir = workspace.to_path_buf();
    config.components.juice = false;
    let core = CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(config, DomainSet::full(), ToolGroups::none()).definitions(definitions),
    );
    let mut parent = parent;
    parent.workspace_dir = workspace.to_path_buf();
    parent.session_id = session_id.to_owned();
    let mut host_context = crate::agent::tinyagents::host::OpenHumanRunContext::new()
        .with_core(Some(Arc::clone(&core)))
        .with_parent(parent);
    host_context.thread_id = Some(thread_id.to_owned());
    (
        tinyagents_harness::context::RunContext::new(
            tinyagents_harness::context::RunConfig::new(session_id).with_thread(thread_id),
            host_context,
        ),
        core,
    )
}
