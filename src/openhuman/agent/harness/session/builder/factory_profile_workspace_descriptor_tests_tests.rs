use super::{build_profile_security, derive_profile_workspace_descriptor};
use crate::openhuman::agent::profiles::store::built_in_default_profile;

fn profile(id: &str, dedicated_workspace: bool) -> crate::openhuman::agent::profiles::AgentProfile {
    let mut p = built_in_default_profile();
    p.id = id.to_string();
    p.name = id.to_string();
    p.built_in = false;
    p.is_master = false;
    p.memory_dir_suffix = None;
    p.dedicated_workspace = dedicated_workspace;
    p
}

#[test]
fn dedicated_workspace_profile_roots_descriptor_at_profile_dir() {
    // Real temp action_dir: the production fn creates the profile dir as a
    // side effect, so assert against the resolved path suffix.
    let action = tempfile::tempdir().expect("action tempdir");
    let p = profile("alice", true);
    let desc = derive_profile_workspace_descriptor(action.path(), Some(&p))
        .expect("dedicated_workspace profile yields a descriptor");
    let expected = action.path().join("profiles").join("alice");
    assert_eq!(desc.root.as_path(), expected.as_path());
    // The production path really created the dir.
    assert!(desc.root.is_dir());
}

#[test]
fn shared_profile_yields_no_descriptor() {
    let action = tempfile::tempdir().expect("action tempdir");
    let p = profile("bob", false);
    assert!(derive_profile_workspace_descriptor(action.path(), Some(&p)).is_none());
}

#[test]
fn none_profile_yields_no_descriptor() {
    let action = tempfile::tempdir().expect("action tempdir");
    assert!(derive_profile_workspace_descriptor(action.path(), None).is_none());
}

#[test]
fn legacy_invalid_id_yields_no_descriptor_even_when_opted_in() {
    let action = tempfile::tempdir().expect("action tempdir");
    // An id that fails validation can't mint a workspace path → no descriptor,
    // so the session falls back to the shared action_dir cwd.
    let p = profile("Bad Id", true);
    assert!(derive_profile_workspace_descriptor(action.path(), Some(&p)).is_none());
}

#[test]
fn create_dir_failure_yields_no_descriptor() {
    // Point `action_dir` at a regular file so `profiles/…` can't be created.
    // The function must fall back to `None` (shared action_dir cwd) rather
    // than hand tools a descriptor rooted at a nonexistent dir.
    let tmp = tempfile::tempdir().expect("tempdir");
    let action_file = tmp.path().join("not-a-dir");
    std::fs::write(&action_file, b"x").expect("write file");
    let p = profile("alice", true);
    assert!(
        derive_profile_workspace_descriptor(&action_file, Some(&p)).is_none(),
        "a create_dir_all failure must fall back to None"
    );
}

#[test]
fn shared_profile_still_arms_cross_profile_guard() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::openhuman::config::Config::default();
    config.action_dir = temp.path().join("actions");
    config.workspace_dir = temp.path().join("state");
    let profile = profile("default", false);

    let security = build_profile_security(&config, Some(&profile));

    let guard = security
        .active_profile
        .expect("every active profile must arm the guard");
    assert_eq!(guard.profile_id, "default");
    assert_eq!(guard.action_dir, config.action_dir);
}

#[test]
fn profile_less_session_leaves_cross_profile_guard_disarmed() {
    let config = crate::openhuman::config::Config::default();
    assert!(build_profile_security(&config, None)
        .active_profile
        .is_none());
}
