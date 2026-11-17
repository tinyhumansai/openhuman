use super::*;
use crate::agent::subagent_host::{spawn_async_via_driver_tool_with_jobs, SubagentRunStatus};

#[tokio::test]
async fn prepared_subagent_runs_through_driver_jobs_dispatcher() {
    let scratch = tempfile::tempdir().expect("isolated async spawn fixture");
    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create isolated workspace");
    let provider = ScriptedProvider::new(vec![text_response("driver result")]);
    let mut parent_context = make_parent(provider.clone(), vec![]);
    parent_context.workspace_dir = workspace;
    let mut options = with_parent(parent_context, SubagentRunOptions::default());
    options.checkpoint_dir = Some(scratch.path().join("checkpoints"));
    options.run_context.thread_id = Some("driver-spawn-thread".into());
    let parent = options.run_context.clone().into_tinyagents(
        tinyagents_harness::context::RunConfig::new("driver-spawn-parent")
            .with_thread("driver-spawn-thread"),
    );

    let jobs = tinyagents_orchestration::subagent::SubAgentJobRegistry::new();
    let (completed_tx, completed_rx) = tokio::sync::oneshot::channel();
    let completed_tx = Arc::new(std::sync::Mutex::new(Some(completed_tx)));
    let task_id = spawn_async_via_driver_tool_with_jobs(
        &parent,
        make_def_named_tools(&[]),
        "return the requested result".into(),
        options,
        jobs.clone(),
        move |_target, result, effects| {
            let completed_tx = Arc::clone(&completed_tx);
            Box::pin(async move {
                if let Some(sender) = completed_tx.lock().unwrap().take() {
                    let _ = sender.send((result, effects));
                }
            })
        },
    )
    .await
    .expect("detached driver spawn should register");
    let (outcome, _effects) = tokio::time::timeout(std::time::Duration::from_secs(5), completed_rx)
        .await
        .expect("detached driver should complete")
        .expect("completion callback sends the host result");
    let outcome = outcome.expect("driver-backed child should complete");

    assert!(matches!(outcome.status, SubagentRunStatus::Completed));
    assert_eq!(outcome.output, "driver result");
    assert!(
        !outcome.task_id.is_empty(),
        "host must retain generated task ID"
    );
    assert_eq!(provider.captured.lock().len(), 1, "child provider must run");
    let job = jobs.get(&task_id).expect("driver tool registered the job");
    assert_eq!(job.agent, "test");
    assert_eq!(
        job.status,
        tinyagents_orchestration::subagent::SubAgentJobStatus::Completed
    );
}

struct BlockingCrossBundleTool {
    started: tokio::sync::mpsc::Sender<()>,
    release: Arc<tokio::sync::Notify>,
    executions: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl tinytools::Tool for BlockingCrossBundleTool {
    fn name(&self) -> &str {
        "blocking_cross_bundle_fixture"
    }

    fn description(&self) -> &str {
        "Blocks the shared child until the test releases it."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<tinytools::ToolResult> {
        self.executions
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _ = self.started.send(()).await;
        self.release.notified().await;
        Ok(tinytools::ToolResult::success("released"))
    }

    fn permission_level(&self) -> tinytools::PermissionLevel {
        tinytools::PermissionLevel::None
    }
}

#[tokio::test]
async fn independent_host_bundles_share_and_project_the_persisted_winner() {
    use tinyagents_orchestration::subagent::SubagentTaskKey;

    let scratch = tempfile::tempdir().expect("isolated cross-bundle fixture");
    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create isolated workspace");
    let (started_tx, mut started_rx) = tokio::sync::mpsc::channel(1);
    let release = Arc::new(tokio::sync::Notify::new());
    let executions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let provider = ScriptedProvider::new(vec![
        tool_response("blocking_cross_bundle_fixture", "{}"),
        {
            let mut response = text_response(
                "[artifact] kind=output path=outputs/report.md bytes=1\nreport ready",
            );
            response.usage = Some(tinyinference_llm::usage::Usage {
                input_tokens: 12,
                output_tokens: 7,
                total_tokens: 19,
                ..Default::default()
            });
            response
        },
    ]);
    let mut parent_context = make_parent(
        provider.clone(),
        vec![Box::new(BlockingCrossBundleTool {
            started: started_tx,
            release: release.clone(),
            executions: executions.clone(),
        })],
    );
    parent_context.workspace_dir = workspace;
    let definition = make_def_named_tools(&["blocking_cross_bundle_fixture"]);
    let mut options = with_parent(parent_context, SubagentRunOptions::default());
    options.task_id = Some("cross-bundle-task".into());
    options.thread_id = Some("cross-bundle-thread".into());
    let checkpoint_dir = scratch.path().join("checkpoints");
    options.checkpoint_dir = Some(checkpoint_dir.clone());
    let mut test_config = crate::config::Config::default();
    test_config.components.juice = false;
    let core = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        test_config,
    );
    options.run_context = options.run_context.with_core(Some(Arc::clone(&core)));
    let leader_parent = options.run_context.clone().into_tinyagents(
        tinyagents_harness::context::RunConfig::new("cross-bundle-parent")
            .with_thread("cross-bundle-thread"),
    );
    let follower_parent = options.run_context.clone().into_tinyagents(
        tinyagents_harness::context::RunConfig::new("cross-bundle-parent")
            .with_thread("cross-bundle-thread"),
    );
    let task_key = SubagentTaskKey::from_context(&leader_parent, "cross-bundle-task", None);
    let flights = crate::agent::subagent_host::lifecycle::shared_subagent_flight(
        &leader_parent.data.dispatch_ctx(),
        &checkpoint_dir,
    );
    let leader_definition = definition.clone();
    let leader_options = options.clone();
    let leader = tokio::spawn(async move {
        crate::agent::subagent_host::OpenHumanSubagentHost::new(leader_definition, leader_options)
            .run(&leader_parent, "produce a report".into())
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), started_rx.recv())
        .await
        .expect("leader reaches the blocking child tool")
        .expect("blocking tool start notification");
    let leader_entry = flights
        .get(&task_key)
        .expect("leader reserves the shared task key");

    let cancelled_context = options.run_context.clone();
    let cancelled_options = options.clone();
    let follower = tokio::spawn(async move {
        crate::agent::subagent_host::OpenHumanSubagentHost::new(definition, options)
            .run(&follower_parent, "produce a report".into())
            .await
    });
    let mut cancelled_parent = cancelled_context.into_tinyagents(
        tinyagents_harness::context::RunConfig::new("cross-bundle-parent")
            .with_thread("cross-bundle-thread"),
    );
    let observer_cancellation = tinyagents_harness::CancellationToken::new();
    cancelled_parent.cancellation = observer_cancellation.clone();
    let cancelled_definition = make_def_named_tools(&["blocking_cross_bundle_fixture"]);
    let cancelled_follower = tokio::spawn(async move {
        crate::agent::subagent_host::OpenHumanSubagentHost::new(
            cancelled_definition,
            cancelled_options,
        )
        .run(&cancelled_parent, "produce a report".into())
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while leader_entry.waiter_count() != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("both independent host bundles join the leader's flight");

    observer_cancellation.cancel();
    let cancelled = tokio::time::timeout(std::time::Duration::from_secs(5), cancelled_follower)
        .await
        .expect("cancelled observer exits independently")
        .expect("cancelled observer task")
        .expect("observer cancellation is a normal lifecycle outcome");
    assert!(
        matches!(cancelled.status, SubagentRunStatus::Cancelled),
        "the cancelled observer receives its own cancelled result"
    );
    assert_eq!(
        executions.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "cancelling a waiter does not cancel or duplicate the leader"
    );

    release.notify_waiters();
    let leader = tokio::time::timeout(std::time::Duration::from_secs(5), leader)
        .await
        .expect("leader completes after release")
        .expect("leader task")
        .expect("leader outcome");
    let follower = tokio::time::timeout(std::time::Duration::from_secs(5), follower)
        .await
        .expect("follower receives the persisted winner")
        .expect("follower task")
        .expect("follower outcome");

    assert_eq!(leader.output, follower.output);
    assert_eq!(leader.agent_id, follower.agent_id);
    assert_eq!(leader.iterations, follower.iterations);
    assert_eq!(leader.elapsed, follower.elapsed);
    assert_eq!(leader.mode, follower.mode);
    assert!(matches!(leader.status, SubagentRunStatus::Completed));
    assert!(matches!(follower.status, SubagentRunStatus::Completed));
    assert_eq!(leader.final_history.len(), follower.final_history.len());
    // The neutral transcript contract identifies a tool result row by its
    // call ID, even when the host's transient row left `id` unset. Normalize
    // only that documented field so all other native row metadata remains
    // subject to the full-history equality check.
    let mut canonical_leader_history = leader.final_history.clone();
    for row in &mut canonical_leader_history {
        if row.role == "tool" && !row.preserve_request_id {
            row.id = row.tool_call_id.clone().or_else(|| row.id.clone());
        }
    }
    assert_eq!(
        serde_json::to_value(&canonical_leader_history).unwrap(),
        serde_json::to_value(&follower.final_history).unwrap(),
        "follower receives the full winner history"
    );
    assert!(
        !leader.final_history.is_empty(),
        "winner history is preserved"
    );
    assert!(leader
        .final_history
        .iter()
        .any(|row| { row.role == "tool" && row.tool_call_id.as_deref() == Some("call-1") }));
    assert_eq!(leader.usage.input_tokens, follower.usage.input_tokens);
    assert!(leader.usage.input_tokens > 0, "winner usage is preserved");
    assert_eq!(leader.artifact_paths, follower.artifact_paths);
    assert_eq!(leader.artifact_paths, vec!["outputs/report.md"]);
    assert_eq!(
        leader.persistence_disposition,
        tinyagents_orchestration::subagent::SubagentPersistenceDisposition::TerminalInserted
    );
    assert_eq!(
        follower.persistence_disposition,
        tinyagents_orchestration::subagent::SubagentPersistenceDisposition::TerminalExisting
    );
    assert_eq!(
        executions.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the child executes once across independently constructed bundles"
    );
    assert_eq!(provider.captured.lock().len(), 2, "one model loop ran");
}

#[tokio::test]
async fn closing_a_durable_driver_session_cancels_only_its_registered_child() {
    use crate::agent::orchestration::subagent_sessions::{
        DurableSubagentSession, DurableSubagentStatus, OpenHumanTinySubagentSessionStore,
        SubagentSessionStore as HostSessionStore,
    };
    use tinyagents_harness_tools::subagents::{SubAgentJobAction, SubAgentJobsTool};
    use tinyagents_orchestration::subagent::{
        SubAgentJobRegistry, SubAgentJobStatus, SubagentSessionStore as TinySessionStore,
        SubagentTaskKey,
    };

    let scratch = tempfile::tempdir().expect("isolated driver close fixture");
    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create isolated workspace");
    let (started_tx, mut started_rx) = tokio::sync::mpsc::channel(1);
    let (finished_tx, mut finished_rx) = tokio::sync::mpsc::channel(1);
    let (completion_tx, mut completion_rx) = tokio::sync::mpsc::channel(1);
    let release = Arc::new(tokio::sync::Notify::new());
    let provider = ScriptedProvider::new(vec![
        tool_response("blocking_close_fixture", "{}"),
        text_response("sibling completed"),
    ]);
    let mut parent_context = make_parent(
        provider.clone(),
        vec![Box::new(BlockingSubagentCloseTool {
            started: started_tx,
            release: release.clone(),
            finished: finished_tx,
        })],
    );
    parent_context.workspace_dir = workspace.clone();
    let mut definition = make_def_named_tools(&["blocking_close_fixture"]);
    definition.id = "cancelled-child".into();
    let mut options = with_parent(parent_context, SubagentRunOptions::default());
    options.task_id = Some("close-driver-task".into());
    options.thread_id = Some("close-driver-thread".into());
    options.checkpoint_dir = Some(scratch.path().join("checkpoints"));
    let mut test_config = crate::config::Config::default();
    // This lifecycle fixture exercises driver cancellation, not TinyJuice's
    // module-backed output compaction. Keep the optional component disabled so
    // the scripted child cannot wait on a module service that this unit test
    // does not install.
    test_config.components.juice = false;
    let core = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        test_config,
    );
    options.run_context = options.run_context.with_core(Some(Arc::clone(&core)));
    let parent = options.run_context.clone().into_tinyagents(
        tinyagents_harness::context::RunConfig::new("close-driver-parent")
            .with_thread("close-driver-thread"),
    );
    let parent_cancellation = parent.data.child().cancellation;
    let jobs =
        (*crate::core::runtime::slot_for::<SubAgentJobRegistry>(&parent.data.dispatch_ctx()))
            .clone();
    let task_key = SubagentTaskKey::from_context(&parent, "close-driver-task", None);
    let now = chrono::Utc::now().to_rfc3339();
    let sessions = HostSessionStore::new(workspace.clone());
    sessions
        .save(&[DurableSubagentSession {
            subagent_session_id: "close-driver-session".into(),
            parent_session: "test-session".into(),
            parent_thread_id: Some("close-driver-thread".into()),
            worker_thread_id: None,
            agent_id: "cancelled-child".into(),
            display_name: None,
            model: None,
            sandbox_mode: "workspace".into(),
            action_root: None,
            task_key: "close driver lifecycle".into(),
            task_title: "Close driver lifecycle".into(),
            current_task_id: Some("close-driver-task".into()),
            lifecycle_key: Some(task_key),
            parent_epoch: None,
            status: DurableSubagentStatus::Running,
            reusable: true,
            latest_history: None,
            latest_error: None,
            created_at: now.clone(),
            updated_at: now.clone(),
            last_used_at: now,
        }])
        .expect("save the authenticated durable session");

    let _spawn_reference = spawn_async_via_driver_tool_with_jobs(
        &parent,
        definition,
        "must be cancelled before the child runs".into(),
        options,
        jobs.clone(),
        move |_target, result, _emit_effects| {
            let completion_tx = completion_tx.clone();
            Box::pin(async move {
                let _ = completion_tx.send(result).await;
            })
        },
    )
    .await
    .expect("background driver registers the child");
    let job_id = jobs
        .list()
        .into_iter()
        .find(|job| job.agent == "cancelled-child")
        .expect("driver child is visible in the shared registry")
        .id;
    tokio::time::timeout(std::time::Duration::from_secs(5), started_rx.recv())
        .await
        .expect("child reaches its blocking tool")
        .expect("blocking tool notification");

    let port = OpenHumanTinySubagentSessionStore::new(
        workspace.clone(),
        "test-session",
        Some("close-driver-thread".into()),
    )
    .with_jobs(jobs.clone());
    let owner = crate::agent::tinyagents::tool_registration::subagent_owner_key(
        &workspace,
        "test-session",
        Some("close-driver-thread"),
    );
    let owner_scope: std::sync::Arc<dyn Fn(u64) -> String + Send + Sync> =
        std::sync::Arc::new(move |_| owner.clone());
    assert!(port
        .close("close-driver-session")
        .await
        .expect("Tiny close adapter cancels then closes the durable row"));
    assert_eq!(
        jobs.get(job_id.as_str()).unwrap().status,
        SubAgentJobStatus::Running,
        "the job remains visible while cooperative cancellation unwinds"
    );
    assert!(port
        .close("close-driver-session")
        .await
        .expect("repeated close is idempotent without re-cancelling"));
    assert!(!parent_cancellation.is_cancelled(), "parent stays live");

    let tiny_store: std::sync::Arc<dyn TinySessionStore> = std::sync::Arc::new(port.clone());
    let wait = SubAgentJobsTool::new(jobs.clone())
        .with_tool_name("wait_subagent")
        .with_fixed_action(SubAgentJobAction::Await)
        .with_identifier_fields("task_id", "subagent_session_id")
        .with_await_timeout_field("timeout_secs", 1, 3600)
        .with_owner_scope(owner_scope.clone())
        .with_sessions(tiny_store, |_| "test-session".to_owned());
    let wait_result = tinyagents_harness::tool::ToolDispatch::execute(
        &wait,
        &(),
        tinyagents_harness::CallId::new("wait-cancelled-close-child"),
        serde_json::json!({"task_id":"close-driver-task","timeout_secs":1}),
        tinytools::ToolCallOptions::default(),
        &parent,
    )
    .await
    .expect("Tiny await observes close cancellation");
    assert!(
        !wait_result.is_error,
        "raw product task ID resolves through the authenticated session port: {}",
        wait_result.output()
    );
    let running_wait: serde_json::Value =
        serde_json::from_str(&wait_result.output()).expect("Tiny wait returns a JSON job snapshot");
    assert_eq!(running_wait["id"].as_str(), Some(job_id.as_str()));
    assert_eq!(running_wait["status"], "running");
    release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(2), finished_rx.recv())
        .await
        .expect("released tool future must unwind")
        .expect("tool reports that it unwound");
    let completion = tokio::time::timeout(std::time::Duration::from_secs(2), completion_rx.recv())
        .await
        .expect("driver should finish after the tool unwinds")
        .expect("driver invokes the host completion projection");
    assert!(
        matches!(&completion, Ok(outcome) if matches!(&outcome.status, SubagentRunStatus::Cancelled)),
        "the driver should classify the released child as cancelled: {completion:?}"
    );
    let final_wait = SubAgentJobsTool::new(jobs.clone())
        .with_tool_name("wait_subagent")
        .with_fixed_action(SubAgentJobAction::Await)
        .with_identifier_fields("task_id", "subagent_session_id")
        .with_required_identifier()
        .with_owner_scope(owner_scope.clone())
        .with_sessions(std::sync::Arc::new(port.clone()), |_| {
            "test-session".to_owned()
        });
    let settled = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tinyagents_harness::tool::ToolDispatch::execute(
            &final_wait,
            &(),
            tinyagents_harness::CallId::new("wait-cancelled-close-child-settled"),
            serde_json::json!({"task_id":"close-driver-task"}),
            tinytools::ToolCallOptions::default(),
            &parent,
        ),
    )
    .await;
    assert!(
        settled.is_ok(),
        "shared Jobs wait should settle after tool unwind: job={:?}, provider_calls={}",
        jobs.get(job_id.as_str()),
        provider.captured.lock().len()
    );
    let settled = settled
        .expect("shared Jobs wait observes cooperative cancellation settlement")
        .expect("Tiny await dispatches successfully");
    assert!(!settled.is_error, "settled Jobs wait must remain a result");
    assert_eq!(
        jobs.get(job_id.as_str()).unwrap().status,
        SubAgentJobStatus::Cancelled
    );

    let cancelled = jobs
        .list()
        .into_iter()
        .find(|job| job.agent == "cancelled-child")
        .expect("driver child is visible in the shared registry");
    assert_eq!(cancelled.status, SubAgentJobStatus::Cancelled);
    assert_eq!(
        sessions.load().unwrap()[0].status,
        DurableSubagentStatus::Closed,
        "the completion callback cannot revive a closed session"
    );
    assert_eq!(
        provider.captured.lock().len(),
        1,
        "child reaches one tool call"
    );

    let mut sibling_options = SubagentRunOptions::default();
    sibling_options.task_id = Some("close-driver-sibling-task".into());
    sibling_options.thread_id = Some("close-driver-thread".into());
    sibling_options.checkpoint_dir = Some(scratch.path().join("checkpoints"));
    sibling_options.run_context = parent.data.clone();
    let mut sibling = make_def_named_tools(&[]);
    sibling.id = "live-sibling".into();
    let (sibling_tx, sibling_rx) = tokio::sync::oneshot::channel();
    let sibling_tx = Arc::new(std::sync::Mutex::new(Some(sibling_tx)));
    spawn_async_via_driver_tool_with_jobs(
        &parent,
        sibling,
        "complete independently".into(),
        sibling_options,
        tinyagents_orchestration::subagent::SubAgentJobRegistry::new(),
        move |_target, result, effects| {
            let sibling_tx = Arc::clone(&sibling_tx);
            Box::pin(async move {
                if let Some(sender) = sibling_tx.lock().unwrap().take() {
                    let _ = sender.send((result, effects));
                }
            })
        },
    )
    .await
    .expect("sibling driver remains usable");
    let (sibling, _effects) = tokio::time::timeout(std::time::Duration::from_secs(5), sibling_rx)
        .await
        .expect("sibling driver should complete")
        .expect("sibling completion callback sends the result");
    assert!(matches!(
        sibling.expect("sibling child completes").status,
        SubagentRunStatus::Completed
    ));
    assert_eq!(provider.captured.lock().len(), 2);
}

#[tokio::test]
async fn stopped_parent_thread_rejects_a_new_driver_spawn_before_execution() {
    let scratch = tempfile::tempdir().expect("isolated stopped-thread fixture");
    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create isolated workspace");
    let provider = ScriptedProvider::new(vec![text_response("must not run")]);
    let mut parent_context = make_parent(provider.clone(), vec![]);
    parent_context.workspace_dir = workspace;
    let mut options = with_parent(parent_context, SubagentRunOptions::default());
    options.checkpoint_dir = Some(scratch.path().join("checkpoints"));
    options.run_context.thread_id = Some("stopped-driver-thread".into());
    let parent = options.run_context.clone().into_tinyagents(
        tinyagents_harness::context::RunConfig::new("stopped-driver-parent")
            .with_thread("stopped-driver-thread"),
    );
    let dispatch_ctx = parent.data.dispatch_ctx();
    crate::agent::orchestration::background_completions::discard_pending_for_thread(
        &dispatch_ctx,
        "stopped-driver-thread",
    );

    let result = spawn_async_via_driver_tool_with_jobs(
        &parent,
        make_def_named_tools(&[]),
        "return the requested result".into(),
        options,
        tinyagents_orchestration::subagent::SubAgentJobRegistry::new(),
        |_target, _result, _effects| Box::pin(async {}),
    )
    .await;

    assert!(result.is_err(), "stopped thread must reject new children");
    assert!(
        provider.captured.lock().is_empty(),
        "the child model must not run after Stop"
    );
}

#[tokio::test]
async fn detached_driver_spawn_repairs_replayed_host_outcome_without_reexecution() {
    let scratch = tempfile::tempdir().expect("isolated async spawn fixture");
    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create isolated workspace");
    let provider = ScriptedProvider::new(vec![text_response("durable driver result")]);
    let mut parent_context = make_parent(provider.clone(), vec![]);
    parent_context.workspace_dir = workspace;
    let parent_options = with_parent(parent_context, SubagentRunOptions::default());
    let parent = parent_options.clone().run_context.into_tinyagents(
        tinyagents_harness::context::RunConfig::new("driver-replay-parent")
            .with_thread("driver-replay-thread"),
    );

    let (first_tx, first_rx) = tokio::sync::oneshot::channel();
    let first_tx = Arc::new(std::sync::Mutex::new(Some(first_tx)));
    let mut first_options = parent_options.clone();
    first_options.task_id = Some("driver-replay-task".into());
    first_options.checkpoint_dir = Some(scratch.path().join("checkpoints"));
    first_options.run_context.thread_id = Some("driver-replay-thread".into());
    spawn_async_via_driver_tool_with_jobs(
        &parent,
        make_def_named_tools(&[]),
        "return the persisted result".into(),
        first_options,
        tinyagents_orchestration::subagent::SubAgentJobRegistry::new(),
        move |_target, result, effects| {
            let first_tx = first_tx.clone();
            Box::pin(async move {
                if let Some(first_tx) = first_tx.lock().unwrap().take() {
                    let _ = first_tx.send((result, effects));
                }
            })
        },
    )
    .await
    .expect("first detached driver spawn should register");
    let (first_result, first_effects) =
        tokio::time::timeout(std::time::Duration::from_secs(5), first_rx)
            .await
            .expect("first completion callback should not be skipped")
            .expect("first completion callback");
    let first_result = first_result.expect("first host outcome");
    assert!(first_effects);
    assert_eq!(first_result.output, "durable driver result");
    assert_eq!(provider.captured.lock().len(), 1, "child runs exactly once");

    let (replay_tx, replay_rx) = tokio::sync::oneshot::channel();
    let replay_tx = Arc::new(std::sync::Mutex::new(Some(replay_tx)));
    let mut replay_options = parent_options;
    replay_options.task_id = Some("driver-replay-task".into());
    replay_options.checkpoint_dir = Some(scratch.path().join("checkpoints"));
    replay_options.run_context.thread_id = Some("driver-replay-thread".into());
    spawn_async_via_driver_tool_with_jobs(
        &parent,
        make_def_named_tools(&[]),
        "return the persisted result".into(),
        replay_options,
        tinyagents_orchestration::subagent::SubAgentJobRegistry::new(),
        move |_target, result, effects| {
            let replay_tx = replay_tx.clone();
            Box::pin(async move {
                if let Some(replay_tx) = replay_tx.lock().unwrap().take() {
                    let _ = replay_tx.send((result, effects));
                }
            })
        },
    )
    .await
    .expect("replayed detached driver spawn should register");
    let (replay_result, replay_effects) =
        tokio::time::timeout(std::time::Duration::from_secs(5), replay_rx)
            .await
            .expect("replay repair callback should run")
            .expect("replay repair callback");
    let replay_result = replay_result.expect("durable replay must retain host result");
    assert!(!replay_effects, "terminal replay must not repeat effects");
    assert_eq!(replay_result.output, "durable driver result");
    assert_eq!(replay_result.agent_id, "test");
    assert_eq!(
        provider.captured.lock().len(),
        1,
        "replay must not execute child"
    );
}
