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

// -- Workspace boundary -------------------------------------------------------
//
// These drive the boundary through the real `validate_parent_path` gate every
// file write funnels through.

/// A `workspace_only` policy whose workspace is `<root>/home`, plus a separate
/// `<root>/checkout` directory standing in for a tree outside it.
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

#[path = "policy_action_dir_tests.rs"]
mod action_dir_tests;
#[path = "policy_allowlist_tests.rs"]
mod allowlist_tests;
#[path = "policy_config_and_action_tests.rs"]
mod config_and_action_tests;
#[path = "policy_paths_and_risk_tests.rs"]
mod paths_and_risk_tests;
#[path = "policy_trusted_roots_tests.rs"]
mod trusted_roots_tests;
