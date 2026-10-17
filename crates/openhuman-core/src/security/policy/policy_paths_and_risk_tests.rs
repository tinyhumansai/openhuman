use super::*;
use tinybox_bus::CommandClass;

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

#[cfg(feature = "modules")]
#[test]
fn tinybox_command_facts_drive_policy_and_keep_the_path_floor_when_disabled() {
    use crate::modules::tinybox::contract::{CommandAnalysis, CommandSegment};

    let write_segment = CommandSegment {
        source: "ls -la".into(),
        command: "ls -la".into(),
        basename: "ls".into(),
        normalized_name: "ls".into(),
        arguments: vec!["-la".into()],
        class: CommandClass::Write,
        leading_env_assignment: false,
        dangerous_env_prefix: false,
        executor: false,
    };
    let analysis = CommandAnalysis {
        segments: vec![write_segment],
        hidden_execution: false,
        redirection: false,
        expansion: false,
        tee: false,
        background: false,
        literal_words: vec!["ls".into(), "-la".into()],
    };
    let supervised = SecurityPolicy {
        enabled: true,
        autonomy: AutonomyLevel::Supervised,
        ..SecurityPolicy::default()
    };
    assert!(supervised.is_command_allowed_analysis(&analysis));
    assert!(!supervised.is_command_allowed_analysis(&CommandAnalysis {
        redirection: true,
        ..analysis.clone()
    }));
    assert!(!supervised.is_command_allowed_analysis(&CommandAnalysis {
        segments: vec![CommandSegment {
            leading_env_assignment: true,
            ..analysis.segments[0].clone()
        }],
        ..analysis.clone()
    }));
    assert!(!supervised.is_command_allowed_analysis(&CommandAnalysis {
        segments: vec![CommandSegment {
            normalized_name: "python3".into(),
            executor: true,
            ..analysis.segments[0].clone()
        }],
        ..analysis.clone()
    }));

    let readonly = SecurityPolicy {
        enabled: true,
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    };
    assert!(readonly.check_gated_command_analysis(&analysis).is_err());

    let disabled = SecurityPolicy {
        enabled: false,
        ..SecurityPolicy::default()
    };
    let protected = CommandAnalysis {
        literal_words: vec!["cat".into(), "~/.ssh/id_rsa".into()],
        ..analysis
    };
    assert!(disabled.check_gated_command_analysis(&protected).is_err());

    let traversal = CommandAnalysis {
        literal_words: vec!["cat".into(), "workspace/../.ssh/id_rsa".into()],
        ..protected.clone()
    };
    assert!(disabled.check_gated_command_analysis(&traversal).is_err());

    let nul_path = CommandAnalysis {
        literal_words: vec!["cat".into(), "safe\0/credential".into()],
        ..protected
    };
    assert!(disabled.check_gated_command_analysis(&nul_path).is_err());
}

#[cfg(feature = "modules")]
#[test]
fn tinybox_fact_allowlist_treats_heredoc_as_data_and_unknown_commands_as_writes() {
    use crate::modules::tinybox::contract::{CommandAnalysis, CommandSegment};

    let policy = SecurityPolicy {
        enabled: true,
        autonomy: AutonomyLevel::Supervised,
        ..SecurityPolicy::default()
    };
    // This is the `AnalyzeCommand` reply shape for a quoted heredoc: body text
    // does not appear among live segments or literal words, while the command
    // that writes the document remains classified as a write.
    let heredoc = CommandAnalysis {
        segments: vec![CommandSegment {
            source: "cat > notes.txt << 'EOF'".into(),
            command: "cat > notes.txt".into(),
            basename: "cat".into(),
            normalized_name: "cat".into(),
            arguments: vec!["notes.txt".into()],
            class: CommandClass::Write,
            leading_env_assignment: false,
            dangerous_env_prefix: false,
            executor: false,
        }],
        hidden_execution: false,
        redirection: true,
        expansion: false,
        tee: false,
        background: false,
        literal_words: vec!["cat".into(), "notes.txt".into()],
    };
    assert!(!policy.is_command_allowed_analysis(&heredoc));

    // The module's conservative unknown-command default is Write. OpenHuman's
    // supervised allowlist still rejects a name it does not explicitly allow.
    let unknown_write = CommandAnalysis {
        segments: vec![CommandSegment {
            source: "unrecognized-tool --flag".into(),
            command: "unrecognized-tool --flag".into(),
            basename: "unrecognized-tool".into(),
            normalized_name: "unrecognized-tool".into(),
            arguments: vec!["--flag".into()],
            class: CommandClass::Write,
            leading_env_assignment: false,
            dangerous_env_prefix: false,
            executor: false,
        }],
        hidden_execution: false,
        redirection: false,
        expansion: false,
        tee: false,
        background: false,
        literal_words: vec!["unrecognized-tool".into(), "--flag".into()],
    };
    assert!(!policy.is_command_allowed_analysis(&unknown_write));
    assert_eq!(
        policy.check_gated_command_analysis(&unknown_write),
        Ok(CommandClass::Write)
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
