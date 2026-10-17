use super::*;
use crate::core::runtime::DispatchCtx;
use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet, Mode};
use crate::tools::toolpacks::ToolGroups;

const CHILD: &str = "OPENHUMAN_DETACHED_TENANT_TEST_CHILD";
const PARENT: &str = "same-parent-session";

// Mode is process-owned and irreversible. Each assertion runs in a fresh
// libtest process, without changing the mode of unrelated desktop tests.
fn child(name: &str, mode: Mode) -> bool {
    let module = module_path!().split_once("::").expect("crate module").1;
    let test = format!("{module}::{name}");
    if std::env::var(CHILD).as_deref() == Ok(test.as_str()) {
        crate::core::runtime::mode::lock_mode(mode).expect("lock child mode");
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", &test, "--nocapture", "--test-threads=1"])
        .env(CHILD, &test)
        .output()
        .expect("run isolated mode test");
    assert!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "isolated test {test} failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

fn profile(name: &str, workspace: &std::path::Path) -> Arc<CoreContext> {
    CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(
            crate::config::Config {
                workspace_dir: workspace.to_path_buf(),
                ..crate::config::Config::default()
            },
            DomainSet::kernel(),
            ToolGroups::none(),
        )
        .profile(name)
        .session_agent("same-agent"),
    )
}

struct Worker {
    task: String,
    session: String,
    queue: Arc<RunQueue<QueuedTurn>>,
    abort: AbortHandle,
    _status: watch::Sender<DetachedSubagentStatus>,
}

impl Worker {
    fn register(workspace: &std::path::Path) -> Self {
        let task = uuid::Uuid::new_v4().to_string();
        let session = uuid::Uuid::new_v4().to_string();
        let queue = run_queue();
        let (tx, rx) = status_channel();
        let task_handle = tokio::spawn(std::future::pending::<()>());
        let abort = task_handle.abort_handle();
        register(
            &DispatchCtx::ambient(),
            task.clone(),
            "researcher".into(),
            PARENT.into(),
            None,
            Some(session.clone()),
            workspace.to_path_buf(),
            None,
            queue.clone(),
            abort.clone(),
            rx,
        )
        .expect("fixture registration admitted");
        Self {
            task,
            session,
            queue,
            abort,
            _status: tx,
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        prune(&self.task);
        self.abort.abort();
    }
}

struct Pair {
    _a_dir: tempfile::TempDir,
    _b_dir: tempfile::TempDir,
    alice: Arc<CoreContext>,
    bob: Arc<CoreContext>,
    a: Worker,
    b: Worker,
}

impl Pair {
    fn new() -> Self {
        let a_dir = tempfile::tempdir().unwrap();
        let b_dir = tempfile::tempdir().unwrap();
        let alice = profile("detached-alice", a_dir.path());
        let bob = profile("detached-bob", b_dir.path());
        let a = CoreContext::sync_scope(alice.clone(), || Worker::register(a_dir.path()));
        let b = CoreContext::sync_scope(bob.clone(), || Worker::register(b_dir.path()));
        Self {
            _a_dir: a_dir,
            _b_dir: b_dir,
            alice,
            bob,
            a,
            b,
        }
    }
}

async fn control(function: &str, task: &str) -> serde_json::Value {
    let controller = crate::agent::orchestration::subagent_control::all_registered_controllers()
        .into_iter()
        .find(|c| c.schema.function == function)
        .expect("public controller");
    let mut params = serde_json::Map::new();
    params.insert("taskId".into(), serde_json::json!(task));
    if function == "steer" {
        params.insert("message".into(), serde_json::json!("ownership regression"));
    }
    (controller.handler)(DispatchCtx::ambient(), params)
        .await
        .expect("control response")
}

fn field(value: &serde_json::Value, key: &str) -> Option<bool> {
    value
        .get("data")
        .and_then(|v| v.get(key))
        .or_else(|| value.get(key))
        .and_then(serde_json::Value::as_bool)
}

async fn dispatched_control(function: &str, task: &str) -> Result<serde_json::Value, String> {
    let mut params = serde_json::json!({ "taskId": task });
    if function == "steer" {
        params["message"] = serde_json::json!("scoped dispatch regression");
    }
    crate::core::invoke::invoke_method(
        DispatchCtx::process_default(),
        crate::core::invoke::default_state(),
        &format!("openhuman.subagent_{function}"),
        params,
    )
    .await
}

#[tokio::test]
async fn saas_dispatch_exposes_only_the_callers_detached_worker_controls() {
    if !child(
        "saas_dispatch_exposes_only_the_callers_detached_worker_controls",
        Mode::Saas,
    ) {
        return;
    }
    let mut pair = Pair::new();
    let admitted = |name: &str, workspace: &std::path::Path| {
        CoreContext::for_test(DomainSet::saas(), None).derive_with(
            ContextOverlay::new(
                crate::config::Config {
                    workspace_dir: workspace.to_path_buf(),
                    ..crate::config::Config::default()
                },
                crate::profiles::host::user_domains(),
                ToolGroups::none(),
            )
            .profile(name)
            .session_agent("same-agent"),
        )
    };
    pair.alice = admitted("detached-alice", pair._a_dir.path());
    pair.bob = admitted("detached-bob", pair._b_dir.path());
    CoreContext::scope(pair.bob.clone(), async {
        for (function, field_name) in [("steer", "steered"), ("cancel", "cancelled")] {
            let foreign = dispatched_control(function, &pair.a.task)
                .await
                .expect("reviewed user-plane control must dispatch");
            assert_eq!(field(&foreign, field_name), Some(false));
        }
        assert_eq!(pair.a.queue.status().await.steers, 0);
        assert_eq!(
            field(
                &dispatched_control("steer", &pair.b.task).await.unwrap(),
                "steered"
            ),
            Some(true)
        );
        assert_eq!(pair.b.queue.status().await.steers, 1);
        assert_eq!(
            field(
                &dispatched_control("cancel", &pair.b.task).await.unwrap(),
                "cancelled"
            ),
            Some(true)
        );
        assert!(crate::core::all::schema_for_rpc_method("openhuman.config_get_config").is_none());
        let unreviewed =
            crate::agent::orchestration::command_center::all_command_center_registered_controllers(
            )
            .into_iter()
            .find(|controller| controller.schema.function == "list")
            .expect("existing Agent controller")
            .schema;
        let method = crate::core::all::rpc_method_name(&unreviewed);
        assert!(crate::core::all::schema_for_rpc_method(&method).is_none());
        assert!(
            crate::core::invoke::invoke_method(
                DispatchCtx::process_default(),
                crate::core::invoke::default_state(),
                &method,
                serde_json::json!({}),
            )
            .await
            .is_err(),
            "unreviewed Agent methods remain absent"
        );
    })
    .await;
    CoreContext::scope(pair.alice.clone(), async {
        assert!(resume_ref_for_task(&DispatchCtx::ambient(), &pair.a.task, PARENT).is_ok());
    })
    .await;
    for function in ["steer", "cancel"] {
        assert!(dispatched_control(function, &pair.a.task).await.is_err());
    }
}

#[tokio::test]
async fn unscoped_async_spawn_refuses_before_persisting_a_running_session() {
    if !child(
        "unscoped_async_spawn_refuses_before_persisting_a_running_session",
        Mode::Saas,
    ) {
        return;
    }
    use crate::agent::harness::fork_context::ParentExecutionContext;
    use crate::agent::orchestration::subagent_sessions::SubagentSessionStore;
    use crate::agent::orchestration::tools::{SpawnAsyncSubagentDispatch, SpawnAsyncSubagentTool};
    use tinyagents_harness::tool::ToolDispatch;
    let _ = crate::agent::harness::host_definition::AgentDefinitionRegistry::init_global_builtins();
    let workspace = tempfile::tempdir().unwrap();
    assert!(
        crate::storage::installed().is_none(),
        "exercise the real file fallback"
    );
    let parent = ParentExecutionContext {
        workspace_descriptor: None,
        agent_definition_id: "orchestrator".into(),
        allowed_subagent_ids: std::collections::HashSet::from(["task_manager_agent".into()]),
        turn_model_source: crate::agent::tinyagents::TurnModelSource::from_model(Arc::new(
            tinyagents_harness::testkit::ScriptedModel::replies(vec!["done"]),
        )),
        all_tools: Arc::new(Vec::new()),
        all_tool_specs: Arc::new(Vec::new()),
        visible_tool_specs: Arc::new(Vec::new()),
        visible_tool_names: Default::default(),
        subagent_tool_ceiling_names: Default::default(),
        model_name: "test-model".into(),
        temperature: 0.0,
        workspace_dir: workspace.path().to_path_buf(),
        agent_config: crate::config::AgentConfig::default(),
        workflows: Arc::new(Vec::new()),
        memory_context: Arc::new(None),
        session_id: PARENT.into(),
        channel: "test".into(),
        connected_integrations: Vec::new(),
        tool_call_format: tinytools_agent::dialect::ToolCallFormat::Native,
        session_key: "scope-refusal-parent".into(),
        session_parent_prefix: None,
        on_progress: None,
        run_queue: None,
        host_env: Default::default(),
    };
    let run = crate::agent::tinyagents::host::OpenHumanRunContext::new()
        .with_parent(parent)
        .into_tinyagents(
            tinyagents_harness::context::RunConfig::new("scope-refusal")
                .with_thread("scope-refusal-parent"),
        );
    let result = SpawnAsyncSubagentDispatch::new(Arc::new(SpawnAsyncSubagentTool::new()))
        .execute(
            &(),
            tinyagents_harness::ids::CallId::new("scope-refusal-spawn"),
            serde_json::json!({"agent_id": "task_manager_agent", "prompt": "deterministic fixture"}),
            tinytools::ToolCallOptions::default(),
            &run,
        )
        .await
        .expect("tool dispatch reports a refusal as a tool outcome");
    // An unscoped SaaS call fails closed on the store itself, so nothing was
    // persisted either way.
    let sessions = match SubagentSessionStore::new(workspace.path().to_path_buf()).load() {
        Ok(sessions) => sessions,
        Err(err) => {
            assert!(
                err.contains("no tenant scope in SaaS mode; refusing to act as the operator"),
                "unexpected store error: {err}"
            );
            Vec::new()
        }
    };
    assert!(
        result.is_error,
        "unscoped spawn must not report accepted: {}",
        result.output()
    );
    assert!(
        result.output().contains("scope"),
        "typed refusal must explain missing scope"
    );
    assert!(
        sessions.is_empty(),
        "refused spawn must not create durable sessions: {sessions:?}"
    );
    assert!(snapshot_for_parent(&DispatchCtx::ambient(), PARENT).is_empty());
    assert!(
        std::fs::read_dir(workspace.path())
            .unwrap()
            .next()
            .is_none(),
        "scope refusal must precede session and worker-thread writes"
    );
}

#[tokio::test]
async fn same_parent_rosters_are_profile_isolated() {
    if !child("same_parent_rosters_are_profile_isolated", Mode::Saas) {
        return;
    }
    let pair = Pair::new();
    for (ctx, own) in [(&pair.alice, &pair.a), (&pair.bob, &pair.b)] {
        let tasks = CoreContext::sync_scope(ctx.clone(), || {
            snapshot_for_parent(&DispatchCtx::ambient(), PARENT)
        });
        assert_eq!(
            tasks.iter().map(|t| t.task_id.as_str()).collect::<Vec<_>>(),
            [own.task.as_str()]
        );
    }
}

#[tokio::test]
async fn same_parent_resolution_and_wait_refuse_foreign_workers() {
    if !child(
        "same_parent_resolution_and_wait_refuse_foreign_workers",
        Mode::Saas,
    ) {
        return;
    }
    let pair = Pair::new();
    CoreContext::scope(pair.bob.clone(), async {
        assert!(matches!(
            task_id_for_session(&DispatchCtx::ambient(), &pair.a.session, PARENT),
            Err(WaitError::NotOwned)
        ));
        assert!(matches!(
            resume_ref_for_task(&DispatchCtx::ambient(), &pair.a.task, PARENT),
            Err(WaitError::NotOwned)
        ));
        assert!(matches!(
            wait(
                &DispatchCtx::ambient(),
                &pair.a.task,
                PARENT,
                Duration::ZERO
            )
            .await,
            Err(WaitError::NotOwned)
        ));
        assert_eq!(
            task_id_for_session(&DispatchCtx::ambient(), &pair.b.session, PARENT).unwrap(),
            pair.b.task
        );
    })
    .await;
}

#[tokio::test]
async fn same_parent_agent_steering_refuses_foreign_worker() {
    if !child(
        "same_parent_agent_steering_refuses_foreign_worker",
        Mode::Saas,
    ) {
        return;
    }
    let pair = Pair::new();
    CoreContext::scope(pair.bob.clone(), async {
        assert_eq!(
            steer(
                &DispatchCtx::ambient(),
                &pair.a.task,
                PARENT,
                "foreign".into(),
                QueueLane::Steer
            )
            .await,
            Err(SteerError::NotOwned)
        );
        assert_eq!(pair.a.queue.status().await.steers, 0);
        steer(
            &DispatchCtx::ambient(),
            &pair.b.task,
            PARENT,
            "own".into(),
            QueueLane::Steer,
        )
        .await
        .unwrap();
        assert_eq!(pair.b.queue.status().await.steers, 1);
    })
    .await;
}

#[tokio::test]
async fn public_steering_refuses_another_profiles_task() {
    if !child("public_steering_refuses_another_profiles_task", Mode::Saas) {
        return;
    }
    let pair = Pair::new();
    CoreContext::scope(pair.bob.clone(), async {
        assert_eq!(
            field(&control("steer", &pair.a.task).await, "steered"),
            Some(false)
        );
        assert_eq!(pair.a.queue.status().await.steers, 0);
        assert_eq!(
            field(&control("steer", &pair.b.task).await, "steered"),
            Some(true)
        );
        assert_eq!(pair.b.queue.status().await.steers, 1);
    })
    .await;
}

#[tokio::test]
async fn public_cancellation_refuses_another_profiles_task() {
    if !child(
        "public_cancellation_refuses_another_profiles_task",
        Mode::Saas,
    ) {
        return;
    }
    let pair = Pair::new();
    CoreContext::scope(pair.bob.clone(), async {
        assert_eq!(
            field(&control("cancel", &pair.a.task).await, "cancelled"),
            Some(false)
        );
        assert!(resume_ref_for_task(&DispatchCtx::ambient(), &pair.b.task, PARENT).is_ok());
        assert_eq!(
            field(&control("cancel", &pair.b.task).await, "cancelled"),
            Some(true)
        );
    })
    .await;
    CoreContext::scope(pair.alice.clone(), async {
        assert!(
            resume_ref_for_task(&DispatchCtx::ambient(), &pair.a.task, PARENT).is_ok(),
            "foreign cancel leaves original worker live"
        );
    })
    .await;
}

#[tokio::test]
async fn missing_saas_scope_cannot_observe_or_control_workers() {
    if !child(
        "missing_saas_scope_cannot_observe_or_control_workers",
        Mode::Saas,
    ) {
        return;
    }
    let pair = Pair::new();
    assert!(CoreContext::scoped().is_none());
    assert!(snapshot_for_parent(&DispatchCtx::ambient(), PARENT).is_empty());
    assert!(matches!(
        resume_ref_for_task(&DispatchCtx::ambient(), &pair.a.task, PARENT),
        Err(WaitError::NotOwned)
    ));
    assert_eq!(
        steer(
            &DispatchCtx::ambient(),
            &pair.a.task,
            PARENT,
            "unscoped".into(),
            QueueLane::Steer
        )
        .await,
        Err(SteerError::NotOwned)
    );
    assert_eq!(
        field(&control("steer", &pair.a.task).await, "steered"),
        Some(false)
    );
    assert_eq!(
        field(&control("cancel", &pair.a.task).await, "cancelled"),
        Some(false)
    );
    assert_eq!(pair.a.queue.status().await.steers, 0);
    CoreContext::sync_scope(pair.alice.clone(), || {
        assert!(resume_ref_for_task(&DispatchCtx::ambient(), &pair.a.task, PARENT).is_ok())
    });
    // A scoped kernel context without an admitted profile is not authority.
    CoreContext::scope(CoreContext::for_test(DomainSet::full(), None), async {
        assert!(snapshot_for_parent(&DispatchCtx::ambient(), PARENT).is_empty());
        assert!(matches!(
            resume_ref_for_task(&DispatchCtx::ambient(), &pair.a.task, PARENT),
            Err(WaitError::NotOwned)
        ));
        assert_eq!(
            field(&control("steer", &pair.a.task).await, "steered"),
            Some(false)
        );
        assert_eq!(
            field(&control("cancel", &pair.a.task).await, "cancelled"),
            Some(false)
        );
    })
    .await;
}

#[tokio::test]
async fn desktop_public_controls_remain_available_without_parent_scope() {
    if !child(
        "desktop_public_controls_remain_available_without_parent_scope",
        Mode::SingleUser,
    ) {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let worker = Worker::register(directory.path());
    assert_eq!(
        snapshot_for_parent(&DispatchCtx::ambient(), PARENT).len(),
        1
    );
    assert_eq!(
        field(&control("steer", &worker.task).await, "steered"),
        Some(true)
    );
    assert_eq!(worker.queue.status().await.steers, 1);
    assert_eq!(
        field(&control("cancel", &worker.task).await, "cancelled"),
        Some(true)
    );
}

/// The explicit context decides, with no task scope anywhere: a worker is
/// reachable through its own profile's context and refused through another's,
/// across awaits, from spawned tasks and concurrently.
#[tokio::test]
async fn saas_controls_follow_the_explicit_context_not_the_tasks_scope() {
    if !child(
        "saas_controls_follow_the_explicit_context_not_the_tasks_scope",
        Mode::Saas,
    ) {
        return;
    }
    let pair = Pair::new();
    let alice = DispatchCtx::new(pair.alice.clone());
    let bob = DispatchCtx::new(pair.bob.clone());
    let (a_task, b_task) = (pair.a.task.clone(), pair.b.task.clone());

    // Not a single `CoreContext::scope` here: with the ambient reader the
    // task would be unscoped, which fails closed in SaaS.
    assert!(resume_ref_for_task(&alice, &a_task, PARENT).is_ok());
    assert!(matches!(
        resume_ref_for_task(&bob, &a_task, PARENT),
        Err(WaitError::NotOwned)
    ));
    assert!(cancel_by_task(&bob, &a_task).is_none());
    assert_eq!(
        steer_control(&bob, &a_task, "x".into(), QueueLane::Steer).await,
        Err(SteerError::NotOwned)
    );
    assert_eq!(pair.a.queue.status().await.steers, 0);
    assert!(snapshot_for_parent(&bob, PARENT)
        .iter()
        .all(|worker| worker.task_id != a_task));

    assert!(
        cancel_by_task(&alice, &b_task).is_none(),
        "alice cannot cancel bob's worker"
    );

    // Concurrent, across awaits, and from spawned tasks: each carries its own.
    let spawned = |ctx: DispatchCtx, task: String| {
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            let own = resume_ref_for_task(&ctx, &task, PARENT).is_ok();
            tokio::task::yield_now().await;
            let foreign = cancel_by_task(&ctx, &task).is_some();
            (own, foreign)
        })
    };
    let (a_side, b_side) = tokio::join!(
        spawned(alice.clone(), a_task.clone()),
        spawned(bob.clone(), b_task.clone())
    );
    assert_eq!(
        a_side.unwrap(),
        (true, true),
        "alice reaches her own worker"
    );
    assert_eq!(b_side.unwrap(), (true, true), "bob reaches his own worker");
    assert!(
        resume_ref_for_task(&alice, &a_task, PARENT).is_err(),
        "each cancel removed only that profile's worker, and it is gone"
    );
}
