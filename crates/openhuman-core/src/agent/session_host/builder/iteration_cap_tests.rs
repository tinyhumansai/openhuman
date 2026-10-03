use super::*;
use crate::agent::harness::definition::IterationPolicy;

fn def_with_cap(max_iterations: usize) -> AgentDefinition {
    let mut def: AgentDefinition = toml::from_str(
        r#"
id = "cap_probe"
when_to_use = "tests the iteration cap"
"#,
    )
    .expect("minimal definition parses");
    def.max_iterations = max_iterations;
    def
}

fn agent_config(global: usize, override_cap: Option<usize>) -> AgentConfig {
    AgentConfig {
        max_tool_iterations: global,
        max_tool_iterations_override: override_cap,
        ..AgentConfig::default()
    }
}

/// No definition: the global cap is all there is.
#[test]
fn without_a_definition_the_global_cap_applies() {
    assert_eq!(
        resolve_max_tool_iterations(&agent_config(10, None), None),
        10
    );
}

/// The documented #4868 behaviour stays: a definition's cap replaces the
/// global default, in both directions.
#[test]
fn a_definition_cap_replaces_the_global_default() {
    let def = def_with_cap(50);
    assert_eq!(
        resolve_max_tool_iterations(&agent_config(10, None), Some(&def)),
        50
    );
    let strict = def_with_cap(5);
    assert_eq!(
        resolve_max_tool_iterations(&agent_config(10, None), Some(&strict)),
        5
    );
}

/// Extended agents still get the harness-wide extended ceiling.
#[test]
fn an_extended_definition_keeps_its_extended_ceiling() {
    let mut def = def_with_cap(10);
    def.iteration_policy = IterationPolicy::Extended;
    assert_eq!(
        resolve_max_tool_iterations(&agent_config(10, None), Some(&def)),
        def.effective_max_iterations()
    );
}

/// #6958: the bench had to sed `agent.toml` because no setting could lift the
/// orchestrator's definition cap. An explicit override wins over it.
#[test]
fn an_explicit_override_raises_the_definition_cap() {
    let def = def_with_cap(50);
    assert_eq!(
        resolve_max_tool_iterations(&agent_config(10, Some(300)), Some(&def)),
        300
    );
}

/// The override is a cap, not a floor: an operator may also tighten it.
#[test]
fn an_explicit_override_can_lower_the_definition_cap() {
    let def = def_with_cap(200);
    assert_eq!(
        resolve_max_tool_iterations(&agent_config(10, Some(20)), Some(&def)),
        20
    );
}

/// The override applies to definition-less turns too.
#[test]
fn an_explicit_override_wins_without_a_definition() {
    assert_eq!(
        resolve_max_tool_iterations(&agent_config(10, Some(75)), None),
        75
    );
}

/// Zero would make the very first model call the concluding one; treat it as
/// unset rather than as a cap that forbids every tool.
#[test]
fn a_zero_override_is_ignored() {
    let def = def_with_cap(50);
    assert_eq!(
        resolve_max_tool_iterations(&agent_config(10, Some(0)), Some(&def)),
        50
    );
}
