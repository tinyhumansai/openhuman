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

/// The scheduled-actions lift itself stays on the cron root: a child starts
/// without it. What a child does carry is the fact that its tree's root was a
/// scheduled-actions run, so the sub-agent runner can decide whether to
/// re-grant it to one direct child.
#[test]
fn a_child_keeps_the_scheduled_actions_root_mark_but_not_the_lift() {
    let mut root = OpenHumanRunContext::new();
    root.scheduled_actions = true;
    root.scheduled_actions_root = true;

    let child = root.child();
    assert!(!child.scheduled_actions, "the lift is never inherited");
    assert!(child.scheduled_actions_root, "the root mark is inherited");
    let grandchild = child.child();
    assert!(!grandchild.scheduled_actions);
    assert!(grandchild.scheduled_actions_root);
    let detached = root.detached_child();
    assert!(!detached.scheduled_actions);
    assert!(detached.scheduled_actions_root);

    assert!(!OpenHumanRunContext::new().child().scheduled_actions_root);
}

/// Pins what a sub-agent's run context carries for the files-only jail: the
/// parent's value, whatever the child's own definition says. `child()` clones
/// it and `for_subagent` does not reset it.
#[test]
fn a_subagent_inherits_the_parents_files_only_jail_not_its_own_definitions() {
    let mut files_only = crate::agent_registry::host_definition::AgentDefinition {
        id: "jailed".into(),
        ..Default::default()
    };
    files_only.extensions.sandbox_filesystem_only = true;
    let plain = crate::agent_registry::host_definition::AgentDefinition {
        id: "plain".into(),
        ..Default::default()
    };

    let mut parent = OpenHumanRunContext::new();
    parent.sandbox_filesystem_only = true;
    assert!(
        parent
            .child()
            .for_subagent(&plain, None)
            .sandbox_filesystem_only,
        "a child of a files-only parent keeps it although its definition does not ask"
    );

    parent.sandbox_filesystem_only = false;
    assert!(
        !parent
            .child()
            .for_subagent(&files_only, None)
            .sandbox_filesystem_only,
        "a child's own opt-in is not applied to its run"
    );
}

fn definition(id: &str, browser_view_only: bool) -> crate::agent_registry::host_definition::AgentDefinition {
    let mut definition = crate::agent_registry::host_definition::AgentDefinition {
        id: id.into(),
        ..Default::default()
    };
    definition.extensions.browser_view_only = browser_view_only;
    definition
}

/// What a tool in `context`'s run reads for the view-only browser.
fn tool_sees_browser_view_only(context: OpenHumanRunContext) -> bool {
    let run = context.into_tinyagents(tinyagents_harness::context::RunConfig::new("view-only"));
    let call = tinyagents_harness::tool::ToolExecutionContext::from_run_context(
        &run,
        tinyagents_harness::ids::CallId::new("view-only-call"),
    );
    browser_view_only_for_tool(Some(&call))
}

/// A session's root run takes the view-only browser from its own definition,
/// and a tool in that run reads it.
#[test]
fn a_root_run_is_browser_view_only_when_its_definition_says_so() {
    let mut root = OpenHumanRunContext::new();
    root.tighten_browser_view_only(Some(&definition("reader", true)));
    assert!(root.browser_view_only);
    assert!(tool_sees_browser_view_only(root));

    let mut plain = OpenHumanRunContext::new();
    plain.tighten_browser_view_only(Some(&definition("plain", false)));
    plain.tighten_browser_view_only(None);
    assert!(!plain.browser_view_only);
    assert!(!tool_sees_browser_view_only(plain));
}

/// Unlike the files-only jail, the view-only browser only ever tightens down
/// a run tree: a child is view-only when its parent is, whatever its own
/// definition says, and also when only its own definition asks for it.
#[test]
fn a_subagent_is_browser_view_only_when_its_parent_or_its_own_definition_is() {
    let mut view_only_parent = OpenHumanRunContext::new();
    view_only_parent.browser_view_only = true;
    let child = view_only_parent
        .child()
        .for_subagent(&definition("plain", false), None);
    assert!(child.browser_view_only, "a child cannot clear its parent's");
    assert!(tool_sees_browser_view_only(child.clone()));
    assert!(
        child.child().for_subagent(&definition("plain", false), None).browser_view_only,
        "nor can a grandchild"
    );
    assert!(view_only_parent.detached_child().browser_view_only);

    let parent = OpenHumanRunContext::new();
    let opted = parent
        .child()
        .for_subagent(&definition("reader", true), None);
    assert!(opted.browser_view_only, "a child's own opt-in counts");
    assert!(tool_sees_browser_view_only(opted));

    let neither = parent
        .child()
        .for_subagent(&definition("plain", false), None);
    assert!(!neither.browser_view_only);
    assert!(!tool_sees_browser_view_only(neither));
}

/// A root run whose carrier already came in view-only stays so even when its
/// own definition does not ask for it.
#[test]
fn a_definition_without_the_flag_never_clears_a_view_only_run() {
    let mut context = OpenHumanRunContext::new();
    context.browser_view_only = true;
    context.tighten_browser_view_only(Some(&definition("plain", false)));
    context.tighten_browser_view_only(None);
    assert!(context.browser_view_only);
}
