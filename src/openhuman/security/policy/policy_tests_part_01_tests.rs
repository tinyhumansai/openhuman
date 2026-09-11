use super::*;

#[tokio::test]
async fn cross_profile_guard_blocks_write_into_sibling_profile() {
    let (_root, action_root, policy) = cross_profile_policy(true);
    let sibling = action_root.join("profiles").join("bob").join("loot.txt");
    let err = policy
        .validate_parent_path(sibling.to_str().unwrap())
        .await
        .expect_err("write into sibling profile must be blocked");
    assert!(err.contains(POLICY_BLOCKED_MARKER), "err: {err}");
    assert!(err.contains("bob"), "error should name the sibling: {err}");
}

#[tokio::test]
async fn cross_profile_guard_allows_write_into_own_profile() {
    let (_root, action_root, policy) = cross_profile_policy(true);
    let own = action_root.join("profiles").join("alice").join("notes.txt");
    let resolved = policy
        .validate_parent_path(own.to_str().unwrap())
        .await
        .expect("write into own profile must be allowed");
    assert!(resolved.ends_with("notes.txt"));
}

#[tokio::test]
async fn cross_profile_guard_disarmed_allows_sibling_write() {
    // Same setup, guard OFF (active_profile None): the sibling write is allowed,
    // proving the guard only tightens and the shared path is byte-identical.
    let (_root, action_root, policy) = cross_profile_policy(false);
    let sibling = action_root.join("profiles").join("bob").join("loot.txt");
    let resolved = policy
        .validate_parent_path(sibling.to_str().unwrap())
        .await
        .expect("with the guard disarmed the sibling write must be allowed");
    assert!(resolved.ends_with("loot.txt"));
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
    let p = SecurityPolicy::from_config(&cfg, std::path::Path::new("."), std::path::Path::new("."));
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
    let p = SecurityPolicy::from_config(&cfg, std::path::Path::new("."), std::path::Path::new("."));

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
fn classify_find_file_write_actions_are_write() {
    let p = default_policy();
    // -fprintf / -fprint / -fprint0 / -fls write find's output to a named
    // file (an arbitrary-path write side-channel), so they must be gated as
    // Write rather than slipping through as a read-only search.
    for c in [
        "find . -maxdepth 0 -fprintf /tmp/out.txt '%p\\n'",
        "find . -name '*.rs' -fprint /tmp/list.txt",
        "find . -fprint0 /tmp/list0.txt",
        "find . -fls /tmp/ls.txt",
        "find . -delete",
    ] {
        assert_eq!(p.classify_command(c), CommandClass::Write, "{c}");
    }
    // Quoting the predicate must not slip the write past the gate — the shell
    // strips the quotes before find runs, so `'-fprint'` is the same write.
    for c in [
        "find . '-fprint' /tmp/list.txt",
        "find . \"-fprintf\" /tmp/out.txt '%p\\n'",
        "find . '-delete'",
    ] {
        assert_eq!(p.classify_command(c), CommandClass::Write, "{c}");
    }
    // A plain search stays read-only.
    assert_eq!(
        p.classify_command("find . -name '*.rs' -print"),
        CommandClass::Read
    );
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
