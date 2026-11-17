use super::*;

#[tokio::test]
async fn generic_spawn_jobs_are_owned_by_their_workspace() {
    use crate::agent::orchestration::subagent_sessions::SubagentSessionStore;
    use crate::agent::tinyagents::host::OpenHumanRunContext;
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::tools::toolpacks::ToolGroups;
    use tinyagents_harness::context::{RunConfig, RunContext};
    use tinyagents_orchestration::subagent::SubAgentJobRegistry;

    const PROMPT_A: &str = "workspace-owner-child-a-canary";
    const PROMPT_B: &str = "workspace-owner-child-b-canary";

    let scratch = tempfile::tempdir().expect("workspace owner isolation fixture");
    let workspace_a = scratch.path().join("workspace-a");
    let workspace_b = scratch.path().join("workspace-b");
    std::fs::create_dir_all(&workspace_a).expect("create workspace A");
    std::fs::create_dir_all(&workspace_b).expect("create workspace B");

    let model = Arc::new(ScriptedModel::new(vec![
        (PROMPT_A, "workspace-a-child-result"),
        (PROMPT_B, "workspace-b-child-result"),
    ]));
    let mut base_config = crate::config::Config::default();
    base_config.workspace_dir = workspace_a.clone();
    base_config.components.juice = false;
    let base = CoreContext::for_test_with_config(DomainSet::full(), base_config.clone());
    let definitions = Arc::new(AgentDefinitionRegistry::builtins_only());

    let make_live = |workspace: &Path, core: &Arc<CoreContext>, run_id: &str| {
        let mut parent = parent_context(workspace, model.clone(), vec![]);
        parent.workspace_dir = workspace.to_path_buf();
        parent.session_id = "shared-parent-session".into();
        let mut host = OpenHumanRunContext::new()
            .with_core(Some(Arc::clone(core)))
            .with_parent(parent);
        host.thread_id = Some("shared-parent-thread".into());
        RunContext::new(
            RunConfig::new(run_id).with_thread("shared-parent-thread"),
            host,
        )
    };

    let make_core = |workspace: &Path| {
        let mut config = base_config.clone();
        config.workspace_dir = workspace.to_path_buf();
        base.derive_with(
            ContextOverlay::new(config, DomainSet::full(), ToolGroups::none())
                .definitions(Arc::clone(&definitions)),
        )
    };
    let core_a = make_core(&workspace_a);
    let core_b = make_core(&workspace_b);
    let live_a = make_live(&workspace_a, &core_a, "workspace-owner-parent-a");
    let live_b = make_live(&workspace_b, &core_b, "workspace-owner-parent-b");

    async fn spawn(live: &RunContext<OpenHumanRunContext>, prompt: &str) -> String {
        let accepted = crate::core::runtime::CoreContext::scope(
            live.data.dispatch_ctx().resolved().expect("live core context"),
            Box::pin(SpawnSubagentTool::new().execute_with_live_parent_context(
                json!({
                    "agent_id": "task_manager_agent",
                    "prompt": prompt,
                    "model": "test-model",
                    "task_key": "same task key in separate workspaces"
                }),
                None,
                live.data.child(),
                crate::agent::subagent_host::detached_subagent_parent(live, "workspace-child")
                    .expect("detached parent"),
            )),
        )
        .await
        .expect("generic spawn execution");
        assert!(!accepted.is_error, "{}", accepted.output());
        accepted
            .output()
            .split("[async_subagent_ref]\n")
            .nth(1)
            .and_then(|value| value.split("\n[/async_subagent_ref]").next())
            .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
            .and_then(|value| value["task_id"].as_str().map(str::to_owned))
            .expect("accepted generic spawn contains its public task id")
    }

    let task_a = spawn(&live_a, PROMPT_A).await;
    let task_b = spawn(&live_b, PROMPT_B).await;
    assert_ne!(task_a, task_b, "workspace-scoped tasks remain distinct");

    let registry_a = CoreContext::sync_scope(Arc::clone(&core_a), || {
        crate::core::runtime::slot_for::<SubAgentJobRegistry>(&live_a.data.dispatch_ctx())
    });
    let registry_b = CoreContext::sync_scope(Arc::clone(&core_b), || {
        crate::core::runtime::slot_for::<SubAgentJobRegistry>(&live_b.data.dispatch_ctx())
    });
    assert!(
        Arc::ptr_eq(&registry_a, &registry_b),
        "the fixture exercises one shared generic job registry"
    );

    let load_row = |core: &Arc<CoreContext>, workspace: &Path, task_id: &str| {
        CoreContext::sync_scope(Arc::clone(core), || {
            SubagentSessionStore::new(workspace.to_path_buf())
                .load()
                .expect("read generic child session")
                .into_iter()
                .find(|row| row.current_task_id.as_deref() == Some(task_id))
                .expect("generic child session is persisted")
        })
    };
    let row_a = load_row(&core_a, &workspace_a, &task_a);
    let row_b = load_row(&core_b, &workspace_b, &task_b);
    let key_a = row_a.lifecycle_key.expect("A row has a lifecycle key");
    let key_b = row_b.lifecycle_key.expect("B row has a lifecycle key");
    let owner_a = crate::agent::tinyagents::tool_registration::subagent_owner_key(
        &workspace_a,
        "shared-parent-session",
        Some("shared-parent-thread"),
    );
    let owner_b = crate::agent::tinyagents::tool_registration::subagent_owner_key(
        &workspace_b,
        "shared-parent-session",
        Some("shared-parent-thread"),
    );
    let job_a = registry_a
        .driver_job_for_key(&key_a, &owner_a)
        .expect("workspace A owns its generic child job");
    let job_b = registry_b
        .driver_job_for_key(&key_b, &owner_b)
        .expect("workspace B owns its generic child job");
    assert_ne!(job_a, job_b);
    assert!(
        registry_a.driver_job_for_key(&key_a, &owner_b).is_none(),
        "workspace B cannot resolve workspace A's generic child job"
    );
    assert!(
        registry_b.driver_job_for_key(&key_b, &owner_a).is_none(),
        "workspace A cannot resolve workspace B's generic child job"
    );
    for (registry, job_id, owner, expected_output) in [
        (&registry_a, job_a, &owner_a, "workspace-a-child-result"),
        (&registry_b, job_b, &owner_b, "workspace-b-child-result"),
    ] {
        let terminal = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            registry.wait_owned(job_id.as_str(), owner),
        )
        .await
        .expect("generic child reaches a terminal state")
        .expect("owner can await its generic child");
        assert_eq!(terminal.output.as_deref(), Some(expected_output));
    }
}
