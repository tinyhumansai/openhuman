use super::*;

#[test]
fn profile_capabilities_deny_the_registered_spawn_tool() {
    let capabilities = ProfileCapabilities::new(None, None, None, HashMap::new(), Vec::new(), None);

    // `continue_subagent` is retired (tinyagents owns resume); only the
    // registered spawn tools remain to deny.
    for name in tinyagents_definition::SPAWN_TOOL_NAMES.iter().copied() {
        assert!(
            !capabilities.admits_capability(name),
            "profiles must not admit spawn tool {name}"
        );
    }
}
