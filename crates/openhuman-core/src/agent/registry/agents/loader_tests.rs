use super::*;
use crate::agent::harness::definition::{ModelSpec, SandboxMode, SubagentEntry, ToolScope};

fn find(id: &str) -> AgentDefinition {
    load_builtins()
        .unwrap()
        .into_iter()
        .find(|d| d.id == id)
        .unwrap_or_else(|| panic!("missing built-in {id}"))
}

#[path = "loader_tests_builtin_registration_tests.rs"]
mod builtin_registration_tests;
#[path = "loader_tests_orchestrator_tier_tests.rs"]
mod orchestrator_tier_tests;
#[path = "loader_tests_specialist_agents_tests.rs"]
mod specialist_agents_tests;
