use super::*;

/// A tool context for a run that acts on memory as `identity`.
fn tool_context_acting_as(
    identity: tinymemory_tools::layout::identity::Identity,
) -> tinyagents_harness::tool::ToolExecutionContext {
    let mut host = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    host.memory_identity = Some(identity);
    let run = host.into_tinyagents(tinyagents_harness::context::RunConfig::new("identity-test"));
    tinyagents_harness::tool::ToolExecutionContext::from_run_context(
        &run,
        tinyagents_harness::ids::CallId::new("identity-test-call"),
    )
}

/// The memory tool acts as the identity its run carries, not as whatever task
/// it happens to be polled on: a team member's learnings land in the team's
/// layout under the member's own memory agent id.
#[test]
fn gather_acts_as_the_identity_the_run_carries() {
    let config = Config::default();
    let context = tool_context_acting_as(
        tinymemory_tools::layout::identity::Identity::team_member("acme", "scout"),
    );
    let facts = CallFacts::gather(&config, Some(&context));
    assert_eq!(facts.agent_id.as_deref(), Some("scout"));
    assert_eq!(facts.reach, Reach::subtree("team:acme".parse().unwrap()));
    assert_eq!(facts.namespace.to_string(), "team:acme");

    // A run that carries none, and direct dispatch, act as the root.
    let bare = CallFacts::gather(&config, None);
    assert_eq!(
        bare.agent_id.as_deref(),
        Some(tinymemory_tools::layout::identity::DEFAULT_AGENT_ID)
    );
    assert!(bare.reach.at.is_root());
}
