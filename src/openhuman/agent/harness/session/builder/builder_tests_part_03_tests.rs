use super::*;

// ── Memory-write instruction (#6048) ─────────────────────────────────────────
//
// "Please remember …" was answered with "saved" and zero tool calls. The rule
// that a request to remember must become a write before the reply is keyed on
// the write tools being offered — never on `learning.enabled`, which the
// default install leaves off.

#[tokio::test]
async fn memory_write_instruction_is_present_with_learning_disabled() {
    use crate::openhuman::agent::context::prompt::LearnedContextData;
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert!(
        !config.learning.enabled,
        "the default install has learning off; the write rule must not depend on it"
    );
    let orchestrator = builtin_def("orchestrator");
    let agent = Agent::build_session_agent_inner(
        &config,
        "orchestrator",
        Some(&orchestrator),
        None,
        false,
        None,
    )
    .expect("build session");

    let prompt = agent
        .build_system_prompt(LearnedContextData::default())
        .expect("build_system_prompt");
    assert!(
        prompt.contains("## Remembering"),
        "a session offering memory_store / save_preference must carry the write rule"
    );
}

#[tokio::test]
async fn memory_write_instruction_is_absent_when_no_write_tool_is_visible() {
    use crate::openhuman::agent::context::prompt::LearnedContextData;
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let orchestrator = builtin_def("orchestrator");
    let mut profile = crate::openhuman::agent::profiles::store::built_in_default_profile();
    profile.id = "alice".to_string();
    profile.built_in = false;
    profile.allowed_tools = Some(vec!["file_read".to_string()]);

    let agent = Agent::build_session_agent_inner(
        &config,
        "orchestrator",
        Some(&orchestrator),
        None,
        false,
        Some(&profile),
    )
    .expect("build profile-scoped session");

    let prompt = agent
        .build_system_prompt(LearnedContextData::default())
        .expect("build_system_prompt");
    assert!(
        !prompt.contains("## Remembering"),
        "a rule about tools the model cannot see would only teach it to apologise"
    );
}

/// The live case both reviewers pointed at: `archivist`'s `[tools] named`
/// scope carries `memory_store` but not `save_preference` (so do
/// `trigger_reactor` and `workflow_builder`). Naming both routes there would
/// mandate a tool the session cannot call — the failure the visibility gate
/// exists to prevent.
#[tokio::test]
async fn memory_write_instruction_names_only_the_write_tool_a_scoped_agent_holds() {
    use crate::openhuman::agent::context::prompt::LearnedContextData;
    use crate::openhuman::agent::harness::session::types::Agent;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let archivist = builtin_def("archivist");
    let agent =
        Agent::build_session_agent_inner(&config, "archivist", Some(&archivist), None, false, None)
            .expect("build archivist session");

    let prompt = agent
        .build_system_prompt(LearnedContextData::default())
        .expect("build_system_prompt");
    let block = prompt
        .split("## Remembering")
        .nth(1)
        .expect("the archivist offers memory_store, so the write rule must be present");
    // Read only this section: other sections may mention other tools.
    let block = block.split("\n## ").next().unwrap_or(block);
    assert!(
        block.contains("`memory_store`"),
        "the offered write tool must be named: {block}"
    );
    assert!(
        !block.contains("`save_preference`"),
        "a scope without save_preference must not be routed to it: {block}"
    );
}
