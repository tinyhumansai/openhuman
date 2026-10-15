//! Phase-0 policy characterization through the public `SecurityPolicy` surface
//! (issue #7328, plan Task 1). The command grammar, allowlist and the
//! always-forbidden table are pinned beside the code in
//! `security/policy/policy_characterization_tests.rs`, because classification
//! and the allowlist are crate-private outside tests. This file pins what an
//! embedder or tool can observe: the shipped defaults, the disabled floor, the
//! autonomy gate matrix, the hourly limiter and trusted-root grants.

#[path = "support/env_guard.rs"]
mod env_guard;

use std::path::Path;

use tinybox_bus::CommandClass;

use openhuman_core::{
    config::AutonomyConfig,
    security::{
        policy::GateDecision, AutonomyLevel, SecurityPolicy, ToolOperation, TrustedAccess,
        TrustedRoot,
    },
};

fn policy(enabled: bool, autonomy: AutonomyLevel) -> SecurityPolicy {
    SecurityPolicy {
        enabled,
        autonomy,
        ..SecurityPolicy::default()
    }
}

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

#[test]
fn gate_decision_matrix_is_pinned() {
    use CommandClass::*;
    use GateDecision::*;
    let classes = [Read, Write, Network, Install, Destructive];
    let expect = [
        (AutonomyLevel::ReadOnly, [Allow, Block, Block, Block, Block]),
        (
            AutonomyLevel::Supervised,
            [Allow, Prompt, Prompt, Prompt, Prompt],
        ),
        (AutonomyLevel::Full, [Allow, Allow, Prompt, Prompt, Prompt]),
    ];
    for (tier, row) in expect {
        for (class, want) in classes.iter().zip(row) {
            assert_eq!(
                policy(true, tier).gate_decision(*class),
                want,
                "{tier:?} {class:?}"
            );
            assert_eq!(
                policy(false, tier).gate_decision(*class),
                Allow,
                "disabled {tier:?} {class:?}"
            );
        }
    }
    assert_eq!(
        SecurityPolicy::parse_declared_class("NETWORK"),
        Some(Network)
    );
    assert_eq!(SecurityPolicy::parse_declared_class("nonsense"), None);
}

#[test]
fn disabled_policy_still_forbids_credential_stores_and_system_roots() {
    let off = policy(false, AutonomyLevel::Full);
    for forbidden in [
        "/home/u/.ssh/id_rsa",
        "/home/u/.aws/credentials",
        "/home/u/.gnupg/x",
        "/etc/passwd",
        "/root/x",
    ] {
        assert!(
            !off.is_resolved_path_allowed(Path::new(forbidden)),
            "{forbidden}"
        );
        assert!(!off.is_path_string_allowed(forbidden), "{forbidden}");
    }
    assert!(off.is_resolved_path_allowed(Path::new("/srv/anything/else")));
}

#[test]
fn enabled_policy_confines_paths_to_workspace_and_trusted_roots() {
    let ws = tempfile::tempdir().unwrap();
    let workspace = ws.path().canonicalize().unwrap();
    let p = SecurityPolicy {
        enabled: true,
        workspace_dir: workspace.clone(),
        action_dir: workspace.clone(),
        workspace_only: true,
        trusted_roots: vec![
            TrustedRoot {
                path: "/data/ro".into(),
                access: TrustedAccess::Read,
            },
            TrustedRoot {
                path: "/data/rw".into(),
                access: TrustedAccess::ReadWrite,
            },
            // A grant over a credential directory never works.
            TrustedRoot {
                path: "/home/u".into(),
                access: TrustedAccess::ReadWrite,
            },
        ],
        ..SecurityPolicy::default()
    };
    assert!(p.is_resolved_path_allowed(&workspace.join("a.txt")));
    assert!(!p.is_resolved_path_allowed(Path::new("/elsewhere/a.txt")));
    assert!(p.is_resolved_path_allowed_for(Path::new("/data/ro/a"), false));
    assert!(!p.is_resolved_path_allowed_for(Path::new("/data/ro/a"), true));
    assert!(p.is_resolved_path_allowed_for(Path::new("/data/rw/a"), true));
    assert!(!p.is_resolved_path_allowed_for(Path::new("/home/u/.ssh/id_rsa"), false));
    assert!(p.is_resolved_path_allowed_for(Path::new("/home/u/project/a"), true));
}

#[tokio::test]
async fn validate_path_rejects_traversal_and_the_floor_even_when_disabled() {
    let ws = tempfile::tempdir().unwrap();
    let workspace = ws.path().canonicalize().unwrap();
    for enabled in [true, false] {
        let p = SecurityPolicy {
            enabled,
            workspace_dir: workspace.clone(),
            action_dir: workspace.clone(),
            ..SecurityPolicy::default()
        };
        assert!(
            p.validate_path("/etc/passwd").await.is_err(),
            "enabled={enabled}"
        );
        assert!(
            p.validate_path("../../../etc/passwd").await.is_err(),
            "enabled={enabled}"
        );
        assert!(
            p.validate_path("nul\0byte").await.is_err(),
            "enabled={enabled}"
        );
    }
}

#[test]
fn hourly_limiter_counts_acts_not_reads_and_names_the_limit() {
    let p = SecurityPolicy {
        max_actions_per_hour: 2,
        ..policy(true, AutonomyLevel::Supervised)
    };
    for _ in 0..20 {
        assert!(p
            .enforce_tool_operation(ToolOperation::Read, "read")
            .is_ok());
    }
    assert!(!p.is_rate_limited());
    assert!(p.enforce_tool_operation(ToolOperation::Act, "a").is_ok());
    assert!(p.enforce_tool_operation(ToolOperation::Act, "b").is_ok());
    assert!(p.is_rate_limited());
    let err = p
        .enforce_tool_operation(ToolOperation::Act, "c")
        .unwrap_err();
    assert!(
        err.contains("Rate limit exceeded") && err.contains("2 actions/hour"),
        "{err}"
    );
    // Each policy instance owns its window; a fresh policy starts empty.
    let fresh = SecurityPolicy {
        max_actions_per_hour: 2,
        ..policy(true, AutonomyLevel::Supervised)
    };
    assert!(!fresh.is_rate_limited());
}

#[test]
fn read_only_and_disabled_tiers_on_acts() {
    let ro = policy(true, AutonomyLevel::ReadOnly);
    assert!(!ro.can_act());
    assert!(ro
        .enforce_tool_operation(ToolOperation::Act, "write")
        .is_err());
    assert!(ro
        .enforce_tool_operation(ToolOperation::Read, "read")
        .is_ok());
    let off = SecurityPolicy {
        max_actions_per_hour: 0,
        ..policy(false, AutonomyLevel::ReadOnly)
    };
    assert!(off.can_act(), "a disabled policy never blocks acting");
    assert!(off
        .enforce_tool_operation(ToolOperation::Act, "write")
        .is_ok());
    assert!(!off.is_rate_limited());
}
