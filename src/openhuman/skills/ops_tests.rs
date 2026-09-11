use super::*;

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

/// Workspace-only variant of [`load_workflow_metadata`] used by tests that care only
/// about project-scope semantics. The production [`load_workflow_metadata`] now
/// consults `dirs::home_dir()`; in unit tests that would non-deterministically
/// pick up whatever skills the developer has installed under their real
/// home. Tests exercising user-scope delegation drive a tempdir through
/// [`discover_workflows`] explicitly (see `load_skills_surfaces_user_scope`).
fn load_skills_ws(workspace_dir: &Path) -> Vec<Workflow> {
    let trusted = is_workspace_trusted(workspace_dir);
    discover_workflows_inner(None, Some(workspace_dir), None, trusted)
}

// -- read_workflow_resource -------------------------------------------------
//
// These tests exercise the resource-read path via legacy-scope skills
// (`<ws>/skills/<name>/`) because that scope doesn't require the trust
// marker, is fully workspace-scoped, and avoids touching the user's home
// directory. The guarantees tested here apply equally to user- and
// project-scope skills since they all flow through the same
// `canonicalize` + `symlink_metadata` + size check gauntlet.

fn make_legacy_skill(ws: &Path, name: &str) -> PathBuf {
    let skill_dir = ws.join("skills").join(name);
    write(
        &skill_dir.join("SKILL.md"),
        &format!("---\nname: {name}\ndescription: test skill\n---\n# {name}\n"),
    );
    skill_dir
}

#[path = "ops_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "ops_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "ops_tests_part_03_tests.rs"]
mod part_03_tests;
