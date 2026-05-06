//! Built-in [`AgentDefinition`]s.
//!
//! The authoritative list of built-in agents lives in
//! [`crate::openhuman::agent::agents`] — each agent is a subfolder
//! containing `agent.toml` + `prompt.md`. This module is a thin
//! wrapper that loads that set.
//!
//! Custom TOML definitions loaded later by
//! [`super::definition_loader`] override any built-in with the same id.

use super::definition::{AgentDefinition, DefinitionSource};

/// All built-in definitions, in stable order.
///
/// Panics if the baked-in built-in TOML fails to parse. `include_str!`
/// guarantees at compile time that each file exists, but the actual
/// TOML parse happens at runtime; the unit tests in
/// [`crate::openhuman::agent::agents`] verify in CI that every entry in
/// [`crate::openhuman::agent::agents::BUILTINS`] still parses cleanly.
pub fn all() -> Vec<AgentDefinition> {
    crate::openhuman::agent::agents::load_builtins()
        .expect("built-in agent TOML must always parse (see agents/*/agent.toml)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_definitions_present() {
        let defs = all();
        assert_eq!(defs.len(), crate::openhuman::agent::agents::BUILTINS.len());
    }

    #[test]
    fn all_builtin_ids_are_stamped_builtin_source() {
        for def in all() {
            assert_eq!(
                def.source,
                DefinitionSource::Builtin,
                "{} should be Builtin",
                def.id
            );
        }
    }

    #[test]
    fn expected_builtin_ids_are_present() {
        let ids: Vec<String> = all().into_iter().map(|d| d.id).collect();
        for expected in [
            "orchestrator",
            "planner",
            "code_executor",
            "integrations_agent",
            "tool_maker",
            "researcher",
            "critic",
            "archivist",
            "summarizer",
        ] {
            assert!(ids.contains(&expected.to_string()), "missing {expected}");
        }
    }
}
