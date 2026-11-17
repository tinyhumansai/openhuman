//! OpenHuman's path-policy adapter for the TinyAgents `AGENTS.md` loader.

use std::path::Path;

/// Load one instruction file after applying OpenHuman's path-root policy.
pub fn load_agents_md(dir: &Path) -> Option<String> {
    tinyagents_harness::context::load_agents_md(dir, |path, root| {
        crate::security::validate_path_within_root(path, root).is_ok()
    })
}

/// Load the workspace and effective action-directory layers once each.
pub fn load_agents_md_layers(
    workspace_dir: &Path,
    action_dir: &Path,
) -> tinyagents_harness::context::AgentsMdContent {
    tinyagents_harness::context::load_agents_md_layers(
        workspace_dir,
        action_dir,
        true,
        |path, root| crate::security::validate_path_within_root(path, root).is_ok(),
    )
}

#[cfg(test)]
#[path = "agents_md_tests.rs"]
mod tests;
