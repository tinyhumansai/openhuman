use super::*;

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

    // All leading env-var assignments are now rejected — including
    // previously-benign-looking ones (TZ, LANG, LC_ALL, custom names).
    // The allowlist already names every command we want to permit, and
    // none need an operator-set env var at invoke time, so the broader
    // gate has no false-positive surface on the approved path.
    assert!(!p.is_command_allowed("TZ=UTC git log"));
    assert!(!p.is_command_allowed("LANG=en_US.UTF-8 git log"));
    assert!(!p.is_command_allowed("LC_ALL=C git status"));
    assert!(!p.is_command_allowed("FOO=bar git status"));
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
fn command_env_var_prefix_is_always_rejected() {
    let p = default_policy();
    // ANY env assignment is rejected — including in front of an
    // otherwise-allowed command. Helper-style exec primitives
    // (GIT_SSH=, SSH_ASKPASS=, LD_PRELOAD=) and benign-looking
    // overrides (FOO=, LANG=) both go through the same gate so the
    // policy doesn't have to enumerate every shape of every
    // downstream tool's hook surface.
    assert!(!p.is_command_allowed("FOO=bar ls"));
    assert!(!p.is_command_allowed("LANG=C grep pattern file"));
    assert!(!p.is_command_allowed("FOO=bar rm -rf /"));
}

#[test]
fn validate_command_rejects_leading_env_var_assignment() {
    let p = default_policy();
    // Helper-style exec primitives that mutate which binary the
    // approved command actually runs as: rejected.
    assert!(!p.is_command_allowed("GIT_SSH=./wrapper.sh git ls-remote ssh://x"));
    assert!(!p.is_command_allowed("SSH_ASKPASS=./y ssh user@host"));
    assert!(!p.is_command_allowed("LD_PRELOAD=./libx.so ls"));
    // Negative: same command without the env prefix passes the
    // structural guard (it may still fail later on its own merits,
    // but the env-prefix gate doesn't fire).
    assert!(p.is_command_allowed("git ls-remote ssh://example.com"));
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
fn expand_tilde_delegates_to_config_single_source_of_truth() {
    // The policy method must stay byte-for-byte identical to the canonical
    // config helper so path checks and config expansion never diverge (#3353).
    let p = SecurityPolicy::default();
    let input = "~/OpenHuman/projects";
    assert_eq!(
        p.expand_tilde(input),
        crate::openhuman::config::expand_tilde(input)
    );
    // Non-tilde inputs are returned unchanged on both sides.
    assert_eq!(p.expand_tilde("/abs"), "/abs");
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
    let policy = SecurityPolicy::from_config(&autonomy_config, &workspace, &workspace);
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
