use super::*;

fn default_policy() -> SecurityPolicy {
    SecurityPolicy::default()
}

fn readonly_policy() -> SecurityPolicy {
    SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    }
}

fn full_policy() -> SecurityPolicy {
    SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        ..SecurityPolicy::default()
    }
}

// -- Cross-profile write guard (1b) -------------------------------
//
// These drive the guard through the real `validate_parent_path` gate that every
// file write tool funnels through, proving the tightening lands at the shared
// call site (not just in the standalone classifier). `active_profile = None`
// keeps the exact same setup passing, pinning the byte-identical shared path.

/// Build a `<root>/projects/profiles/{alice,bob}` layout and a policy whose cwd
/// is scoped to alice (as `security_for_tool_context` would), with the guard
/// optionally armed for alice against the broad action root.
fn cross_profile_policy(arm_for_alice: bool) -> (tempfile::TempDir, PathBuf, SecurityPolicy) {
    let root = tempfile::tempdir().expect("root tempdir");
    let action_root = root.path().join("projects");
    let profiles = action_root.join("profiles");
    for id in ["alice", "bob"] {
        std::fs::create_dir_all(profiles.join(id)).unwrap();
    }
    let alice_dir = profiles.join("alice");
    let policy = SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        // Everything under `root` is inside the workspace, so the sibling path
        // clears containment and reaches the cross-profile check.
        workspace_dir: root.path().to_path_buf(),
        // Mirrors the per-tool-call override: cwd scoped to the profile dir.
        action_dir: alice_dir.clone(),
        workspace_only: false,
        // Clear the default forbidden list (it blocks /tmp, /var, …, which the
        // OS tempdir lives under) so the guard is what does the blocking.
        forbidden_paths: Vec::new(),
        active_profile: arm_for_alice.then(|| ActiveProfileGuard {
            profile_id: "alice".to_string(),
            action_dir: action_root.clone(),
        }),
        ..SecurityPolicy::default()
    };
    (root, action_root, policy)
}

// -- trusted_roots allow-list (Phase 1) ---------------------------

use std::fs;
use std::path::Path as StdPath;
use std::path::PathBuf as StdPathBuf;

fn trusted_policy(workspace: StdPathBuf, roots: Vec<TrustedRoot>) -> SecurityPolicy {
    SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        action_dir: workspace.clone(),
        workspace_dir: workspace,
        workspace_only: true,
        trusted_roots: roots,
        ..SecurityPolicy::default()
    }
}

/// (workspace_dir, outside_dir) under a fresh temp root.
fn ws_and_outside() -> (tempfile::TempDir, StdPathBuf, StdPathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path().join("workspace");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&outside).unwrap();
    (tmp, workspace, outside)
}

// -- Per-turn workspace grant (agent::turn_workspace) ------------------------
//
// These drive the grant through the real `validate_parent_path` gate every file
// write funnels through, so they prove the tightening/loosening lands at the
// shared call site rather than only in the standalone predicate.

/// A `workspace_only` policy whose workspace is `<root>/home`, plus a separate
/// `<root>/checkout` directory standing in for the run's own tree.
fn turn_workspace_policy() -> (tempfile::TempDir, PathBuf, SecurityPolicy) {
    let root = tempfile::tempdir().expect("root tempdir");
    let workspace = root.path().join("home");
    let checkout = root.path().join("checkout");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&checkout).unwrap();
    let policy = SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        workspace_only: true,
        // The OS tempdir lives under /tmp or /var, both on the default
        // forbidden list; clear it so the workspace boundary is what decides.
        forbidden_paths: Vec::new(),
        trusted_roots: Vec::new(),
        ..SecurityPolicy::default()
    };
    (root, checkout, policy)
}

#[path = "policy_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "policy_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "policy_tests_part_03_tests.rs"]
mod part_03_tests;
#[path = "policy_tests_part_04_tests.rs"]
mod part_04_tests;
#[path = "policy_tests_part_05_tests.rs"]
mod part_05_tests;
