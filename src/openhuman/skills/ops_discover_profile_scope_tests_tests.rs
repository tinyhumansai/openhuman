use super::*;

/// Write a minimal `WORKFLOW.md` bundle under `root/slug/`.
fn seed_bundle(root: &Path, slug: &str) {
    seed_bundle_with_name(root, slug, slug);
}

fn seed_bundle_with_name(root: &Path, slug: &str, name: &str) {
    let dir = root.join(slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("WORKFLOW.md"),
        format!("---\nname: {name}\ndescription: {name} desc\n---\n\n{name} body\n"),
    )
    .unwrap();
}

/// Profile-local skills appear ONLY when their root is passed, and never for
/// the profile-less session or a *different* profile's root (2a scoping
/// matrix).
#[test]
fn profile_local_skills_scoped_to_their_owner() {
    let home = tempfile::TempDir::new().unwrap();
    // A global user-scope skill everyone sees.
    seed_bundle(
        &home.path().join(".openhuman").join("skills"),
        "global-skill",
    );

    // Two distinct profile roots (alice / bob), each with a private skill.
    let alice_root = tempfile::TempDir::new().unwrap();
    seed_bundle(alice_root.path(), "alice-only");
    let bob_root = tempfile::TempDir::new().unwrap();
    seed_bundle(bob_root.path(), "bob-only");

    let names = |workflows: Vec<Workflow>| {
        let mut n: Vec<String> = workflows.into_iter().map(|w| w.name).collect();
        n.sort();
        n
    };

    // No profile: only the global skill.
    let none = names(discover_workflows_with_profile(
        Some(home.path()),
        None,
        None,
        false,
    ));
    assert_eq!(none, vec!["global-skill"]);

    // Alice's turn: global + alice-only, never bob-only.
    let alice = names(discover_workflows_with_profile(
        Some(home.path()),
        None,
        Some(alice_root.path()),
        false,
    ));
    assert_eq!(alice, vec!["alice-only", "global-skill"]);

    // Bob's turn: global + bob-only, never alice-only.
    let bob = names(discover_workflows_with_profile(
        Some(home.path()),
        None,
        Some(bob_root.path()),
        false,
    ));
    assert_eq!(bob, vec!["bob-only", "global-skill"]);
}

/// A profile-local skill named the same as a global skill wins for its owner
/// (highest precedence) and is tagged `WorkflowScope::Profile` (2a collision
/// precedence).
#[test]
fn profile_local_wins_same_name_collision() {
    let home = tempfile::TempDir::new().unwrap();
    seed_bundle(
        &home.path().join(".openhuman").join("skills"),
        "shared-name",
    );
    let profile_root = tempfile::TempDir::new().unwrap();
    seed_bundle(profile_root.path(), "shared-name");

    let workflows =
        discover_workflows_with_profile(Some(home.path()), None, Some(profile_root.path()), false);
    let winner = workflows
        .iter()
        .find(|w| w.name == "shared-name")
        .expect("shared-name resolved");
    // Exactly one entry for the name, and it is the profile-local copy.
    assert_eq!(
        workflows.iter().filter(|w| w.name == "shared-name").count(),
        1,
        "collision must collapse to a single winner"
    );
    assert_eq!(
        winner.scope,
        WorkflowScope::Profile,
        "profile-local skill must win the same-name collision"
    );
    // The winner resolves under the profile root, not the global one.
    let canon_profile = std::fs::canonicalize(profile_root.path()).unwrap();
    let loc = std::fs::canonicalize(winner.location.as_ref().unwrap()).unwrap();
    assert!(
        loc.starts_with(&canon_profile),
        "winning skill must live under the profile root, got {}",
        loc.display()
    );
}

#[test]
fn profile_local_wins_same_runnable_id_with_different_display_name() {
    let home = tempfile::TempDir::new().unwrap();
    seed_bundle_with_name(
        &home.path().join(".openhuman").join("skills"),
        "shared-slug",
        "Global display name",
    );
    let profile_root = tempfile::TempDir::new().unwrap();
    seed_bundle_with_name(profile_root.path(), "shared-slug", "Profile display name");

    let workflows =
        discover_workflows_with_profile(Some(home.path()), None, Some(profile_root.path()), false);
    let by_slug: Vec<_> = workflows
        .iter()
        .filter(|workflow| workflow.dir_name == "shared-slug")
        .collect();
    assert_eq!(by_slug.len(), 1, "runnable ids must be unique");
    assert_eq!(by_slug[0].scope, WorkflowScope::Profile);
    assert_eq!(by_slug[0].name, "Profile display name");
}

/// `WorkflowScope::Profile` outranks every global scope in the precedence
/// ladder (the mechanism the collision test relies on).
#[test]
fn profile_scope_has_highest_precedence() {
    assert!(precedence(WorkflowScope::Profile) > precedence(WorkflowScope::Project));
    assert!(precedence(WorkflowScope::Profile) > precedence(WorkflowScope::User));
    assert!(precedence(WorkflowScope::Profile) > precedence(WorkflowScope::Legacy));
}

/// Seed a runnable bundle with a bundled resource under `references/`.
fn seed_bundle_with_resource(root: &Path, slug: &str, resource_body: &str) {
    let dir = root.join(slug);
    std::fs::create_dir_all(dir.join("references")).unwrap();
    std::fs::write(
        dir.join("WORKFLOW.md"),
        format!("---\nname: {slug}\ndescription: {slug} desc\n---\n\n{slug} body\n"),
    )
    .unwrap();
    std::fs::write(dir.join("references").join("note.md"), resource_body).unwrap();
}

/// `read_workflow_resource_with_profile` (the `read_workflow_resource` tool's
/// seam) resolves a profile's private skill resources for the owner only,
/// resolves collisions to the profile-local copy, hides them from other
/// profiles / the profile-less session, and leaves global-only resources
/// readable everywhere.
#[test]
fn read_workflow_resource_with_profile_resolution_matrix() {
    let ws = tempfile::TempDir::new().unwrap();
    let profile_root = tempfile::TempDir::new().unwrap();
    let other_root = tempfile::TempDir::new().unwrap();

    // Global (legacy) skill + resource, private skill + resource, and a
    // collision under both.
    seed_bundle_with_resource(&ws.path().join("skills"), "resglobal7788", "GLOBAL_RES");
    seed_bundle_with_resource(profile_root.path(), "reslocal7788", "LOCAL_RES");
    seed_bundle_with_resource(&ws.path().join("skills"), "rescollide7788", "GLOBAL_RES");
    seed_bundle_with_resource(profile_root.path(), "rescollide7788", "PROFILE_RES");

    let rel = Path::new("references/note.md");
    let read = |id: &str, root: Option<&Path>| {
        read_workflow_resource_with_profile(ws.path(), id, rel, root)
    };

    // Owner reads its private skill's resource.
    assert_eq!(
        read("reslocal7788", Some(profile_root.path())).unwrap(),
        "LOCAL_RES"
    );
    // Profile-less + other profile cannot resolve the private skill at all.
    assert!(read("reslocal7788", None).is_err());
    assert!(read("reslocal7788", Some(other_root.path())).is_err());

    // Global-only resource is readable with or without a profile root.
    assert_eq!(read("resglobal7788", None).unwrap(), "GLOBAL_RES");
    assert_eq!(
        read("resglobal7788", Some(profile_root.path())).unwrap(),
        "GLOBAL_RES"
    );

    // Collision: owner reads the profile-local resource; everyone else the global.
    assert_eq!(
        read("rescollide7788", Some(profile_root.path())).unwrap(),
        "PROFILE_RES"
    );
    assert_eq!(read("rescollide7788", None).unwrap(), "GLOBAL_RES");
}

/// `profile_local_skill_ids` returns both runnable names and directory slugs
/// under the profile root (the implicit-allow set the describe/read/run tools
/// consult), and is empty for the profile-less session.
#[test]
fn profile_local_skill_ids_lists_only_the_profile_root() {
    let profile_root = tempfile::TempDir::new().unwrap();
    seed_bundle(profile_root.path(), "priv-a");
    seed_bundle(profile_root.path(), "priv-b");

    let ids = profile_local_skill_ids(Some(profile_root.path()));
    assert_eq!(ids.len(), 2);
    assert!(ids.contains("priv-a"));
    assert!(ids.contains("priv-b"));

    assert!(
        profile_local_skill_ids(None).is_empty(),
        "profile-less session has no implicitly-allowed profile-local ids"
    );
}

#[test]
fn profile_local_skill_ids_include_distinct_name_and_slug() {
    let profile_root = tempfile::TempDir::new().unwrap();
    seed_bundle_with_name(profile_root.path(), "mail-helper", "Inbox Assistant");

    let ids = profile_local_skill_ids(Some(profile_root.path()));
    assert_eq!(ids.len(), 2);
    assert!(ids.contains("mail-helper"));
    assert!(ids.contains("Inbox Assistant"));
}

/// A `None` profile root reproduces `load_workflow_metadata` byte-for-byte —
/// the back-compat guarantee for the profile-less session.
#[test]
fn none_profile_root_matches_plain_discovery() {
    let home = tempfile::TempDir::new().unwrap();
    seed_bundle(&home.path().join(".openhuman").join("skills"), "a-skill");
    let with_none = discover_workflows_with_profile(Some(home.path()), None, None, false);
    let plain = discover_workflows(Some(home.path()), None, false);
    let names: Vec<&str> = with_none.iter().map(|w| w.name.as_str()).collect();
    let plain_names: Vec<&str> = plain.iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, plain_names);
}
