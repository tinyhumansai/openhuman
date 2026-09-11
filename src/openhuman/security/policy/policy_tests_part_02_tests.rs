use super::*;

#[test]
fn always_forbidden_blocks_core_os_dirs_cross_platform() {
    use std::path::Path;
    for p in [
        "/etc/passwd",
        "/root/.bashrc",
        "/boot/x",
        "/proc/1",
        "/sys/x",
        "/System/Library/x",
        "C:\\Windows\\System32\\config",
        "C:\\WINDOWS\\x", // case-insensitive
        "C:\\Program Files\\App\\x",
        "C:\\ProgramData\\secret",
    ] {
        assert!(SecurityPolicy::is_always_forbidden(Path::new(p)), "{p}");
    }
}

#[test]
fn always_forbidden_leaves_gray_area_dirs_to_overridable_forbidden_paths() {
    use std::path::Path;
    // NOT unconditional — a trusted_root grant may reach these (e.g.
    // /usr/local, /opt, ~/Library, project dirs).
    for p in [
        "/usr/local/bin/tool",
        "/opt/app/x",
        "/var/data/x",
        "/Users/u/Library/Application Support/x",
        "/home/u/projects/myrepo/src/main.rs",
        "C:\\Users\\u\\projects\\app\\src",
    ] {
        assert!(!SecurityPolicy::is_always_forbidden(Path::new(p)), "{p}");
    }
}

// -- LLM escalate-only category (Phase G) -------------------------

#[test]
fn parse_declared_class_maps_known_and_rejects_unknown() {
    assert_eq!(
        SecurityPolicy::parse_declared_class("destructive"),
        Some(CommandClass::Destructive)
    );
    assert_eq!(
        SecurityPolicy::parse_declared_class("  WRITE "),
        Some(CommandClass::Write)
    );
    assert_eq!(
        SecurityPolicy::parse_declared_class("network"),
        Some(CommandClass::Network)
    );
    assert_eq!(
        SecurityPolicy::parse_declared_class("install"),
        Some(CommandClass::Install)
    );
    assert_eq!(SecurityPolicy::parse_declared_class("bogus"), None);
    assert_eq!(SecurityPolicy::parse_declared_class(""), None);
    // Escalate-only contract: max() raises but never lowers.
    assert_eq!(
        CommandClass::Write.max(CommandClass::Destructive),
        CommandClass::Destructive
    );
    assert_eq!(
        CommandClass::Destructive.max(CommandClass::Read),
        CommandClass::Destructive
    );
}

#[test]
fn command_risk_medium_for_command_executors() {
    // Interpreters / code executors are medium-risk now (not high): a coding
    // agent must be able to run them — prompted in Supervised, allowed in Full.
    let p = default_policy();
    for command in [
        "xargs rm",
        "awk 'BEGIN{system(\"id\")}'",
        "perl -e 'system \"id\"'",
        "python3 -c 'import os; os.system(\"id\")'",
        "pythonw3 -c 'import os; os.system(\"id\")'",
        "ruby -e 'system \"id\"'",
        "bash -lc 'id'",
        "sh -c 'id'",
        "C:\\Python312\\python.EXE -c 'print(1)'",
        "C:\\Python312\\pythonw3.12.exe -c 'print(1)'",
        "/usr/bin/env python3 -c 'print(1)'",
    ] {
        assert_eq!(
            p.command_risk_level(command),
            CommandRiskLevel::Medium,
            "{command} should be medium risk"
        );
    }
}

#[test]
fn validate_command_requires_approval_for_medium_risk() {
    let p = SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        require_approval_for_medium_risk: true,
        allowed_commands: vec!["touch".into()],
        ..SecurityPolicy::default()
    };

    let denied = p.validate_command_execution("touch test.txt", false);
    assert!(denied.is_err());
    assert!(denied.unwrap_err().contains("requires explicit approval"),);

    let allowed = p.validate_command_execution("touch test.txt", true);
    assert_eq!(allowed.unwrap(), CommandRiskLevel::Medium);
}

#[test]
fn validate_command_blocks_high_risk_by_default() {
    let p = SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        allowed_commands: vec!["rm".into()],
        ..SecurityPolicy::default()
    };

    let result = p.validate_command_execution("rm -rf /tmp/test", true);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("high-risk"));
}

#[test]
fn validate_command_full_mode_skips_medium_risk_approval_gate() {
    let p = SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        require_approval_for_medium_risk: true,
        allowed_commands: vec!["touch".into()],
        ..SecurityPolicy::default()
    };

    let result = p.validate_command_execution("touch test.txt", false);
    assert_eq!(result.unwrap(), CommandRiskLevel::Medium);
}

#[test]
fn validate_command_rejects_background_chain_bypass() {
    let p = default_policy();
    let result = p.validate_command_execution("ls & python3 -c 'print(1)'", false);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not allowed"));
}

// Regression: OPENHUMAN-TAURI-GW (#1813). A multi-byte UTF-8 char straddling
// byte 80 of the command string used to panic the log truncator with
// `byte index 80 is not a char boundary`, killing the core thread. All five
// `&command[..80]` log sites must now round down to a UTF-8 boundary.
#[test]
fn validate_command_does_not_panic_on_multibyte_char_at_log_truncation_boundary() {
    // Real-world Sentry repro: `cmd /c "dir /b "%USERPROFILE%\Desktop\*.lnk"
    // 2>nul | findstr /i "Warcraft WoW 魔兽 Battle"` — the 3-byte `'魔'`
    // occupies bytes 78..81, so a naked `&command[..80]` panics.
    let cmd = "cmd /c \"dir /b \"%USERPROFILE%\\Desktop\\*.lnk\" 2>nul | findstr /i \"Warcraft WoW 魔兽 Battle\"";
    assert!(
        cmd.len() > 80,
        "test fixture must be long enough to trigger truncation"
    );
    assert!(
        !cmd.is_char_boundary(80),
        "test fixture must place a multi-byte char across byte 80"
    );

    // Exercise the allowlist-deny path (cmd starts with "cmd" which is not on
    // the default allowlist), which fires the truncating warn! at policy.rs.
    let p = default_policy();
    let result = p.validate_command_execution(cmd, false);
    assert!(
        result.is_err(),
        "command should be blocked, but did not panic"
    );

    // And the high-risk-blocked path: allowlist passes (dd is allowed), then
    // risk gate fires (dd is a high-risk command), exercising the truncating
    // warn! site at the block_high_risk_commands branch.
    let prefix = "dd if=/dev/zero of=/dev/";
    let filler = "a".repeat(80 - prefix.len() - 1);
    let high_risk_cmd = format!("{prefix}{filler}魔");
    assert!(
        !high_risk_cmd.is_char_boundary(80),
        "fixture must straddle byte 80 with a multi-byte char"
    );
    let high_risk_policy = SecurityPolicy {
        allowed_commands: vec!["dd".into()],
        ..SecurityPolicy::default()
    };
    let blocked = high_risk_policy.validate_command_execution(&high_risk_cmd, true);
    assert!(blocked.is_err());
    assert!(blocked.unwrap_err().contains("high-risk"));
}

// Pathological short multi-byte command — exercises the boundary logic at the
// edge case where `cmd.len() < 80`.
#[test]
fn validate_command_handles_short_multibyte_command() {
    let p = default_policy();
    // 6 bytes (two 3-byte CJK chars) — well under the 80-byte log cap.
    let _ = p.validate_command_execution("魔兽", false);
}

// -- is_path_allowed ----------------------------------------------

#[test]
fn relative_paths_allowed() {
    let p = default_policy();
    assert!(p.is_path_string_allowed("file.txt"));
    assert!(p.is_path_string_allowed("src/main.rs"));
    assert!(p.is_path_string_allowed("deep/nested/dir/file.txt"));
}

#[test]
fn relative_personalities_path_resolves_under_action_dir() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path().join("state");
    let action = tmp.path().join("projects");
    std::fs::create_dir_all(workspace.join("personalities").join("alice")).unwrap();
    std::fs::create_dir_all(action.join("personalities")).unwrap();
    let policy = SecurityPolicy {
        workspace_dir: workspace.clone(),
        action_dir: action,
        ..SecurityPolicy::default()
    };

    assert!(policy.is_path_string_allowed("personalities/alice.md"));
    assert!(!policy.is_path_string_allowed(
        workspace
            .join("personalities")
            .join("alice")
            .join("SOUL.md")
            .to_string_lossy()
            .as_ref()
    ));
}

#[test]
fn path_traversal_blocked() {
    let p = default_policy();
    assert!(!p.is_path_string_allowed("../etc/passwd"));
    assert!(!p.is_path_string_allowed("../../root/.ssh/id_rsa"));
    assert!(!p.is_path_string_allowed("foo/../../../etc/shadow"));
    assert!(!p.is_path_string_allowed(".."));
}

#[test]
fn absolute_paths_blocked_when_workspace_only() {
    let p = default_policy();
    assert!(!p.is_path_string_allowed("/etc/passwd"));
    assert!(!p.is_path_string_allowed("/root/.ssh/id_rsa"));
    assert!(!p.is_path_string_allowed("/tmp/file.txt"));
}

#[test]
fn absolute_paths_allowed_when_not_workspace_only() {
    let p = SecurityPolicy {
        workspace_only: false,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };
    assert!(p.is_path_string_allowed("/tmp/file.txt"));
}

#[test]
fn forbidden_paths_blocked() {
    let p = SecurityPolicy {
        workspace_only: false,
        ..SecurityPolicy::default()
    };
    assert!(!p.is_path_string_allowed("/etc/passwd"));
    assert!(!p.is_path_string_allowed("/root/.bashrc"));
    assert!(!p.is_path_string_allowed("~/.ssh/id_rsa"));
    assert!(!p.is_path_string_allowed("~/.gnupg/pubring.kbx"));
}

#[test]
fn empty_path_allowed() {
    let p = default_policy();
    assert!(p.is_path_string_allowed(""));
}

#[test]
fn dotfile_in_workspace_allowed() {
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    std::fs::write(workspace.path().join(".gitignore"), "target/\n").expect("write .gitignore");
    std::fs::write(workspace.path().join(".env"), "LOCAL_ONLY=1\n").expect("write .env");
    let p = SecurityPolicy {
        workspace_dir: workspace.path().to_path_buf(),
        workspace_only: true,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };
    // .gitignore is a regular dotfile — allowed.
    assert!(p.is_path_string_allowed(".gitignore"));
    // .env is in WORKSPACE_INTERNAL_FILES: the agent must not read/write the
    // workspace's .env (may hold secrets / persona config).
    assert!(!p.is_path_string_allowed(".env"));
}

// -- is_path_allowed — symlink safety (#1927) ---------------------

#[cfg(unix)]
#[test]
fn symlink_inside_workspace_escaping_outside_is_blocked() {
    use std::os::unix::fs::symlink;

    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let outside = tempfile::tempdir().expect("outside tempdir");
    let target = outside.path().join("secret.txt");
    std::fs::write(&target, "secret").expect("write secret");

    let link = workspace.path().join("evil");
    symlink(&target, &link).expect("create symlink");

    let p = SecurityPolicy {
        workspace_dir: workspace.path().to_path_buf(),
        workspace_only: true,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };

    // String-level checks pass: "evil" has no "..", isn't absolute, and is
    // not in forbidden_paths. The canonicalize step must catch the symlink
    // pointing outside the workspace root.
    assert!(
        !p.is_path_string_allowed("evil"),
        "symlink that escapes the workspace must be blocked"
    );
}

#[cfg(unix)]
#[test]
fn symlink_to_forbidden_tree_is_blocked() {
    use std::os::unix::fs::symlink;

    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let forbidden = tempfile::tempdir().expect("forbidden tempdir");
    let target = forbidden.path().join("secret");
    std::fs::write(&target, "x").expect("write secret");

    let link = workspace.path().join("link-to-forbidden");
    symlink(&target, &link).expect("create symlink");

    let p = SecurityPolicy {
        workspace_dir: workspace.path().to_path_buf(),
        // Disable workspace_only so the assertion isolates the forbidden_paths
        // path (the symlink escapes the workspace, which would also trip
        // workspace_only — but here we want to prove the forbidden_paths
        // check itself canonicalizes).
        workspace_only: false,
        forbidden_paths: vec![forbidden.path().to_string_lossy().to_string()],
        ..SecurityPolicy::default()
    };

    // The string "link-to-forbidden" does not start with the forbidden
    // tempdir path, so the string-level check passes. Canonical resolution
    // must catch that it resolves into the forbidden tree.
    assert!(
        !p.is_path_string_allowed("link-to-forbidden"),
        "symlink that resolves into a forbidden tree must be blocked"
    );
}

#[test]
fn write_to_not_yet_existing_path_in_workspace_still_allowed() {
    // After adding the symlink-safe canonicalize step, writing to a
    // not-yet-existing path inside the workspace must still pass — the
    // parent-dir fallback canonicalizes the parent and confirms it is
    // inside the workspace root.
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let p = SecurityPolicy {
        workspace_dir: workspace.path().to_path_buf(),
        workspace_only: true,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };

    assert!(p.is_path_string_allowed("new-file.txt"));
    // Whole parent chain missing too — helper returns None, and we fall
    // back to the string-level checks (which would pass for a
    // workspace-relative non-traversal path).
    assert!(p.is_path_string_allowed("not-yet-existing/subdir/file.txt"));
}

// -- auto_approve defaults ----------------------------------------

#[test]
fn config_default_auto_approve_includes_expanded_tools() {
    // Issue #2486: verify read-only tools are auto-approved by default,
    // and write tools are NOT (Supervised mode must prompt for edits).
    let cfg = crate::openhuman::config::AutonomyConfig::default();

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
    let autonomy_config = crate::openhuman::config::AutonomyConfig {
        level: AutonomyLevel::Full,
        workspace_only: false,
        allowed_commands: vec!["docker".into()],
        forbidden_paths: vec!["/secret".into()],
        max_actions_per_hour: 100,
        max_cost_per_day_cents: 1000,
        require_approval_for_medium_risk: false,
        block_high_risk_commands: false,
        auto_approve: vec!["shell".into(), "file_write".into()],
        ..crate::openhuman::config::AutonomyConfig::default()
    };
    let workspace = PathBuf::from("/tmp/test-workspace");
    let policy = SecurityPolicy::from_config(&autonomy_config, &workspace, &workspace);

    assert_eq!(policy.autonomy, AutonomyLevel::Full);
    assert!(!policy.workspace_only);
    assert_eq!(policy.allowed_commands, vec!["docker"]);
    assert_eq!(policy.forbidden_paths, vec!["/secret"]);
    assert_eq!(policy.max_actions_per_hour, 100);
    assert_eq!(policy.max_cost_per_day_cents, 1000);
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

    let enabled_config = crate::openhuman::config::AutonomyConfig {
        auto_approve_all: true,
        ..crate::openhuman::config::AutonomyConfig::default()
    };
    let enabled_policy = SecurityPolicy::from_config(&enabled_config, &workspace, &workspace);
    assert!(enabled_policy.auto_approve_all);

    let disabled_config = crate::openhuman::config::AutonomyConfig {
        auto_approve_all: false,
        ..crate::openhuman::config::AutonomyConfig::default()
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
    assert!(p.max_cost_per_day_cents > 0);
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

#[test]
fn command_injection_semicolon_blocked() {
    let p = default_policy();
    // First word is "ls;" (with semicolon) — doesn't match "ls" in allowlist.
    // This is a safe default: chained commands are blocked.
    assert!(!p.is_command_allowed("ls; rm -rf /"));
}

#[test]
fn command_injection_semicolon_no_space() {
    let p = default_policy();
    assert!(!p.is_command_allowed("ls;rm -rf /"));
}

#[test]
fn quoted_semicolons_do_not_split_sqlite_command() {
    let p = SecurityPolicy {
        allowed_commands: vec!["sqlite3".into()],
        ..SecurityPolicy::default()
    };
    assert!(p.is_command_allowed(
        "sqlite3 /tmp/test.db \"CREATE TABLE t(id INT); INSERT INTO t VALUES(1); SELECT * FROM t;\""
    ));
    assert_eq!(
        p.command_risk_level(
            "sqlite3 /tmp/test.db \"CREATE TABLE t(id INT); INSERT INTO t VALUES(1); SELECT * FROM t;\""
        ),
        CommandRiskLevel::Low
    );
}

#[test]
fn unquoted_semicolon_after_quoted_sql_still_splits_commands() {
    let p = SecurityPolicy {
        allowed_commands: vec!["sqlite3".into()],
        ..SecurityPolicy::default()
    };
    assert!(!p.is_command_allowed("sqlite3 /tmp/test.db \"SELECT 1;\"; rm -rf /"));
}

#[test]
fn command_injection_backtick_blocked() {
    let p = default_policy();
    assert!(!p.is_command_allowed("echo `whoami`"));
    assert!(!p.is_command_allowed("echo `rm -rf /`"));
}

#[test]
fn command_injection_dollar_paren_blocked() {
    let p = default_policy();
    assert!(!p.is_command_allowed("echo $(cat /etc/passwd)"));
    assert!(!p.is_command_allowed("echo $(rm -rf /)"));
}

#[test]
fn command_with_env_var_prefix() {
    let p = default_policy();
    // "FOO=bar" is the first word — not in allowlist
    assert!(!p.is_command_allowed("FOO=bar rm -rf /"));
}

#[test]
fn command_newline_injection_blocked() {
    let p = default_policy();
    // Newline splits into two commands; "rm" is not in allowlist
    assert!(!p.is_command_allowed("ls\nrm -rf /"));
    // Both allowed — OK
    assert!(p.is_command_allowed("ls\necho hello"));
}

#[test]
fn command_injection_and_chain_blocked() {
    let p = default_policy();
    assert!(!p.is_command_allowed("ls && rm -rf /"));
    assert!(!p.is_command_allowed("echo ok && curl http://evil.com"));
    // Both allowed — OK
    assert!(p.is_command_allowed("ls && echo done"));
}

#[test]
fn command_injection_or_chain_blocked() {
    let p = default_policy();
    assert!(!p.is_command_allowed("ls || rm -rf /"));
    // Both allowed — OK
    assert!(p.is_command_allowed("ls || echo fallback"));
}
