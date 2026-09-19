use super::*;

// ── Memory-write instruction (#6048) ─────────────────────────────────────────
//
// "Please remember …" was answered with "saved" and zero tool calls. The rule
// that a request to remember must become a write before the reply is keyed on
// the write tools being offered — never on `learning.enabled`, which the
// default install leaves off.

#[tokio::test]
async fn memory_write_instruction_is_present_with_learning_disabled() {
    use crate::agent::prompts::LearnedContextData;
    use crate::agent::session_host::types::OpenHumanSessionHost;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert!(
        !config.learning.enabled,
        "the default install has learning off; the write rule must not depend on it"
    );
    let orchestrator = builtin_def("orchestrator");
    let agent = OpenHumanSessionHost::build_session_agent_inner(
        &config,
        "orchestrator",
        Some(&orchestrator),
        false,
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
async fn memory_write_instruction_names_only_the_write_tool_a_scoped_agent_holds() {
    use crate::agent::prompts::LearnedContextData;
    use crate::agent::session_host::types::OpenHumanSessionHost;

    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let archivist = builtin_def("archivist");
    let agent = OpenHumanSessionHost::build_session_agent_inner(
        &config,
        "archivist",
        Some(&archivist),
        false,
    )
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
