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
