//! A host that turns security off (`RuntimeBuilder::security`) gets its host
//! tool calls executed, even though the TinySecurity module that would classify
//! them cannot be reached. With security enforced the same call is
//! `[policy-blocked]` (see `security_default.rs`).

mod common;
#[path = "support/security_tool.rs"]
mod scenario;

use openhuman_embed::config::SecurityModeConfig;

#[test]
fn a_runtime_with_security_off_runs_the_tool_the_model_calls() {
    let outcome = scenario::tool_turn(Some(SecurityModeConfig::off()));
    assert!(!outcome.security_enforced);
    assert_eq!(outcome.runs, 1, "tool result: {}", outcome.tool_result);
    assert!(
        !outcome.tool_result.contains("[policy-blocked]"),
        "{}",
        outcome.tool_result
    );
    assert!(
        outcome.tool_result.contains("probe ran"),
        "{}",
        outcome.tool_result
    );
}
