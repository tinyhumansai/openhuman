use super::*;

#[test]
fn config_default_auto_approve_includes_expanded_tools() {
    // Issue #2486: verify read-only tools are auto-approved by default,
    // and write tools are NOT (Supervised mode must prompt for edits).
    // Explicitly enabled: the allowlist / risk gate this test is about only
    // exists when the policy is on, and the shipped config default is off.
    let cfg = crate::config::AutonomyConfig {
        enabled: true,
        ..crate::config::AutonomyConfig::default()
    };

    // Pre-existing auto-approved tools must still be present
    for tool in [
        "file_read",
        "memory_search",
        "memory_list",
        "get_time",
        "list_dir",
    ] {
        assert!(
            cfg.auto_approve.iter().any(|t| t == tool),
            "default auto_approve must still include pre-existing tool: {tool}"
        );
    }

    // Newly added read-only workspace-scoped tools
    for tool in ["glob", "grep"] {
        assert!(
            cfg.auto_approve.iter().any(|t| t == tool),
            "default auto_approve must include newly added tool: {tool}"
        );
    }

    // Write tools must NOT be auto-approved (v4→v5 migration strips these)
    for tool in ["file_write", "edit_file"] {
        assert!(
            !cfg.auto_approve.iter().any(|t| t == tool),
            "write tool {tool} must NOT be auto-approved by default"
        );
    }
}

// -- from_config --------------------------------------------------

#[test]
fn from_config_maps_all_fields() {
    let autonomy_config = crate::config::AutonomyConfig {
        level: AutonomyLevel::Full,
        workspace_only: false,
        allowed_commands: vec!["docker".into()],
        forbidden_paths: vec!["/secret".into()],
        max_actions_per_hour: 100,
        require_approval_for_medium_risk: false,
        block_high_risk_commands: false,
        auto_approve: vec!["shell".into(), "file_write".into()],
        ..crate::config::AutonomyConfig::default()
    };
    let workspace = PathBuf::from("/tmp/test-workspace");
    let policy = SecurityPolicy::from_config(&autonomy_config, &workspace, &workspace);

    assert_eq!(policy.autonomy, AutonomyLevel::Full);
    assert!(!policy.workspace_only);
    assert_eq!(policy.allowed_commands, vec!["docker"]);
    assert_eq!(policy.forbidden_paths, vec!["/secret"]);
    assert_eq!(policy.max_actions_per_hour, 100);
    assert!(!policy.require_approval_for_medium_risk);
    assert!(!policy.block_high_risk_commands);
    assert_eq!(policy.workspace_dir, PathBuf::from("/tmp/test-workspace"));
    // The "Always allow" allowlist is carried onto the policy so the gate can
    // skip prompting for these tools.
    assert_eq!(policy.auto_approve, vec!["shell", "file_write"]);
}

#[test]
fn policy_from_config_carries_auto_approve_all() {
    let workspace = PathBuf::from("/tmp/test-workspace");

    let enabled_config = crate::config::AutonomyConfig {
        auto_approve_all: true,
        ..crate::config::AutonomyConfig::default()
    };
    let enabled_policy = SecurityPolicy::from_config(&enabled_config, &workspace, &workspace);
    assert!(enabled_policy.auto_approve_all);

    let disabled_config = crate::config::AutonomyConfig {
        auto_approve_all: false,
        ..crate::config::AutonomyConfig::default()
    };
    let disabled_policy = SecurityPolicy::from_config(&disabled_config, &workspace, &workspace);
    assert!(!disabled_policy.auto_approve_all);
}

// -- Default policy -----------------------------------------------

#[test]
fn default_policy_has_sane_values() {
    let p = SecurityPolicy::default();
    assert_eq!(p.autonomy, AutonomyLevel::Supervised);
    assert!(p.workspace_only);
    assert!(!p.allowed_commands.is_empty());
    assert!(!p.forbidden_paths.is_empty());
    assert!(p.max_actions_per_hour > 0);
    assert!(p.require_approval_for_medium_risk);
    assert!(p.block_high_risk_commands);
}

// -- ActionTracker / rate limiting --------------------------------

#[test]
fn action_tracker_starts_at_zero() {
    let tracker = ActionTracker::new();
    assert_eq!(tracker.count(), 0);
}

#[test]
fn action_tracker_records_actions() {
    let tracker = ActionTracker::new();
    assert_eq!(tracker.record(), 1);
    assert_eq!(tracker.record(), 2);
    assert_eq!(tracker.record(), 3);
    assert_eq!(tracker.count(), 3);
}

#[test]
fn record_action_allows_within_limit() {
    let p = SecurityPolicy {
        max_actions_per_hour: 5,
        ..SecurityPolicy::default()
    };
    for _ in 0..5 {
        assert!(p.record_action(), "should allow actions within limit");
    }
}

#[test]
fn record_action_blocks_over_limit() {
    let p = SecurityPolicy {
        max_actions_per_hour: 3,
        ..SecurityPolicy::default()
    };
    assert!(p.record_action()); // 1
    assert!(p.record_action()); // 2
    assert!(p.record_action()); // 3
    assert!(!p.record_action()); // 4 — over limit
}

#[test]
fn is_rate_limited_reflects_count() {
    let p = SecurityPolicy {
        max_actions_per_hour: 2,
        ..SecurityPolicy::default()
    };
    assert!(!p.is_rate_limited());
    p.record_action();
    assert!(!p.is_rate_limited());
    p.record_action();
    assert!(p.is_rate_limited());
}

#[test]
fn action_tracker_clone_is_independent() {
    let tracker = ActionTracker::new();
    tracker.record();
    tracker.record();
    let cloned = tracker.clone();
    assert_eq!(cloned.count(), 2);
    tracker.record();
    assert_eq!(tracker.count(), 3);
    assert_eq!(cloned.count(), 2); // clone is independent
}

// -- Edge cases: command injection --------------------------------
