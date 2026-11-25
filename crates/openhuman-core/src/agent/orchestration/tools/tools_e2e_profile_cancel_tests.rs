use super::*;

#[tokio::test]
async fn profile_scope_cannot_cancel_another_profiles_driver_child() {
    const CHILD: &str = "OPENHUMAN_SAAS_SUBAGENT_CANCEL_SCOPE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let module_path = module_path!().trim_start_matches("openhuman_core::");
        let test_name =
            format!("{module_path}::profile_scope_cannot_cancel_another_profiles_driver_child");
        let output =
            std::process::Command::new(std::env::current_exe().expect("unit test executable path"))
                .arg("--exact")
                .arg(&test_name)
                .arg("--nocapture")
                .env(CHILD, "1")
                .output()
                .expect("run isolated SaaS-mode driver cancellation case");
        assert!(
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains(&test_name)
                && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "isolated SaaS cancellation case failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    use crate::agent::orchestration::subagent_sessions::DurableSubagentStatus;
    use crate::agent::tinyagents::host::OpenHumanRunContext;
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    use crate::tools::toolpacks::ToolGroups;
    use tinyagents_harness::context::RunContext;
    use tinyagents_orchestration::subagent::SubAgentJobStatus;

    struct GatedModel {
        started: tokio::sync::mpsc::Sender<()>,
        release: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl ChatModel<()> for GatedModel {
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
            Ok(ModelResponse::assistant("cancelled child was released"))
        }
    }

    crate::core::runtime::lock_mode(crate::core::runtime::Mode::Saas)
        .expect("the child process starts with an unlocked SaaS mode");
    let scratch = tempfile::tempdir().expect("profile cancellation workspace");
    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create shared workspace");
    let (started_tx, mut started_rx) = tokio::sync::mpsc::channel(1);
    let release = Arc::new(tokio::sync::Notify::new());
    let provider = Arc::new(GatedModel {
        started: started_tx,
        release: Arc::clone(&release),
    });
    let mut parent = parent_context(workspace.as_path(), provider, vec![]);
    parent.workspace_dir = workspace.clone();
    parent.session_id = "shared-profile-parent-session".into();

    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.clone();
    config.components.juice = false;
    let root = CoreContext::for_test_with_config(DomainSet::full(), config.clone());
    let definitions = Arc::new(AgentDefinitionRegistry::builtins_only());
    let derive_profile = |profile: &str| {
        root.derive_with(
            ContextOverlay::new(config.clone(), DomainSet::full(), ToolGroups::none())
                .profile(profile)
                .definitions(Arc::clone(&definitions)),
        )
    };
    let alice = derive_profile("cancel-alice");
    let bob = derive_profile("cancel-bob");
    assert_eq!(alice.profile(), Some("cancel-alice"));
    assert_eq!(bob.profile(), Some("cancel-bob"));
    assert_eq!(alice.workspace_dir().unwrap(), workspace);
    assert_eq!(bob.workspace_dir().unwrap(), workspace);

    let thread_id = "shared-profile-parent-thread";
    let mut host = OpenHumanRunContext::new()
        .with_core(Some(Arc::clone(&alice)))
        .with_parent(parent);
    host.thread_id = Some(thread_id.into());
    let live = RunContext::<OpenHumanRunContext>::new(
        tinyagents_harness::context::RunConfig::new("profile-cancel-parent").with_thread(thread_id),
        host,
    );
    let detached =
        crate::agent::subagent_host::detached_subagent_parent(&live, "profile-cancel-child")
            .expect("detached child context");
    let accepted = CoreContext::scope(
        Arc::clone(&alice),
        crate::agent::orchestration::tools::SpawnSubagentTool::new()
            .execute_with_live_parent_context(
                json!({
                    "agent_id": "task_manager_agent",
                    "prompt": "wait for an owner-scoped cancellation",
                    "model": "test-model",
                    "task_key": "profile-cancel-canary"
                }),
                None,
                live.data.child(),
                detached,
            ),
    )
    .await
    .expect("spawn tool execution");
    assert!(!accepted.is_error, "{}", accepted.output());
    let task_id = accepted
        .output()
        .split("[async_subagent_ref]\n")
        .nth(1)
        .and_then(|value| value.split("\n[/async_subagent_ref]").next())
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .and_then(|value| value["task_id"].as_str().map(str::to_owned))
        .expect("accepted child includes public task ID");
    tokio::time::timeout(std::time::Duration::from_secs(5), started_rx.recv())
        .await
        .expect("child reached the scripted provider gate")
        .expect("child start notification");
    let alice_ctx = live.data.dispatch_ctx();
    let alice_jobs = crate::core::runtime::slot_for::<
        tinyagents_orchestration::subagent::SubAgentJobRegistry,
    >(&alice_ctx);
    let row_before = super::driver_sessions_for_parent(
        &alice_ctx,
        &workspace,
        "shared-profile-parent-session",
        Some(thread_id),
    )
    .await
    .expect("driver child row readable")
    .into_iter()
    .find(|row| row.current_task_id.as_deref() == Some(task_id.as_str()))
    .expect("durable row belongs to the live child");
    assert_eq!(row_before.status, DurableSubagentStatus::Running);
    assert_eq!(row_before.parent_session, "shared-profile-parent-session");
    assert_eq!(row_before.parent_thread_id.as_deref(), Some(thread_id));
    let lifecycle_key = row_before
        .lifecycle_key
        .clone()
        .expect("persisted child row carries its lifecycle key");
    let alice_owner = crate::agent::tinyagents::tool_registration::subagent_owner_key(
        &workspace,
        "shared-profile-parent-session",
        Some(thread_id),
    );
    let alice_job_id = alice_jobs
        .driver_job_for_key(&lifecycle_key, &alice_owner)
        .expect("Alice owns a live child job for this lifecycle key");
    let alice_job = alice_jobs
        .get(alice_job_id.as_str())
        .expect("real child is registered in Alice's scoped driver registry");
    assert_eq!(alice_job.status, SubAgentJobStatus::Running);
    assert!(
        crate::agent::orchestration::completion_owners::driver_task_owner(&alice_ctx, &task_id,)
            .is_some(),
        "spawn records Alice's exact task owner scope"
    );

    let bob_ctx = DispatchCtx::new(Arc::clone(&bob));
    let bob_job_registry = crate::core::runtime::slot_for::<
        tinyagents_orchestration::subagent::SubAgentJobRegistry,
    >(&bob_ctx);
    assert!(
        bob_job_registry.list().is_empty(),
        "Bob resolves the same workspace/session/thread but uses Bob's scoped registry"
    );
    let foreign_cancel = CoreContext::scope(Arc::clone(&bob), async {
        crate::agent::orchestration::running_subagents::cancel_by_task_with_workspace_reason(
            &bob_ctx,
            &task_id,
            Some(&workspace),
            Some("foreign profile cancellation"),
        )
        .await
    })
    .await
    .expect("foreign profile cancellation lookup");
    assert!(foreign_cancel.is_none(), "Bob cannot cancel Alice's child");
    assert_eq!(
        alice_jobs
            .get(alice_job.id.as_str())
            .expect("Alice's job remains registered")
            .status,
        SubAgentJobStatus::Running,
        "foreign cancellation leaves the owner job running"
    );
    let after_bob = super::driver_sessions_for_parent(
        &alice_ctx,
        &workspace,
        "shared-profile-parent-session",
        Some(thread_id),
    )
    .await
    .expect("driver child row remains readable")
    .into_iter()
    .find(|row| row.subagent_session_id == row_before.subagent_session_id)
    .expect("foreign attempt keeps Alice's durable row");
    assert_eq!(after_bob.status, DurableSubagentStatus::Running);
    assert_eq!(after_bob.current_task_id, row_before.current_task_id);
    assert_eq!(after_bob.latest_error, row_before.latest_error);
    assert_eq!(
        crate::agent::orchestration::background_completions::pending_for_ctx(
            &bob_ctx, &workspace, thread_id,
        )
        .len(),
        0,
        "Bob's rejected attempt exposes no Alice completion"
    );
    assert!(
        crate::agent::orchestration::running_subagents::cancel_by_task_with_workspace_reason(
            &DispatchCtx::unbound(),
            &task_id,
            None,
            None,
        )
        .await
        .expect("unbound SaaS control refusal")
        .is_none(),
        "SaaS cancellation without a profile fails closed"
    );
    let after_unbound = super::driver_sessions_for_parent(
        &alice_ctx,
        &workspace,
        "shared-profile-parent-session",
        Some(thread_id),
    )
    .await
    .expect("unbound attempt leaves driver child row readable")
    .into_iter()
    .find(|row| row.subagent_session_id == row_before.subagent_session_id)
    .expect("unbound attempt keeps Alice's durable row");
    assert_eq!(after_unbound.status, DurableSubagentStatus::Running);
    assert_eq!(after_unbound.current_task_id, row_before.current_task_id);
    assert_eq!(after_unbound.latest_error, row_before.latest_error);
    assert!(
        crate::agent::orchestration::background_completions::pending_for_ctx(
            &alice_ctx, &workspace, thread_id,
        )
        .is_empty(),
        "neither rejected control attempt emits an Alice completion"
    );

    let owner_scope =
        crate::agent::orchestration::completion_owners::driver_task_owner(&alice_ctx, &task_id)
            .expect("Alice's cached owner scope");
    let job_owner = crate::agent::tinyagents::tool_registration::subagent_owner_key(
        &owner_scope.workspace_dir,
        &owner_scope.parent_session,
        owner_scope.parent_thread_id.as_deref(),
    );
    let registered = alice_jobs
        .list_for_parent_scope(
            &job_owner,
            &tinyagents_orchestration::subagent::ParentJobScope::new(
                owner_scope.workspace.clone(),
                owner_scope.parent_thread_id.as_deref().unwrap_or_default(),
            ),
        )
        .expect("Alice's exact driver job scope");
    assert!(
        registered.iter().any(|job| {
            job.extensions
                .get("task_id")
                .and_then(serde_json::Value::as_str)
                == Some(task_id.as_str())
        }),
        "Alice's owner/workspace/thread must list the driver task: session={:?}, cached workspace={:?}, thread={:?}, job ids={:?}",
        owner_scope.parent_session,
        owner_scope.workspace,
        owner_scope.parent_thread_id,
        registered
            .iter()
            .map(|job| job.id.as_str())
            .collect::<Vec<_>>(),
    );

    let owner_cancel = CoreContext::scope(Arc::clone(&alice), async {
        crate::agent::orchestration::running_subagents::cancel_by_task_with_workspace_reason(
            &alice_ctx,
            &task_id,
            Some(&workspace),
            Some("owner requested stop"),
        )
        .await
    })
    .await
    .expect("owner cancellation lookup");
    assert!(owner_cancel.is_some(), "Alice can cancel her own child");
    let terminal = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        alice_jobs.wait_owned(
            alice_job.id.as_str(),
            crate::agent::tinyagents::tool_registration::subagent_owner_key(
                &workspace,
                "shared-profile-parent-session",
                Some(thread_id),
            ),
        ),
    )
    .await
    .expect("rightful cancellation settles the driver job")
    .expect("owner job remains registered");
    assert_eq!(terminal.status, SubAgentJobStatus::Cancelled);
    release.notify_one();
}
