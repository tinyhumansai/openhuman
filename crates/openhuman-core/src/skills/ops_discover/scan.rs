//! Host adapter over `tinyskills::layout`.
//!
//! The on-disk root layout, scan order and cross-scope collision handling are
//! owned by the module. This file feeds it the host-only inputs (the compiled-in
//! builtin bundles and the per-agent home) and applies the `source_format`
//! rename the product surface expects.

use std::path::Path;

use tinyskills::{discover_layout, LayoutQuery, RootKind};

use super::api::DiscoveryGate;

pub(super) use tinyskills::{ALL_ROOT_KINDS, WORKFLOW_ROOT_KINDS};

use {tinyskills::Skill as Workflow, tinyskills::SkillScope as WorkflowScope};

/// Shared discovery core. `kinds` selects which root categories to scan,
/// letting the full surface ([`super::discover_workflows_inner`]) and the
/// automations-only list ([`super::discover_automations`]) share collision
/// handling.
pub(super) fn discover_filtered(
    home_dir: Option<&Path>,
    workspace_dir: Option<&Path>,
    trusted: bool,
    kinds: &[RootKind],
    gate: &DiscoveryGate,
    flow_generation: Option<u64>,
) -> Vec<Workflow> {
    #[cfg(not(feature = "flows"))]
    let _ = (gate, flow_generation);
    tracing::debug!(
        trusted,
        has_home = home_dir.is_some(),
        has_workspace = workspace_dir.is_some(),
        include_skills = kinds.contains(&RootKind::Skill),
        include_workflows = kinds.contains(&RootKind::Workflow),
        "[workflows] discover:enter"
    );

    // Builtin skills (`<workspace>/.openhuman/builtin-skills/`) are registered
    // first and at the lowest precedence. No trust marker is consulted: the
    // directory is core-managed and its contents were written from constants
    // compiled into this binary. See `skills::bundled`.
    let builtin = match workspace_dir {
        Some(ws) if kinds.contains(&RootKind::Skill) => {
            scan_bundled_root(&crate::skills::bundled::builtin_root(ws))
        }
        _ => Vec::new(),
    };
    #[cfg(feature = "flows")]
    let mut builtin = builtin;
    #[cfg(feature = "flows")]
    if gate.flow_authoring_enabled() && kinds.contains(&RootKind::Skill) {
        if let (Some(workspace), Some(generation)) = (workspace_dir, flow_generation) {
            if let Some(flow_authoring) = crate::skills::bundled::flow_authoring::materialized_skill(
                workspace, gate, generation,
            ) {
                builtin.push(flow_authoring);
            }
        }
    }

    let extra_user_roots = workspace_dir
        .and_then(crate::skills::write_root::agent_skill_home)
        .map(|agent_home| {
            let [skills, workflows] = crate::skills::write_root::agent_user_roots(&agent_home);
            vec![(skills, RootKind::Skill), (workflows, RootKind::Workflow)]
        })
        .unwrap_or_default();

    let mut out = discover_layout(&LayoutQuery {
        home: home_dir,
        workspace: workspace_dir,
        trusted,
        kinds,
        builtin,
        extra_user_roots,
        id_noun: "workflow".into(),
    });
    out.iter_mut().for_each(normalize_source_format);
    tracing::debug!(discovered_count = out.len(), "[workflows] discover:exit");
    out
}

fn scan_bundled_root(root: &Path) -> Vec<Workflow> {
    let mut out = Vec::new();
    for bundled in crate::skills::bundled::BUNDLED {
        let dir = root.join(bundled.dir_name);
        if crate::skills::bundled::is_current_materialization(&dir, bundled) {
            if let Some(workflow) = tinyskills::load_skill_dir(&dir, WorkflowScope::Builtin) {
                out.push(workflow);
            }
        }
    }
    out
}

fn normalize_source_format(workflow: &mut Workflow) {
    if workflow.source_format == "agentskills" {
        workflow.source_format = "openhuman".to_string();
    }
}
