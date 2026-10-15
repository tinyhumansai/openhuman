//! Phase-0 characterization of the host policy that the tinysecurity bus module
//! must reproduce (issue #7328, plan Task 1). These tables pin today's behaviour
//! in one place: command classes, the autonomy gate matrix, the allowlist under
//! each tier, the always-forbidden floor, trusted roots and the hourly limiter.
//! They bless what exists; desired changes are written as separate tests whose
//! names start with `desired_` and which are expected to fail until fixed.

use super::*;
use tinybox_bus::CommandClass;
use tinytools::status::POLICY_BLOCKED_MARKER;

fn policy(autonomy: AutonomyLevel) -> SecurityPolicy {
    SecurityPolicy {
        enabled: true,
        autonomy,
        ..SecurityPolicy::default()
    }
}

#[test]
fn declared_class_parses_and_is_escalate_only_input() {
    assert_eq!(
        SecurityPolicy::parse_declared_class(" Destructive "),
        Some(CommandClass::Destructive)
    );
    assert_eq!(
        SecurityPolicy::parse_declared_class("read"),
        Some(CommandClass::Read)
    );
    assert_eq!(SecurityPolicy::parse_declared_class("bogus"), None);
    assert_eq!(SecurityPolicy::parse_declared_class(""), None);
}

#[test]
fn gate_decision_matrix_is_pinned() {
    use CommandClass::*;
    use GateDecision::*;
    let classes = [Read, Write, Network, Install, Destructive];
    let read_only = [Allow, Block, Block, Block, Block];
    let supervised = [Allow, Prompt, Prompt, Prompt, Prompt];
    let full = [Allow, Allow, Prompt, Prompt, Prompt];
    for (i, class) in classes.iter().enumerate() {
        assert_eq!(
            policy(AutonomyLevel::ReadOnly).gate_decision(*class),
            read_only[i]
        );
        assert_eq!(
            policy(AutonomyLevel::Supervised).gate_decision(*class),
            supervised[i]
        );
        assert_eq!(policy(AutonomyLevel::Full).gate_decision(*class), full[i]);
        // Disabled policy never gates, whatever the tier says.
        let off = SecurityPolicy {
            enabled: false,
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        };
        assert_eq!(off.gate_decision(*class), Allow);
    }
}

#[test]
fn always_forbidden_floor_covers_credential_stores_and_system_roots() {
    for forbidden in [
        "/home/u/.ssh/id_rsa",
        "/home/u/.gnupg/pubring.kbx",
        "/home/u/.aws/credentials",
        "/home/u/.kube/config",
        "/Users/u/Library/Keychains/login.keychain-db",
        "/home/u/.netrc",
        "/home/u/.git-credentials",
        "/home/u/.docker/config.json",
        "/home/u/.cargo/credentials.toml",
        "C:\\Users\\u\\AppData\\Roaming\\Microsoft\\Protect\\blob",
        "/etc/passwd",
        "/root/.bashrc",
        "/proc/self/environ",
        "/sys/kernel",
        "C:/Windows/System32",
        "/ETC/shadow",
    ] {
        assert!(
            SecurityPolicy::is_always_forbidden(Path::new(forbidden)),
            "floor must forbid {forbidden}"
        );
    }
    for allowed in [
        "/home/u/project/src/main.rs",
        "/tmp/work/file",
        "/private/var/folders/x",
        "/home/u/.docker/other.json",
        "/etcetera/file",
        "/home/u/notes.ssh.txt",
    ] {
        assert!(
            !SecurityPolicy::is_always_forbidden(Path::new(allowed)),
            "floor must not forbid {allowed}"
        );
    }
}

#[test]
fn trusted_root_access_tiers_are_pinned() {
    let p = SecurityPolicy {
        enabled: true,
        trusted_roots: vec![
            TrustedRoot {
                path: "/data/ro".into(),
                access: TrustedAccess::Read,
            },
            TrustedRoot {
                path: "/data/rw".into(),
                access: TrustedAccess::ReadWrite,
            },
        ],
        ..SecurityPolicy::default()
    };
    assert!(p.is_within_trusted_root(Path::new("/data/ro/a"), false));
    assert!(!p.is_within_trusted_root(Path::new("/data/ro/a"), true));
    assert!(p.is_within_trusted_root(Path::new("/data/rw/a"), true));
    assert!(!p.is_within_trusted_root(Path::new("/data/other/a"), false));
}

#[test]
fn hourly_rate_limiter_counts_acts_only_and_is_inert_when_disabled() {
    let p = SecurityPolicy {
        enabled: true,
        autonomy: AutonomyLevel::Supervised,
        max_actions_per_hour: 3,
        ..SecurityPolicy::default()
    };
    for _ in 0..10 {
        assert!(p
            .enforce_tool_operation(ToolOperation::Read, "read")
            .is_ok());
    }
    assert_eq!(p.tracker.count(), 0, "reads never consume the budget");
    assert!(!p.is_rate_limited());
    for n in 1..=3 {
        assert!(
            p.enforce_tool_operation(ToolOperation::Act, "act").is_ok(),
            "act {n}"
        );
    }
    assert!(p.is_rate_limited());
    let err = p
        .enforce_tool_operation(ToolOperation::Act, "act")
        .unwrap_err();
    assert!(err.contains("Rate limit exceeded"), "{err}");
    assert!(err.contains("3 actions/hour"), "{err}");

    let off = SecurityPolicy {
        enabled: false,
        max_actions_per_hour: 1,
        ..SecurityPolicy::default()
    };
    for _ in 0..5 {
        assert!(off
            .enforce_tool_operation(ToolOperation::Act, "act")
            .is_ok());
    }
    assert!(!off.is_rate_limited());
}

#[test]
fn read_only_tier_blocks_acts_before_the_budget() {
    let p = SecurityPolicy {
        enabled: true,
        autonomy: AutonomyLevel::ReadOnly,
        max_actions_per_hour: 100,
        ..SecurityPolicy::default()
    };
    assert!(!p.can_act());
    let err = p
        .enforce_tool_operation(ToolOperation::Act, "write")
        .unwrap_err();
    assert!(err.contains(POLICY_BLOCKED_MARKER), "{err}");
    assert_eq!(p.tracker.count(), 0);
    assert!(p.enforce_write_tier("memory").is_err());
}
