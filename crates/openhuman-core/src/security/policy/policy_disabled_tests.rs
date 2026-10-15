//! `[autonomy] enabled = false` — the shipped default.
//!
//! Two halves, and both matter: with the policy disabled a shell is a shell and
//! a working directory is writable, **but** credential stores and system roots
//! are still unreachable. The second half is the one a future edit is most
//! likely to break, so it is asserted here beside the first.

use super::*;
use std::path::PathBuf;
use tinybox_bus::CommandClass;

fn disabled_policy() -> SecurityPolicy {
    SecurityPolicy {
        enabled: false,
        ..SecurityPolicy::default()
    }
}

#[test]
fn config_default_is_disabled_but_struct_default_is_enabled() {
    // The shipped default: nothing in a stock config turns the policy on.
    assert!(!crate::config::AutonomyConfig::default().enabled);
    // The fail-closed fallback for a policy built with no config at all.
    assert!(SecurityPolicy::default().enabled);
}

#[test]
fn from_config_carries_the_disabled_flag() {
    let cfg = crate::config::AutonomyConfig::default();
    let policy = SecurityPolicy::from_config(&cfg, Path::new("/ws"), Path::new("/act"));
    assert!(!policy.enabled);
}

#[test]
fn disabled_policy_never_prompts_or_blocks_on_class() {
    let policy = disabled_policy();
    for class in [
        CommandClass::Read,
        CommandClass::Write,
        CommandClass::Network,
        CommandClass::Install,
        CommandClass::Destructive,
    ] {
        assert_eq!(policy.gate_decision(class), GateDecision::Allow);
    }
    assert!(policy.can_act());
    assert!(!policy.is_rate_limited());
}

#[test]
fn disabled_policy_allows_paths_outside_the_workspace() {
    let policy = SecurityPolicy {
        enabled: false,
        workspace_only: true,
        workspace_dir: PathBuf::from("/ws"),
        action_dir: PathBuf::from("/act"),
        ..SecurityPolicy::default()
    };
    assert!(policy.is_path_string_allowed("/somewhere/else/notes.md"));
    assert!(policy.is_resolved_path_allowed_for(Path::new("/somewhere/else/notes.md"), true));
}

#[test]
fn disabled_policy_still_refuses_credential_stores_and_system_roots() {
    let policy = disabled_policy();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/nobody".into());
    for path in [
        format!("{home}/.ssh/id_rsa"),
        format!("{home}/.gnupg/secring.gpg"),
        format!("{home}/.aws/credentials"),
        "/etc/shadow".to_string(),
        "/root/.bashrc".to_string(),
    ] {
        assert!(
            !policy.is_path_string_allowed(&path),
            "is_always_forbidden must hold with the policy disabled: {path}"
        );
        assert!(
            !policy.is_resolved_path_allowed_for(Path::new(&path), false),
            "is_always_forbidden must hold on resolved paths too: {path}"
        );
    }
}

#[test]
fn disabled_policy_still_refuses_path_traversal_and_null_bytes() {
    let policy = disabled_policy();
    assert!(!policy.is_path_string_allowed("../../etc/passwd"));
    assert!(!policy.is_path_string_allowed("out/note\0.md"));
}
