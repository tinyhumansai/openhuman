//! OpenHuman discovery context around `tinyskills` resource reads.

use super::api::{load_workflow_metadata_with_gate, DiscoveryGate};
use std::path::Path;

pub fn read_workflow_resource(
    workspace_dir: &Path,
    skill_id: &str,
    relative_path: &Path,
) -> Result<String, String> {
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    read_workflow_resource_with_gate(
        workspace_dir,
        skill_id,
        relative_path,
        DiscoveryGate::from_dispatch(&ctx),
        super::api::discovery_home_dir(),
    )
}

/// Read a skill resource under the caller's explicit Config and dispatch
/// identity. Host adapters that already own these inputs use this path so a
/// workspace shared by several profiles cannot reuse another profile's view.
pub fn read_workflow_resource_for(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &crate::config::Config,
    skill_id: &str,
    relative_path: &Path,
) -> Result<String, String> {
    let home = match ctx.resolved() {
        Some(core) if core.user_skill_roots() => dirs::home_dir(),
        Some(_) => None,
        None => super::api::discovery_home_dir(),
    };
    read_workflow_resource_with_gate(
        &config.workspace_dir,
        skill_id,
        relative_path,
        DiscoveryGate::from_config(ctx, config, None),
        home,
    )
}

fn read_workflow_resource_with_gate(
    workspace_dir: &Path,
    skill_id: &str,
    relative_path: &Path,
    gate: DiscoveryGate,
    home: Option<std::path::PathBuf>,
) -> Result<String, String> {
    if skill_id.trim().is_empty() {
        return Err("skill_id must not be empty".to_string());
    }
    let skills = load_workflow_metadata_with_gate(workspace_dir, home, gate.clone());
    let skill = tinyskills::resolve_skill(skills, skill_id)?;
    #[cfg(feature = "flows")]
    if skill.scope == tinyskills::SkillScope::Builtin && skill.id() == "flow-authoring" {
        if !gate.flow_authoring_enabled() {
            return Err("flow-authoring is unavailable in this context".to_string());
        }
        let generation =
            crate::skills::bundled::flow_authoring::generation_if_current(workspace_dir, &gate)
                .ok_or_else(|| "flow-authoring resources are unavailable".to_string())?;
        return crate::skills::bundled::flow_authoring::read_resource(
            workspace_dir,
            &gate,
            generation,
            &skill,
            relative_path,
        );
    }
    tinyskills::read_resource(&skill, relative_path).map_err(|error| error.to_string())
}
