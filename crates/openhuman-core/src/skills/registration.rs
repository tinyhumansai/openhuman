//! Registration of skill discovery, management, and workflow metadata tools.

use super::catalog::tools::{
    SkillRegistryBrowseTool, SkillRegistryInstallTool, SkillRegistrySearchTool,
    SkillRegistrySourcesTool, SkillRegistryUninstallTool,
};
use super::runtime::tools::SkillRuntimeResolveRuntimesTool;
use super::search::SkillSearchTool;
use super::tools::{
    WorkflowCreateTool, WorkflowDescribeTool, WorkflowInstallFromUrlTool, WorkflowListTool,
    WorkflowReadResourceTool, WorkflowReadRunLogTool, WorkflowRecentRunsTool,
    WorkflowUninstallTool,
};
use crate::config::Config;
use std::sync::Arc;
use tinytools::Tool;

/// Build the skill tool family in its persisted catalogue order.
pub(crate) fn tools(config: Arc<Config>) -> Vec<Box<dyn Tool>> {
    vec![
        // Skills metadata tools. `skill_run` is already exposed by RunSkillTool
        // above, so it is not duplicated. Reads ship default-ON; the
        // create/install/uninstall mutators ship default-OFF via
        // `tools::user_filter` (install also fetches remote content).
        Box::new(WorkflowListTool::new(config.clone())),
        Box::new(WorkflowDescribeTool::new(config.clone())),
        Box::new(SkillSearchTool::new(config.clone())),
        // Skill registry tools — browse/search/install from remote registries.
        // Browse and search are read-only (default-ON); install is a write
        // operation (fetches remote content and writes to disk).
        Box::new(SkillRegistryBrowseTool),
        Box::new(SkillRegistrySearchTool),
        Box::new(SkillRegistryInstallTool::new(config.clone())),
        Box::new(SkillRegistrySourcesTool),
        Box::new(SkillRegistryUninstallTool),
        // Skill runtime probes — resolve the reusable Node/Python runtimes
        // that skill execution relies on before a script-backed skill runs.
        Box::new(SkillRuntimeResolveRuntimesTool::new(config.clone())),
        Box::new(WorkflowReadResourceTool::new(config.clone())),
        Box::new(WorkflowRecentRunsTool::new(config.clone())),
        Box::new(WorkflowReadRunLogTool::new(config.clone())),
        Box::new(WorkflowCreateTool::new(config.clone())),
        Box::new(WorkflowInstallFromUrlTool::new(config.clone())),
        Box::new(WorkflowUninstallTool),
    ]
}
