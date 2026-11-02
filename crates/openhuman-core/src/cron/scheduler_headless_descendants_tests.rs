use super::*;
use async_trait::async_trait;
use tinyinference_llm::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};

const CHILD_PROMPT: &str = "headless-cron-child-result-marker";

struct ChildInvocationDrop(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for ChildInvocationDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

struct CronChildModel {
    spawn_calls: std::sync::atomic::AtomicUsize,
    child_calls: std::sync::atomic::AtomicUsize,
    child_invocation_dropped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    accepted_followups: std::sync::atomic::AtomicUsize,
    accepted_followup: tokio::sync::Notify,
    child_attempts_dangerous_tool: bool,
    child_dangerous_tool_schema_seen: std::sync::atomic::AtomicBool,
    child_started: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    child_release: tokio::sync::Notify,
    child_released: std::sync::atomic::AtomicBool,
}

impl CronChildModel {
    /// Release every current and future child invocation. A bare
    /// `Notify::notify_one` stores a single permit, so a second child model call
    /// (a retry or replayed turn) would park forever and hang the drain.
    fn release_child(&self) {
        self.child_released
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.child_release.notify_waiters();
    }

    async fn wait_for_release(&self) {
        loop {
            let notified = self.child_release.notified();
            if self
                .child_released
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return;
            }
            notified.await;
        }
    }
}

#[cfg(feature = "storage-sqlite")]
fn scheduled_agent_fixture(
    timeout_secs: u64,
) -> (
    TempDir,
    crate::storage::test_support::SqliteBackend,
    crate::storage::test_support::InstalledBackendGuard,
    Config,
    std::sync::Arc<crate::core::runtime::CoreContext>,
    crate::core::runtime::DispatchCtx,
    tinyflows_bus::schedule::types::CronJob,
) {
    let temp = TempDir::new().expect("cron workspace");
    let mut config = Config::default();
    config.workspace_dir = temp.path().join("workspace");
    config.action_dir = config.workspace_dir.clone();
    let storage = crate::storage::test_support::sqlite_backend().expect("SQLite test backend");
    config.storage.url = Some(storage.url.clone());
    let storage_guard =
        crate::storage::test_support::install_for_test(&storage.url, Arc::clone(&storage.backend));
    config.default_model = Some("test-model".into());
    config.chat_provider = Some("omlx:test-model".into());
    config.agent.agent_timeout_secs = timeout_secs;
    config.reliability.scheduler_retries = 0;
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace");

    let definitions =
        Arc::new(crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only());
    let core = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config.clone(),
    )
    .derive_with(
        crate::core::runtime::ContextOverlay::new(
            config.clone(),
            crate::core::runtime::DomainSet::full(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .definitions(definitions),
    );
    let dispatch = crate::core::runtime::DispatchCtx::new(std::sync::Arc::clone(&core));
    let mut spec = tinyflows_bus::schedule::AgentJobSpec::new(
        tinyflows_bus::schedule::types::Schedule::Every {
            every_ms: 3_600_000,
        },
        "Delegate this scheduled task to its worker.",
    );
    spec.name = Some("headless child test".into());
    spec.session_target = SessionTarget::Isolated;
    spec.model = Some("test-model".into());
    spec.enabled = true;
    let job =
        crate::cron::add_agent_job_from_spec(&config, spec).expect("persist scheduled agent job");
    crate::cron::store::force_next_run_for_test(
        &config,
        &job.id,
        chrono::Utc::now() - chrono::Duration::seconds(1),
    )
    .expect("make the job due");
    (temp, storage, storage_guard, config, core, dispatch, job)
}

/// Read a job's persisted runs without parking the test's single-threaded
/// runtime. `cron::list_runs` blocks its caller on the storage runtime; called
/// directly from a `#[tokio::test]` it freezes the only thread that drives the
/// scheduler's own callbacks and timers, so no watchdog can fire and the
/// runner's 5-minute kill is the first thing to notice. A plain thread keeps
/// the runtime turning and the deadline real.
async fn list_runs_off_runtime(
    config: &Config,
    job_id: &str,
) -> anyhow::Result<Vec<tinyflows_bus::schedule::types::CronRun>> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let config = config.clone();
    let job_id = job_id.to_string();
    std::thread::spawn(move || {
        let _ = tx.send(crate::cron::list_runs(&config, &job_id, 10));
    });
    match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
        Ok(Ok(runs)) => runs,
        Ok(Err(_)) => Err(anyhow::anyhow!("list_runs thread exited without a result")),
        Err(_) => Err(anyhow::anyhow!("list_runs did not return within 30s")),
    }
}

async fn dispatch_scheduled_agent(
    core: std::sync::Arc<crate::core::runtime::CoreContext>,
    dispatch: crate::core::runtime::DispatchCtx,
    config: Config,
) {
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    let mut dispatcher = JobDispatcher::new(&dispatch, &config, &security);
    crate::core::runtime::CoreContext::scope(
        core,
        Box::pin(async move {
            let dispatched = dispatcher.tick(&config).await.expect("scheduler tick");
            assert_eq!(dispatched, 1, "the persisted due job is dispatched");
            dispatcher.drain().await;
        }),
    )
    .await;
}

#[async_trait]
impl ChatModel<()> for CronChildModel {
    fn profile(&self) -> Option<&ModelProfile> {
        static PROFILE: std::sync::OnceLock<ModelProfile> = std::sync::OnceLock::new();
        Some(PROFILE.get_or_init(|| ModelProfile {
            tool_calling: true,
            parallel_tool_calls: true,
            ..ModelProfile::default()
        }))
    }

    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        if request.messages.iter().any(|message| {
            matches!(message, tinyinference_llm::message::Message::User(_))
                && message.text().contains(CHILD_PROMPT)
        }) {
            let call = self
                .child_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if let Some(started) = self.child_started.lock().unwrap().take() {
                let _ = started.send(());
            }
            if self.child_attempts_dangerous_tool {
                if call > 0 {
                    return Ok(ModelResponse::assistant(
                        "headless-cron-child-returned-after-policy-denial",
                    ));
                }
                self.child_dangerous_tool_schema_seen.store(
                    request
                        .tools
                        .iter()
                        .any(|tool| tool.name == "delete_artifact"),
                    std::sync::atomic::Ordering::SeqCst,
                );
                let mut response = ModelResponse::assistant("");
                response.message.tool_calls = vec![tinyinference_llm::tool::ToolCall::new(
                    "headless-cron-forbidden-write",
                    "delete_artifact",
                    serde_json::json!({"artifact_id": "missing-headless-cron-artifact"}),
                )];
                response.finish_reason = Some("tool_calls".into());
                return Ok(response);
            }
            if let Some(started) = self.child_started.lock().unwrap().take() {
                let _ = started.send(());
            }
            let _drop = ChildInvocationDrop(std::sync::Arc::clone(&self.child_invocation_dropped));
            self.wait_for_release().await;
            return Ok(ModelResponse::assistant("headless-cron-child-result"));
        }

        if request.messages.iter().any(|message| {
            matches!(message, tinyinference_llm::message::Message::Tool(_))
                && message
                    .text()
                    .contains("Accepted async sub-agent `task_manager_agent`")
        }) {
            self.accepted_followups
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.accepted_followup.notify_one();
            return Ok(ModelResponse::assistant("headless-cron-root-result"));
        }

        if self
            .spawn_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            > 0
        {
            return Ok(ModelResponse::assistant("headless-cron-root-result"));
        }
        let mut response = ModelResponse::assistant("");
        response.message.tool_calls = vec![tinyinference_llm::tool::ToolCall::new(
            "headless-cron-spawn",
            "spawn_subagent",
            serde_json::json!({
                "agent_id": "task_manager_agent",
                "prompt": CHILD_PROMPT,
                "task_key": "headless-cron-child-task",
                "context": "headless cron regression",
                "model": "test-model"
            }),
        )];
        response.finish_reason = Some("tool_calls".into());
        Ok(response)
    }
}

#[tokio::test]
#[cfg(feature = "storage-sqlite")]
async fn scheduled_agent_history_includes_its_spawned_child_result() {
    let _inference = crate::inference::inference_test_guard_async().await;
    let model = std::sync::Arc::new(CronChildModel {
        spawn_calls: std::sync::atomic::AtomicUsize::new(0),
        child_calls: std::sync::atomic::AtomicUsize::new(0),
        child_invocation_dropped: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        accepted_followups: std::sync::atomic::AtomicUsize::new(0),
        accepted_followup: tokio::sync::Notify::new(),
        child_attempts_dangerous_tool: false,
        child_dangerous_tool_schema_seen: std::sync::atomic::AtomicBool::new(false),
        child_started: std::sync::Mutex::new(None),
        child_release: tokio::sync::Notify::new(),
        child_released: std::sync::atomic::AtomicBool::new(false),
    });
    let _model =
        crate::inference::provider::factory::test_provider_override::install_model(model.clone());
    let (_temp, _storage, _storage_guard, config, core, dispatch, job) =
        scheduled_agent_fixture(900);

    let (child_started_tx, child_started_rx) = tokio::sync::oneshot::channel();
    *model.child_started.lock().unwrap() = Some(child_started_tx);
    let scheduler_config = config.clone();
    let run =
        tokio::spawn(
            async move { dispatch_scheduled_agent(core, dispatch, scheduler_config).await },
        );
    // These deadlines are watchdogs, not timing assertions. Give loaded CI
    // workers enough room to start the isolated scheduler task.
    match tokio::time::timeout(std::time::Duration::from_secs(30), child_started_rx).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("child-start observation disconnected: {error}"),
        Err(_) => {
            model.release_child();
            let result = tokio::time::timeout(std::time::Duration::from_secs(30), run).await;
            eprintln!(
                "scheduled child start timeout diagnostics: spawn calls {}, followups {}, run result {result:?}",
                model.spawn_calls.load(std::sync::atomic::Ordering::SeqCst),
                model
                    .accepted_followups
                    .load(std::sync::atomic::Ordering::SeqCst),
            );
            panic!(
                "scheduled child never reached its model (spawn calls: {}, followups: {}); run result: {result:?}",
                model.spawn_calls.load(std::sync::atomic::Ordering::SeqCst),
                model
                    .accepted_followups
                    .load(std::sync::atomic::Ordering::SeqCst),
            );
        }
    }
    model.release_child();

    // Hard deadline: a stuck drain must fail this test in seconds, never ride
    // out the runner's 5-minute kill.
    match tokio::time::timeout(std::time::Duration::from_secs(30), run).await {
        Ok(joined) => joined.expect("cron scheduler task"),
        Err(_) => panic!(
            "scheduled run did not drain within 30s (spawn calls: {}, child calls: {}, followups: {})",
            model.spawn_calls.load(std::sync::atomic::Ordering::SeqCst),
            model.child_calls.load(std::sync::atomic::Ordering::SeqCst),
            model.accepted_followups.load(std::sync::atomic::Ordering::SeqCst),
        ),
    }
    let runs = list_runs_off_runtime(&config, &job.id)
        .await
        .expect("read persisted run history");
    assert_eq!(runs.len(), 1, "scheduler persisted one completed run");
    assert_eq!(runs[0].status, "ok", "agent job completed successfully");
    assert!(
        runs[0]
            .output
            .as_deref()
            .unwrap_or_default()
            .contains("headless-cron-child-result"),
        "the persisted cron result must include its spawned child output: {}",
        runs[0].output.as_deref().unwrap_or_default()
    );
    assert_eq!(
        model.spawn_calls.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(
        model.child_calls.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(
        model
            .accepted_followups
            .load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the parent accepted the real spawn result before the child completed"
    );
}

#[tokio::test]
#[cfg(feature = "storage-sqlite")]
async fn scheduled_agent_timeout_cancels_its_blocked_child_before_history_settles() {
    let _inference = crate::inference::inference_test_guard_async().await;
    let model = std::sync::Arc::new(CronChildModel {
        spawn_calls: std::sync::atomic::AtomicUsize::new(0),
        child_calls: std::sync::atomic::AtomicUsize::new(0),
        child_invocation_dropped: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        accepted_followups: std::sync::atomic::AtomicUsize::new(0),
        accepted_followup: tokio::sync::Notify::new(),
        child_attempts_dangerous_tool: false,
        child_dangerous_tool_schema_seen: std::sync::atomic::AtomicBool::new(false),
        child_started: std::sync::Mutex::new(None),
        child_release: tokio::sync::Notify::new(),
        child_released: std::sync::atomic::AtomicBool::new(false),
    });
    let _model =
        crate::inference::provider::factory::test_provider_override::install_model(model.clone());
    let (_temp, _storage, _storage_guard, config, core, dispatch, job) =
        scheduled_agent_fixture(15);
    let inspect_dispatch = dispatch.clone();
    let (child_started_tx, child_started_rx) = tokio::sync::oneshot::channel();
    *model.child_started.lock().unwrap() = Some(child_started_tx);
    let scheduler_config = config.clone();
    let run =
        tokio::spawn(
            async move { dispatch_scheduled_agent(core, dispatch, scheduler_config).await },
        );

    match tokio::time::timeout(std::time::Duration::from_secs(30), child_started_rx).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("scheduled child exited before its model blocked: {error}"),
        Err(_) => {
            model.release_child();
            let _ = tokio::time::timeout(std::time::Duration::from_secs(30), run).await;
            panic!(
                "scheduled child never reached its blocking model (spawns: {}, accepted followups: {})",
                model.spawn_calls.load(std::sync::atomic::Ordering::SeqCst),
                model
                    .accepted_followups
                    .load(std::sync::atomic::Ordering::SeqCst),
            );
        }
    }

    tokio::time::timeout(
        std::time::Duration::from_secs(12),
        model.accepted_followup.notified(),
    )
    .await
    .expect("the root acknowledges the spawned child before its deadline");

    tokio::time::timeout(std::time::Duration::from_secs(20), run)
        .await
        .expect("cron timeout should settle the blocked run")
        .expect("cron scheduler task");
    let owner = crate::agent::tinyagents::tool_registration::subagent_owner_key(
        &config.workspace_dir,
        &format!("cron:{}", job.id),
        None,
    );
    let registry = crate::core::runtime::slot_for::<
        tinyagents_orchestration::subagent::SubAgentJobRegistry,
    >(&inspect_dispatch);
    let jobs = registry
        .try_list_owned(owner.as_str())
        .expect("inspect this Cron run's native descendants");
    assert_eq!(jobs.len(), 1, "one native child belongs to this Cron job");
    assert_eq!(
        jobs[0].status,
        tinyagents_orchestration::subagent::SubAgentJobStatus::Cancelled,
        "the descendant is terminal when the scheduled run drains"
    );
    let child = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        registry.wait_owned(jobs[0].id.as_str(), owner),
    )
    .await
    .expect("root timeout cancellation reaches its child")
    .expect("the child remains available until its terminal state");
    assert_eq!(
        child.status,
        tinyagents_orchestration::subagent::SubAgentJobStatus::Cancelled,
        "the bounded registry wait confirms terminal cancellation"
    );
    assert_eq!(
        model.spawn_calls.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(
        model
            .accepted_followups
            .load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the root accepted the child before waiting on it"
    );
    assert_eq!(
        model.child_calls.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert!(
        model
            .child_invocation_dropped
            .load(std::sync::atomic::Ordering::SeqCst),
        "the root deadline dropped the child model future"
    );

    let runs = list_runs_off_runtime(&config, &job.id)
        .await
        .expect("read persisted run history");
    assert_eq!(runs.len(), 1, "scheduler persisted one completed attempt");
    assert_ne!(runs[0].status, "ok", "the configured deadline expired");
    assert!(!runs[0]
        .output
        .as_deref()
        .unwrap_or_default()
        .contains("headless-cron-child-result"));
    assert_eq!(
        child.status,
        tinyagents_orchestration::subagent::SubAgentJobStatus::Cancelled,
        "the root deadline cancels the blocked native child"
    );
}

#[tokio::test]
#[cfg(feature = "storage-sqlite")]
async fn scheduled_cron_child_cannot_run_its_declared_destructive_tool() {
    let _inference = crate::inference::inference_test_guard_async().await;
    let model = std::sync::Arc::new(CronChildModel {
        spawn_calls: std::sync::atomic::AtomicUsize::new(0),
        child_calls: std::sync::atomic::AtomicUsize::new(0),
        child_invocation_dropped: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        accepted_followups: std::sync::atomic::AtomicUsize::new(0),
        accepted_followup: tokio::sync::Notify::new(),
        child_attempts_dangerous_tool: true,
        child_dangerous_tool_schema_seen: std::sync::atomic::AtomicBool::new(false),
        child_started: std::sync::Mutex::new(None),
        child_release: tokio::sync::Notify::new(),
        child_released: std::sync::atomic::AtomicBool::new(false),
    });
    let _model =
        crate::inference::provider::factory::test_provider_override::install_model(model.clone());
    let (_temp, _storage, _storage_guard, config, core, dispatch, job) =
        scheduled_agent_fixture(900);
    let inspect_dispatch = dispatch.clone();
    let (child_started_tx, child_started_rx) = tokio::sync::oneshot::channel();
    *model.child_started.lock().unwrap() = Some(child_started_tx);
    let scheduler_config = config.clone();
    let run =
        tokio::spawn(
            async move { dispatch_scheduled_agent(core, dispatch, scheduler_config).await },
        );
    match tokio::time::timeout(std::time::Duration::from_secs(60), child_started_rx).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("scheduled child exited before its model call: {error}"),
        Err(_) => {
            let result = tokio::time::timeout(std::time::Duration::from_secs(30), run).await;
            let runs = list_runs_off_runtime(&config, &job.id).await;
            let events = match &runs {
                Ok(runs) => match runs.first().and_then(|run| run.run_id.as_deref()) {
                    Some(run_id) => crate::agent::tinyagents::journal::read_run_events(
                        &inspect_dispatch,
                        run_id,
                        0,
                    )
                    .await
                    .map(|events| {
                        events
                            .into_iter()
                            .map(|event| event.event)
                            .collect::<Vec<_>>()
                    }),
                    None => Ok(Vec::new()),
                },
                Err(error) => Err(anyhow::anyhow!("{error}")),
            };
            eprintln!(
                "scheduled child never reached its model (spawn calls: {}, child calls: {}, result: {result:?}, runs: {runs:?}, events: {events:?})",
                model.spawn_calls.load(std::sync::atomic::Ordering::SeqCst),
                model.child_calls.load(std::sync::atomic::Ordering::SeqCst),
            );
            panic!("scheduled child never reached its model");
        }
    }
    tokio::time::timeout(std::time::Duration::from_secs(30), run)
        .await
        .expect("Cron should settle the denied child result")
        .expect("Cron scheduler task");

    assert_eq!(
        model.spawn_calls.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(
        model.child_calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the child model requested the declared destructive tool"
    );
    assert!(
        model
            .child_dangerous_tool_schema_seen
            .load(std::sync::atomic::Ordering::SeqCst),
        "the real child model request exposed the declared destructive tool"
    );
    let owner = crate::agent::tinyagents::tool_registration::subagent_owner_key(
        &config.workspace_dir,
        &format!("cron:{}", job.id),
        None,
    );
    let jobs = crate::core::runtime::slot_for::<
        tinyagents_orchestration::subagent::SubAgentJobRegistry,
    >(&inspect_dispatch)
    .try_list_owned(owner)
    .expect("inspect the exact Cron session owner");
    assert_eq!(jobs.len(), 1, "one native child belongs to this Cron job");
    assert_eq!(
        jobs[0].status,
        tinyagents_orchestration::subagent::SubAgentJobStatus::Incomplete,
        "the child records the policy-blocked action as an incomplete run"
    );
    let child_run_id = jobs[0]
        .subagent_run_id
        .as_deref()
        .expect("native child records its own run id");
    let child_events =
        crate::agent::tinyagents::journal::read_run_events(&inspect_dispatch, child_run_id, 0)
            .await
            .expect("read the native child's durable tool events");
    let denial = child_events
        .iter()
        .find_map(|observation| match &observation.event {
            tinyagents_harness::events::AgentEvent::ToolCompleted {
                tool_name,
                error: Some(error),
                ..
            } if tool_name == "delete_artifact" => Some(error.as_str()),
            _ => None,
        })
        .expect("the native child journal records the denied delete_artifact call");
    assert!(denial.contains("Scheduled agent turns may use read-only tools only"));
    assert!(!denial.contains("not found"));
    let runs = list_runs_off_runtime(&config, &job.id)
        .await
        .expect("read persisted run history");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, "error");
}
