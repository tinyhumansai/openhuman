//! Tests for the builder module — dedup_visible_tool_specs and related logic.

use super::{
    dedup_visible_tool_specs, ensure_recovery_tool_visible, should_synthesize_delegation_tools,
};
use crate::openhuman::tools::ToolSpec;
use serde_json::json;

fn spec(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: format!("description for {name}"),
        parameters: json!({}),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Issue #4868 — `build_session_agent_inner` must resolve the iteration cap
// from the target `AgentDefinition`'s `effective_max_iterations()`, not the
// global `config.agent.max_tool_iterations` default. These tests drive
// `build_session_agent_inner` directly with a hand-picked `target_def`
// (`pub(crate)` for exactly this purpose), independent of the process-global
// `AgentDefinitionRegistry` singleton's init-once state.
// ─────────────────────────────────────────────────────────────────────────────

fn test_config(tmp: &tempfile::TempDir) -> crate::openhuman::config::Config {
    let config = crate::openhuman::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..crate::openhuman::config::Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

/// Look up a real built-in `AgentDefinition` by id — loaded fresh from the
/// bundled TOML files, entirely independent of the global registry
/// singleton (so tests can't be poisoned by another test's
/// `AgentDefinitionRegistry::init_global*` call, and can't poison later ones).
fn builtin_def(id: &str) -> crate::openhuman::agent::harness::definition::AgentDefinition {
    crate::openhuman::agent::registry::agents::load_builtins()
        .unwrap()
        .into_iter()
        .find(|def| def.id == id)
        .unwrap_or_else(|| panic!("builtin agent definition not found: {id}"))
}

// ── Finding #1 (Codex): dedicated memory subtree on the ordinary session path ─

/// Build a non-default profile with the given id + dedicated-memory flag.
fn custom_profile(
    id: &str,
    dedicated_memory: bool,
) -> crate::openhuman::agent::profiles::AgentProfile {
    let mut profile = crate::openhuman::agent::profiles::store::built_in_default_profile();
    profile.id = id.to_string();
    profile.name = id.to_string();
    profile.built_in = false;
    profile.is_master = false;
    profile.memory_dir_suffix = None;
    profile.dedicated_memory = dedicated_memory;
    profile
}

#[path = "builder_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "builder_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "builder_tests_part_03_tests.rs"]
mod part_03_tests;
