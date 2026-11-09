//! OpenHuman's compiled-in skill table and product-specific install root.

use std::path::{Path, PathBuf};

use tinyskills::{BundledFile, BundledSkill, InstallReport};

#[cfg(feature = "flows")]
pub(crate) mod flow_authoring;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod mod_tests;

/// Skills compiled into this product build.
pub const BUNDLED: &[BundledSkill] = &[
    #[cfg(feature = "modules")]
    BundledSkill {
        dir_name: "desktop-control",
        files: &[BundledFile {
            path: "WORKFLOW.md",
            contents: include_str!("../../desktop/control/WORKFLOW.md"),
        }],
    },
];

/// Product-owned root for materialized builtin skills.
pub fn builtin_root(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join(".openhuman").join("builtin-skills")
}

/// Materialize all compiled-in skills and log the outcome.
pub fn install(workspace_dir: &Path) -> InstallReport {
    let root = builtin_root(workspace_dir);
    let report = tinyskills::install(&root, BUNDLED);
    for skill in &report.written {
        tracing::info!(skill, "[skills][bundled] wrote builtin skill");
    }
    for skill in &report.unchanged {
        tracing::debug!(skill, "[skills][bundled] builtin skill already current");
    }
    for (skill, error) in &report.failed {
        tracing::warn!(
            skill,
            error,
            "[skills][bundled] failed to install builtin skill"
        );
    }
    report
}

/// Boot entry point used by runtime composition.
pub fn install_bundled_skills(workspace_dir: &Path) {
    if !crate::skills::skills_component_enabled() {
        tracing::debug!("[skills][bundled] install skipped ([components] skills off)");
        return;
    }
    let _ = install(workspace_dir);
}

/// Verify a materialized builtin still exactly matches its compiled bytes.
pub fn is_current_materialization(dir: &Path, skill: &BundledSkill) -> bool {
    tinyskills::is_current_materialization(dir, *skill)
}
