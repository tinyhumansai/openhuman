//! Where the tool-policy boundary lands relative to the persona (#5704).
//!
//! Extracted from `context.rs` — the Rust layout gate requires test modules to
//! live in a sibling `*_tests.rs` file rather than inline.

use super::append_tool_policy_boundary;

const PERSONA: &str = "You are the archetype.\nMore persona.";
const BOUNDARY: &str = "## Tool Policy Boundary\n- Agent: alpha";

#[test]
fn the_boundary_goes_after_the_prompt_body() {
    let out = append_tool_policy_boundary(PERSONA.into(), Some(BOUNDARY.into()));
    let body_at = out.find("You are the archetype.").expect("body present");
    let boundary_at = out
        .find("## Tool Policy Boundary")
        .expect("boundary present");
    assert!(
        body_at < boundary_at,
        "the session-scoped block must not precede the stable prompt (#5704):\n{out}"
    );
}

#[test]
fn the_persona_stays_the_opening_line() {
    let out = append_tool_policy_boundary(PERSONA.into(), Some(BOUNDARY.into()));
    assert_eq!(
        out.lines().next(),
        Some("You are the archetype."),
        "prepending replaced every agent's first line with a constant heading"
    );
}

#[test]
fn two_agents_share_the_whole_prompt_body_as_a_common_prefix() {
    // The point of appending: the varying part is last, so everything the
    // two turns have in common is a shared leading prefix the backend can
    // reuse. Prepending moved the first diverging byte to offset 0.
    let alpha = append_tool_policy_boundary(
        PERSONA.into(),
        Some("## Tool Policy Boundary\n- Agent: alpha".into()),
    );
    let beta = append_tool_policy_boundary(
        PERSONA.into(),
        Some("## Tool Policy Boundary\n- Agent: beta".into()),
    );
    let shared = alpha
        .bytes()
        .zip(beta.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    assert!(
        shared >= PERSONA.len(),
        "the shared prefix ({shared} bytes) must cover the whole stable body ({} bytes)",
        PERSONA.len()
    );
}

#[test]
fn no_boundary_leaves_the_prompt_untouched() {
    let out = append_tool_policy_boundary(PERSONA.into(), None);
    assert_eq!(out, PERSONA);
}
