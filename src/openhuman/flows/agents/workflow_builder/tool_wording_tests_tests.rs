/// `list_agent_profiles`'s own tool description used to discourage
/// `agent_ref` with stale "follow-up"/"for now" wording (issue B37, Gap
/// 1) — pin that it now correctly describes the harness's full tool
/// loop instead.
#[test]
fn list_agent_profiles_tool_description_has_no_stale_followup_language() {
    use crate::openhuman::flows::builder_tools::ListAgentProfilesTool;
    use crate::openhuman::tools::traits::Tool;

    let description = ListAgentProfilesTool::new().description().to_string();

    for banned in ["is a follow-up", "for now"] {
        assert!(
            !description.contains(banned),
            "list_agent_profiles description must not carry the stale \
             phrasing `{banned}` — an agent_ref step already gets the \
             selected specialist's full tool loop"
        );
    }
    assert!(
        description.contains("tool loop"),
        "list_agent_profiles description must describe agent_ref as running \
         the specialist's full tool loop"
    );
}
