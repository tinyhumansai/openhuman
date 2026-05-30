//! Tests for workflow discovery (scope resolution, project-shadows-user).

use super::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write_workflow(root: &Path, slug: &str, name: &str, when_to_use: &str) {
    let dir = root.join(slug);
    fs::create_dir_all(&dir).unwrap();
    let body = format!(
        "---\nname: {name}\ndescription: d\nwhen_to_use: {when_to_use}\nphases:\n  on_pick_up_task:\n    rules:\n      - go\n---\n# {name}\n"
    );
    fs::write(dir.join(WORKFLOW_MD), body).unwrap();
}

#[test]
fn discovers_user_scope_workflows() {
    let home = TempDir::new().unwrap();
    let root = home.path().join(".openhuman").join("workflows");
    write_workflow(&root, "alpha", "alpha", "do alpha");

    let found = discover_workflows(Some(home.path()), None, false);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "alpha");
    assert_eq!(found[0].scope, WorkflowScope::User);
    assert!(found[0].phases.contains_key("on_pick_up_task"));
}

#[test]
fn project_scope_skipped_when_untrusted() {
    let home = TempDir::new().unwrap();
    let ws = TempDir::new().unwrap();
    write_workflow(
        &ws.path().join(".openhuman").join("workflows"),
        "proj",
        "proj",
        "p",
    );

    let untrusted = discover_workflows(Some(home.path()), Some(ws.path()), false);
    assert!(untrusted.is_empty());

    let trusted = discover_workflows(Some(home.path()), Some(ws.path()), true);
    assert_eq!(trusted.len(), 1);
    assert_eq!(trusted[0].scope, WorkflowScope::Project);
}

#[test]
fn project_shadows_user_on_name_collision() {
    let home = TempDir::new().unwrap();
    let ws = TempDir::new().unwrap();
    write_workflow(
        &home.path().join(".openhuman").join("workflows"),
        "shared",
        "shared",
        "user version",
    );
    write_workflow(
        &ws.path().join(".openhuman").join("workflows"),
        "shared",
        "shared",
        "project version",
    );

    let found = discover_workflows(Some(home.path()), Some(ws.path()), true);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].scope, WorkflowScope::Project);
    assert_eq!(found[0].when_to_use, "project version");
}

#[test]
fn results_sorted_by_name() {
    let home = TempDir::new().unwrap();
    let root = home.path().join(".openhuman").join("workflows");
    write_workflow(&root, "zeta", "zeta", "z");
    write_workflow(&root, "alpha", "alpha", "a");

    let found = discover_workflows(Some(home.path()), None, false);
    let names: Vec<_> = found.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "zeta"]);
}

#[test]
fn dot_dirs_and_missing_marker_file_ignored() {
    let home = TempDir::new().unwrap();
    let root = home.path().join(".openhuman").join("workflows");
    // A hidden dir and a dir without WORKFLOW.md should both be skipped.
    fs::create_dir_all(root.join(".hidden")).unwrap();
    fs::create_dir_all(root.join("nomarker")).unwrap();
    write_workflow(&root, "real", "real", "r");

    let found = discover_workflows(Some(home.path()), None, false);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "real");
}

#[cfg(unix)]
#[test]
fn symlinked_workflow_md_is_not_followed() {
    use std::os::unix::fs::symlink;

    let home = TempDir::new().unwrap();
    let root = home.path().join(".openhuman").join("workflows");

    // A real workflow elsewhere whose WORKFLOW.md we try to alias into a
    // discovery directory via symlink — discovery must refuse to follow it.
    let real = TempDir::new().unwrap();
    let real_md = real.path().join("WORKFLOW.md");
    fs::write(
        &real_md,
        "---\nname: sneaky\ndescription: d\n---\n# sneaky\n",
    )
    .unwrap();

    let attacker_dir = root.join("attacker");
    fs::create_dir_all(&attacker_dir).unwrap();
    symlink(&real_md, attacker_dir.join(WORKFLOW_MD)).unwrap();

    let found = discover_workflows(Some(home.path()), None, false);
    assert!(
        found.iter().all(|w| w.name != "sneaky"),
        "symlinked WORKFLOW.md must not be followed: {:?}",
        found.iter().map(|w| &w.name).collect::<Vec<_>>()
    );
}
