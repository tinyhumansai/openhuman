//! A runtime that leaves `[security]` unset. In a build with the
//! `security-module` feature security is enforced and fails closed: the module
//! cannot be reached, so the model's tool call is `[policy-blocked]` and the tool
//! never runs. In a build without the feature (for example
//! `--no-default-features --features storage-sqlite,modules,documents,secret-file,legacy-import,testkit`)
//! security is disabled with no switch at all, and the tool runs.

mod common;
#[path = "support/security_tool.rs"]
mod scenario;

#[test]
fn a_runtime_without_a_security_switch_follows_the_build() {
    let outcome = scenario::tool_turn(None);
    if outcome.security_enforced {
        assert_eq!(outcome.runs, 0, "an unclassifiable call never runs");
        assert!(
            outcome.tool_result.contains("[policy-blocked]"),
            "{}",
            outcome.tool_result
        );
    } else {
        assert_eq!(outcome.runs, 1, "tool result: {}", outcome.tool_result);
        assert!(
            outcome.tool_result.contains("probe ran"),
            "{}",
            outcome.tool_result
        );
    }
}
