//! Policy characterization for issue #646 through the public host surface.
//! This target pins shipped defaults, parsing, and trusted-root grants; the
//! core unit corpus pins the command allowlist and classification.

#[path = "support/env_guard.rs"]
mod env_guard;

use std::path::Path;

use openhuman_core::{
    config::AutonomyConfig,
    security::{AutonomyLevel, SecurityPolicy, TrustedAccess, TrustedRoot},
};

#[test]
fn shipped_defaults_are_pinned() {
    let cfg = AutonomyConfig::default();
    assert!(!cfg.enabled, "the policy is off by default");
    assert_eq!(cfg.level, AutonomyLevel::Supervised);
    assert!(cfg.workspace_only);
    assert_eq!(cfg.max_actions_per_hour, u32::MAX, "effectively unlimited");
    assert!(cfg.require_approval_for_medium_risk && cfg.block_high_risk_commands);
    assert!(!cfg.auto_approve_all && !cfg.allow_tool_install);
    assert_eq!(
        cfg.auto_approve,
        [
            "file_read",
            "memory_search",
            "memory_list",
            "get_time",
            "list_dir",
            "glob",
            "grep"
        ]
    );
    assert!(cfg.allowed_commands.iter().any(|c| c == "git"));
    for credential_dir in ["~/.ssh", "~/.gnupg", "~/.aws"] {
        assert!(
            cfg.forbidden_paths.iter().any(|p| p == credential_dir),
            "{credential_dir}"
        );
    }
    // A policy built with no config at all fails closed (enabled).
    assert!(SecurityPolicy::default().enabled);
}

#[test]
fn from_config_carries_every_field_and_adds_the_standard_grants() {
    let _lock = env_guard::env_lock();
    let ws = tempfile::tempdir().unwrap();
    let action = ws.path().join("action");
    let state = ws.path().join("state");
    let cfg = AutonomyConfig {
        enabled: true,
        level: AutonomyLevel::Full,
        auto_approve_all: true,
        max_actions_per_hour: 7,
        trusted_roots: vec![TrustedRoot {
            path: "/data/shared".into(),
            access: TrustedAccess::Read,
        }],
        ..AutonomyConfig::default()
    };
    let p = SecurityPolicy::from_config(&cfg, &state, &action);
    assert!(p.enabled && p.auto_approve_all);
    assert_eq!(p.autonomy, AutonomyLevel::Full);
    assert_eq!(p.max_actions_per_hour, 7);
    assert_eq!(p.workspace_dir, state);
    assert_eq!(p.action_dir, action);
    let root = |path: &str| p.trusted_roots.iter().find(|r| r.path == path);
    assert_eq!(root("/data/shared").unwrap().access, TrustedAccess::Read);
    // The action dir is a read-write root; tool-result artifacts are read-only.
    assert_eq!(
        root(&action.to_string_lossy()).unwrap().access,
        TrustedAccess::ReadWrite
    );
    let artifacts = openhuman_core::security::policy::tool_result_artifacts_dir(&state);
    assert_eq!(
        root(&artifacts.to_string_lossy()).unwrap().access,
        TrustedAccess::Read
    );
}

#[test]
fn action_dir_is_not_granted_when_it_covers_the_workspace() {
    let _lock = env_guard::env_lock();
    let root = tempfile::tempdir().unwrap();
    let cfg = AutonomyConfig {
        enabled: true,
        ..AutonomyConfig::default()
    };
    let p = SecurityPolicy::from_config(&cfg, &root.path().join("state"), root.path());
    assert!(
        p.trusted_roots
            .iter()
            .all(|r| Path::new(&r.path) != root.path()),
        "an ancestor of the workspace must not become a trusted root"
    );
}
