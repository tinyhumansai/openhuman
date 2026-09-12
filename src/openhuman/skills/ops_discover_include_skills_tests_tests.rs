use super::*;

/// Write a minimal `<file>`-named bundle under `root/slug/`.
fn seed_bundle(root: &Path, slug: &str, file: &str) {
    let dir = root.join(slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(file),
        format!("---\nname: {slug}\ndescription: {slug} desc\n---\n\n{slug} body\n"),
    )
    .unwrap();
}

/// `discover_automations` lists only `workflows/`-root automations, while
/// `discover_workflows` additionally surfaces `skills/`-root installs. This
/// is exactly the branch `handle_skills_list` selects on `include_skills`
/// so the Skills Explorer's Installed tab can show registry installs (#3954).
#[test]
fn automations_excludes_skill_roots_but_full_discover_includes_them() {
    let home = tempfile::TempDir::new().unwrap();
    let home_path = home.path();
    // A registry-style install lands under `~/.openhuman/skills/`.
    seed_bundle(
        &home_path.join(".openhuman").join("skills"),
        "installed-skill",
        "SKILL.md",
    );
    // A "New workflow" automation lands under `~/.openhuman/workflows/`.
    seed_bundle(
        &home_path.join(".openhuman").join("workflows"),
        "my-automation",
        "WORKFLOW.md",
    );

    // Automations-only view (the default `skills_list` path) hides the skill.
    let automations = discover_automations(Some(home_path), None, false);
    let auto_names: Vec<&str> = automations.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(
        auto_names,
        vec!["my-automation"],
        "discover_automations must exclude `skills/`-root installs"
    );

    // Full view (`include_skills=true`) surfaces both.
    let full = discover_workflows(Some(home_path), None, false);
    let mut full_names: Vec<&str> = full.iter().map(|w| w.name.as_str()).collect();
    full_names.sort_unstable();
    assert_eq!(
        full_names,
        vec!["installed-skill", "my-automation"],
        "discover_workflows must include `skills/`-root installs"
    );
}
