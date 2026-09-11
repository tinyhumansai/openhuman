use super::*;
use crate::openhuman::agent::harness::definition::{
    ModelSpec, SandboxMode, SubagentEntry, ToolScope, TriggerMemoryAgent,
};
use crate::openhuman::inference::tokenjuice::AgentTokenjuiceCompression;

fn find(id: &str) -> AgentDefinition {
    load_builtins()
        .unwrap()
        .into_iter()
        .find(|d| d.id == id)
        .unwrap_or_else(|| panic!("missing built-in {id}"))
}

#[path = "loader_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "loader_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "loader_tests_part_03_tests.rs"]
mod part_03_tests;
