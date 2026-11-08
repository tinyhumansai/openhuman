use super::*;

/// The memory identity is explicit on the run: a child inherits it (the
/// sub-agent runner then narrows it with `child(<definition id>)`), and a host
/// environment that carries one — the team runtime's member — applies it, while
/// an environment without one leaves the run's own alone.
#[test]
fn the_memory_identity_rides_the_run_and_its_children() {
    let mut context = OpenHumanRunContext::new();
    assert_eq!(context.memory_identity, None);

    let env = crate::agent::turn_env::HostTurnEnv {
        memory_identity: Some(tinymemory_tools::layout::identity::Identity::team_member(
            "acme", "lead",
        )),
        ..Default::default()
    };
    context = context.with_host_env(&env);
    assert_eq!(
        context.memory_identity,
        Some(tinymemory_tools::layout::identity::Identity::team_member(
            "acme", "lead"
        ))
    );

    let child = context.child();
    assert_eq!(child.memory_identity, context.memory_identity);
    assert_eq!(
        child.memory_identity.unwrap().child("scout"),
        tinymemory_tools::layout::identity::Identity::team_member("acme", "scout")
    );

    let unchanged = context.with_host_env(&crate::agent::turn_env::HostTurnEnv::default());
    assert_eq!(
        unchanged.memory_identity,
        Some(tinymemory_tools::layout::identity::Identity::team_member(
            "acme", "lead"
        ))
    );
}

fn tool_context_for(run: OpenHumanRunContext) -> tinyagents_harness::tool::ToolExecutionContext {
    let run = run.into_tinyagents(tinyagents_harness::context::RunConfig::new("core-carrier"));
    tinyagents_harness::tool::ToolExecutionContext::from_run_context(
        &run,
        tinyagents_harness::ids::CallId::new("core-carrier-call"),
    )
}

#[tokio::test]
async fn a_tool_reads_the_runs_core_not_the_ambient_scope() {
    use crate::core::runtime::{CoreContext, DomainSet};

    let runs = CoreContext::for_test(DomainSet::full(), None);
    let ambient = CoreContext::for_test(DomainSet::full(), None);
    let context = tool_context_for(OpenHumanRunContext::new().with_core(Some(runs.clone())));

    CoreContext::scope(ambient.clone(), async {
        let ctx = dispatch_ctx_for_tool(Some(&context));
        assert!(Arc::ptr_eq(ctx.core().expect("bound"), &runs));
        assert!(!Arc::ptr_eq(ctx.core().expect("bound"), &ambient));
    })
    .await;
}

#[tokio::test]
async fn concurrent_runs_keep_their_own_core_across_awaits_and_spawns() {
    use crate::core::runtime::{CoreContext, DomainSet};

    let a = CoreContext::for_test(DomainSet::full(), None);
    let b = CoreContext::for_test(DomainSet::full(), None);
    let ctx_a = tool_context_for(OpenHumanRunContext::new().with_core(Some(a.clone())));
    let ctx_b = tool_context_for(OpenHumanRunContext::new().with_core(Some(b.clone())));

    let read = |context: tinyagents_harness::tool::ToolExecutionContext| async move {
        tokio::task::yield_now().await;
        // A plain spawn loses every task-local; the carrier does not.
        tokio::spawn(async move { dispatch_ctx_for_tool(Some(&context)).into_core() })
            .await
            .unwrap()
    };
    let (got_a, got_b) = tokio::join!(read(ctx_a), read(ctx_b));
    assert!(Arc::ptr_eq(&got_a.expect("a bound"), &a));
    assert!(Arc::ptr_eq(&got_b.expect("b bound"), &b));
}

#[test]
fn the_core_rides_the_host_env_and_survives_child_and_detached_child() {
    use crate::core::runtime::{CoreContext, DomainSet};

    let core = CoreContext::for_test(DomainSet::full(), None);
    let env = crate::agent::turn_env::HostTurnEnv {
        core: Some(Arc::downgrade(&core)),
        ..Default::default()
    };
    let run = OpenHumanRunContext::new().with_host_env(&env);
    let bound = |run: &OpenHumanRunContext| run.core.as_ref().and_then(std::sync::Weak::upgrade);
    assert!(Arc::ptr_eq(&bound(&run).unwrap(), &core));
    assert!(Arc::ptr_eq(&bound(&run.child()).unwrap(), &core));
    assert!(Arc::ptr_eq(&bound(&run.detached_child()).unwrap(), &core));
    // An env with no core leaves the run's own in place.
    let kept = run.with_host_env(&crate::agent::turn_env::HostTurnEnv::default());
    assert!(Arc::ptr_eq(&bound(&kept).unwrap(), &core));
}

#[test]
fn a_run_kept_past_its_turn_does_not_pin_the_tenant() {
    use crate::core::runtime::{CoreContext, DomainSet};

    let core = CoreContext::for_test(DomainSet::full(), None);
    let run = OpenHumanRunContext::new().with_core(Some(core.clone()));
    let env = crate::agent::turn_env::HostTurnEnv {
        core: Some(Arc::downgrade(&core)),
        ..Default::default()
    };
    let _kept_run = run.child();
    let _kept_env = env.clone();
    // Only the handle in this test holds the context: sessions and runs that
    // outlive a turn reference it weakly.
    assert!(!core.tenant_in_use());
}

#[test]
fn the_turns_still_in_the_prompt_reach_the_runs_tools() {
    let config = crate::config::Config::default();
    let mut host = OpenHumanRunContext::new();
    host.memory_turn = Some(std::sync::Arc::new(
        crate::memory::lifecycle::hooks::MemoryTurn {
            identity: tinymemory_tools::layout::identity::Identity::agent("a")
                .resolve(&crate::memory::scope::identity_pins(&config)),
            config: std::sync::Arc::new(config),
            thread_id: "thread-long".into(),
            pack: None,
            in_prompt_from: std::sync::Arc::new(
                tinymemory_tools::lifecycle::window::InPromptFrom::new(12, 20),
            ),
            core: None,
        },
    ));
    let run = host.into_tinyagents(tinyagents_harness::context::RunConfig::new("memory-window"));
    let call = tinyagents_harness::tool::ToolExecutionContext::from_run_context(
        &run,
        tinyagents_harness::ids::CallId::new("memory-window-call"),
    );
    assert_eq!(
        memory_in_prompt_for_tool(Some(&call)),
        Some(tinymemory_api::ThreadWindow {
            thread_id: "thread-long".into(),
            from_turn: 12,
        })
    );

    // A run without the turn's memory (a sub-agent's, a direct dispatch)
    // knows no window.
    let bare = OpenHumanRunContext::new()
        .into_tinyagents(tinyagents_harness::context::RunConfig::new("no-memory"));
    let call = tinyagents_harness::tool::ToolExecutionContext::from_run_context(
        &bare,
        tinyagents_harness::ids::CallId::new("no-memory-call"),
    );
    assert_eq!(memory_in_prompt_for_tool(Some(&call)), None);
    assert_eq!(memory_in_prompt_for_tool(None), None);
}

#[test]
fn session_run_options_carry_expiry_reasoning_and_discovery_onto_a_fresh_config() {
    use tinyagents_harness::context::RunConfig;
    use tinyagents_harness::reasoning_replay::ReasoningReplay;
    use tinyagents_harness::result_expiry::ExpiryOverrides;
    use tinyagents_harness::tool::discover::DiscoveryOverrides;

    let session = RunConfig::new("session")
        .with_result_expiry_overrides(ExpiryOverrides {
            enabled: Some(true),
            max_age_turns: Some(2),
            exempt_tools: Some(vec!["tool_search".into()]),
        })
        .with_replay_reasoning(ReasoningReplay::CurrentTurn)
        .with_discovery_overrides(DiscoveryOverrides {
            collapse_search_results: Some(true),
            ..DiscoveryOverrides::default()
        });
    let options = SessionRunOptions::of(&session);

    let carried = options.apply_to(RunConfig::new("turn"));
    assert_eq!(carried.result_expiry, session.result_expiry);
    assert_eq!(carried.replay_reasoning, Some(ReasoningReplay::CurrentTurn));
    assert_eq!(carried.discovery, session.discovery);
    assert_eq!(carried.run_id.as_str(), "turn");
}

#[test]
fn empty_session_run_options_leave_a_config_that_carries_its_own_untouched() {
    use tinyagents_harness::context::RunConfig;
    use tinyagents_harness::reasoning_replay::ReasoningReplay;

    let own = RunConfig::new("child").with_replay_reasoning(ReasoningReplay::CurrentTurn);
    let kept = SessionRunOptions::default().apply_to(own);
    assert_eq!(kept.replay_reasoning, Some(ReasoningReplay::CurrentTurn));
    assert!(OpenHumanRunContext::new()
        .child()
        .session_run_options
        .is_empty());
}
