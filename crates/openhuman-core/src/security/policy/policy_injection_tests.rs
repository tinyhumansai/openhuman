use super::*;

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
    assert_eq!(p.expand_tilde(input), crate::config::expand_tilde(input));
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
    let autonomy_config = crate::config::AutonomyConfig {
        level: AutonomyLevel::Full,
        workspace_only: false,
        allowed_commands: vec![],
        forbidden_paths: vec![],
        max_actions_per_hour: 10,
        require_approval_for_medium_risk: true,
        block_high_risk_commands: true,
        ..crate::config::AutonomyConfig::default()
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

// -- quoted heredoc bodies are data, not shell -------------------------------
//
// Regression for the life-scenario `meal-plan` failure: a five-day dinner plan
// was refused as "background (&) is not allowed" because four recipe titles
// read "Chicken & Spinach". `classify_command` and the structural guard scan
// the raw command string, and until `strip_quoted_heredoc_bodies` existed a
// `<< 'EOF'` body — where the shell expands nothing — was scanned as live text.

/// The command as the model actually sent it, trimmed to the offending lines.
const MEAL_PLAN_HEREDOC: &str = r#"cat > out/meal_plan.md << 'EOF'
# 5-Day Mediterranean Dinner Plan (Family of 4)

## Day 1 — Greek Chicken & Spinach Orzo Skillet
## Day 3 — Spinach & Feta Stuffed Chicken
## Day 5 — Shrimp & Vegetable Mediterranean Pasta
EOF"#;
