use super::*;

// ── action sandbox (issue #3052) ──────────────────────────────────────────

#[test]
fn is_workspace_internal_path_blocks_state_dirs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ws = tmp.path().to_path_buf();
    std::fs::create_dir_all(ws.join("memory")).expect("create memory dir");
    std::fs::create_dir_all(ws.join("sessions")).expect("create sessions dir");
    std::fs::create_dir_all(ws.join("state")).expect("create state dir");
    std::fs::create_dir_all(ws.join("cron")).expect("create cron dir");
    let policy = SecurityPolicy {
        workspace_dir: ws.clone(),
        action_dir: ws.join("action"),
        ..SecurityPolicy::default()
    };
    assert!(policy.is_workspace_internal_path(&ws.join("memory")));
    assert!(policy.is_workspace_internal_path(&ws.join("memory").join("namespaces")));
    assert!(policy.is_workspace_internal_path(&ws.join("memory-alice").join("memory.db")));
    assert!(policy.is_workspace_internal_path(&ws.join("memory_tree-alice").join("tree")));
    assert!(policy.is_workspace_internal_path(
        &ws.join("session_raw-alice")
            .join("1700000000_orchestrator.jsonl")
    ));
    assert!(policy.is_workspace_internal_path(&ws.join("sessions")));
    assert!(policy.is_workspace_internal_path(&ws.join("state")));
    assert!(policy.is_workspace_internal_path(&ws.join("cron")));
    assert!(policy.is_workspace_internal_path(&ws.join("memory_tree")));
    assert!(
        policy.is_workspace_internal_path(&ws.join("personalities").join("alice").join("SOUL.md"))
    );
    assert!(policy.is_workspace_internal_path(
        &ws.join("personalities")
            .join("alice")
            .join("skills")
            .join("private-skill")
            .join("SKILL.md")
    ));
    assert!(policy.is_workspace_internal_path(&ws.join("approval")));
    assert!(policy.is_workspace_internal_path(&ws.join("mcp_clients")));
    assert!(policy.is_workspace_internal_path(&ws.join("codegraph")));
}

#[test]
fn is_workspace_internal_path_blocks_state_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ws = tmp.path().to_path_buf();
    std::fs::File::create(ws.join("core.token")).expect("create core.token");
    let policy = SecurityPolicy {
        workspace_dir: ws.clone(),
        action_dir: ws.join("action"),
        ..SecurityPolicy::default()
    };
    assert!(policy.is_workspace_internal_path(&ws.join("core.token")));
    assert!(policy.is_workspace_internal_path(&ws.join("dev-keychain.json")));
    assert!(policy.is_workspace_internal_path(&ws.join("SOUL.md")));
    assert!(policy.is_workspace_internal_path(&ws.join(".env")));
}

#[test]
fn is_workspace_internal_path_allows_non_internal() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ws = tmp.path().to_path_buf();
    std::fs::create_dir_all(ws.join("projects")).expect("create projects dir");
    let policy = SecurityPolicy {
        workspace_dir: ws.clone(),
        action_dir: ws.join("action"),
        ..SecurityPolicy::default()
    };
    assert!(!policy.is_workspace_internal_path(&ws.join("projects")));
    assert!(!policy.is_workspace_internal_path(&ws.join("projects").join("my-app")));
    assert!(!policy.is_workspace_internal_path(&std::env::temp_dir().join("other")));
}

#[test]
fn is_path_string_allowed_blocks_workspace_internal() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ws = tmp.path().to_path_buf();
    std::fs::create_dir_all(ws.join("memory")).expect("create memory dir");
    let policy = SecurityPolicy {
        workspace_dir: ws.clone(),
        action_dir: ws.join("action"),
        workspace_only: false,
        ..SecurityPolicy::default()
    };
    let memory_path = ws.join("memory").join("test.db");
    assert!(
        !policy.is_path_string_allowed(&memory_path.to_string_lossy()),
        "absolute path to workspace internal dir should be blocked"
    );
}

/// #5505: an artifact's `meta.json` / `args.json` name the file Download and
/// `read_artifact_bytes` follow, so the agent's file tools must not write them
/// even through a trusted root that reaches the workspace, while the
/// `artifacts/tool-results/` read-back stays reachable.
#[tokio::test]
async fn artifact_metadata_is_internal_but_tool_results_are_not() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ws = tmp.path().join("workspace");
    let record = ws.join("artifacts").join("abc");
    let results = ws.join("artifacts").join("tool-results").join("s1");
    std::fs::create_dir_all(&record).expect("create record dir");
    std::fs::create_dir_all(&results).expect("create tool-results dir");
    std::fs::write(record.join("meta.json"), "{}").expect("write meta");
    std::fs::write(results.join("call-1.txt"), "output").expect("write result");
    let policy = SecurityPolicy {
        workspace_dir: ws.clone(),
        action_dir: ws.clone(),
        workspace_only: false,
        trusted_roots: vec![TrustedRoot {
            path: ws.to_string_lossy().into_owned(),
            access: TrustedAccess::ReadWrite,
        }],
        ..SecurityPolicy::default()
    };

    let meta = record.join("meta.json");
    assert!(!policy.is_path_string_allowed(&meta.to_string_lossy()));
    assert!(policy.validate_path(&meta.to_string_lossy()).await.is_err());
    assert!(policy
        .validate_parent_path(&record.join("args.json").to_string_lossy())
        .await
        .is_err());

    let result = results.join("call-1.txt");
    assert!(policy.is_path_string_allowed(&result.to_string_lossy()));
    assert!(policy
        .validate_path(&result.to_string_lossy())
        .await
        .is_ok());
}

#[tokio::test]
async fn trusted_root_cannot_expose_workspace_internal_state() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ws = tmp.path().join("workspace");
    let personality = ws.join("personalities").join("alice");
    std::fs::create_dir_all(&personality).expect("create profile home");
    let soul = personality.join("SOUL.md");
    std::fs::write(&soul, "private identity").expect("write soul");
    let policy = SecurityPolicy {
        workspace_dir: ws.clone(),
        action_dir: ws.clone(),
        workspace_only: false,
        trusted_roots: vec![TrustedRoot {
            path: ws.to_string_lossy().into_owned(),
            access: TrustedAccess::ReadWrite,
        }],
        ..SecurityPolicy::default()
    };

    assert!(!policy.is_path_string_allowed(&soul.to_string_lossy()));
    assert!(policy.validate_path(&soul.to_string_lossy()).await.is_err());
    assert!(policy
        .validate_parent_path(
            &ws.join("session_raw-alice")
                .join("new.jsonl")
                .to_string_lossy()
        )
        .await
        .is_err());
}

#[test]
fn action_dir_in_default_policy() {
    let policy = SecurityPolicy::default();
    assert_eq!(policy.action_dir, std::path::PathBuf::from("."));
}

/// Without a scoped root the checkout is simply outside the workspace, and a
/// write into it is refused — the behaviour every existing caller keeps.
#[tokio::test]
async fn write_outside_the_workspace_is_refused_without_a_turn_root() {
    let (_root, checkout, policy) = turn_workspace_policy();
    let target = checkout.join("notes.md");
    let err = policy
        .validate_parent_path(target.to_str().unwrap())
        .await
        .expect_err("a path outside the workspace must not validate");
    assert!(
        err.contains(POLICY_BLOCKED_MARKER),
        "unexpected error: {err}"
    );
}

/// With the checkout scoped as this turn's workspace, the same write validates:
/// the grant is what lets an embedded turn work in the tree its host named.
#[tokio::test]
async fn write_into_the_scoped_turn_root_is_allowed() {
    let (_root, checkout, policy) = turn_workspace_policy();
    let target = checkout.join("notes.md");
    let resolved = crate::agent::turn_workspace::with_workspace(checkout.clone(), {
        let policy = &policy;
        let target = target.clone();
        async move { policy.validate_parent_path(target.to_str().unwrap()).await }
    })
    .await
    .expect("the scoped turn root must be writable");
    assert!(resolved.ends_with("notes.md"), "resolved: {resolved:?}");
}

/// The grant covers its own subtree and nothing beside it: a sibling directory
/// stays outside, so scoping one checkout does not open the whole parent.
#[tokio::test]
async fn the_turn_root_grant_does_not_reach_a_sibling_directory() {
    let (root, checkout, policy) = turn_workspace_policy();
    let sibling = root.path().join("other");
    std::fs::create_dir_all(&sibling).unwrap();
    let target = sibling.join("notes.md");
    let err = crate::agent::turn_workspace::with_workspace(checkout, {
        let policy = &policy;
        let target = target.clone();
        async move { policy.validate_parent_path(target.to_str().unwrap()).await }
    })
    .await
    .expect_err("a sibling of the scoped root must stay outside it");
    assert!(
        err.contains(POLICY_BLOCKED_MARKER),
        "unexpected error: {err}"
    );
}

/// The grant is exactly as strong as a configured trusted root and no stronger:
/// a credential store under the scoped tree is still unreachable.
#[tokio::test]
async fn the_turn_root_grant_never_reaches_a_credential_store() {
    let (_root, checkout, policy) = turn_workspace_policy();
    let ssh = checkout.join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    let target = ssh.join("id_rsa");
    let err = crate::agent::turn_workspace::with_workspace(checkout, {
        let policy = &policy;
        let target = target.clone();
        async move { policy.validate_parent_path(target.to_str().unwrap()).await }
    })
    .await
    .expect_err("credential stores stay forbidden inside a granted root");
    assert!(
        err.contains(POLICY_BLOCKED_MARKER),
        "unexpected error: {err}"
    );
}

/// #5505: the account config beside the workspace holds the autonomy policy
/// and the files folders the artifact escape guard trusts, so a trusted root
/// over the account dir must not make it readable or writable, while other
/// files there stay reachable.
#[tokio::test]
async fn the_account_config_beside_the_workspace_is_internal() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let account = tmp.path().join("users").join("u1");
    let ws = account.join("workspace");
    std::fs::create_dir_all(&ws).expect("create workspace");
    let config = account.join("config.toml");
    std::fs::write(&config, "files_dir_history = []").expect("write config");
    let notes = account.join("notes.txt");
    std::fs::write(&notes, "hello").expect("write notes");
    let policy = SecurityPolicy {
        workspace_dir: ws.clone(),
        action_dir: ws.clone(),
        workspace_only: false,
        trusted_roots: vec![TrustedRoot {
            path: account.to_string_lossy().into_owned(),
            access: TrustedAccess::ReadWrite,
        }],
        ..SecurityPolicy::default()
    };

    assert!(!policy.is_path_string_allowed(&config.to_string_lossy()));
    assert!(policy
        .validate_path(&config.to_string_lossy())
        .await
        .is_err());
    assert!(policy
        .validate_parent_path(&config.to_string_lossy())
        .await
        .is_err());
    assert!(policy.validate_path(&notes.to_string_lossy()).await.is_ok());
}

/// #5505 carved out `config.toml` alone, leaving four siblings that carry
/// secrets or privilege agent-reachable: the keyring's on-disk encryption key,
/// the credential profiles it decrypts, the config one save behind, and the
/// Claude Code `full_access` toggle (which switches that provider to
/// `--permission-mode bypassPermissions` with its full native toolset, whose
/// calls never reach the approval gate).
#[test]
fn account_dir_secrets_and_privilege_files_are_internal() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let account = tmp.path().join("users").join("u1");
    let ws = account.join("workspace");
    std::fs::create_dir_all(&ws).expect("create workspace");
    let policy = SecurityPolicy {
        workspace_dir: ws,
        action_dir: tmp.path().join("projects"),
        ..SecurityPolicy::default()
    };

    // Pinned before this change, and unchanged by it.
    assert!(policy.is_workspace_internal_path(&account.join("config.toml")));
    // Each of these was reachable before it.
    for name in [
        "config.toml.bak",
        ".secret_key",
        "auth-profiles.json",
        "claude_code_settings.json",
    ] {
        assert!(
            policy.is_workspace_internal_path(&account.join(name)),
            "{name} must not be part of the agent's action surface"
        );
    }
}

/// The account dir stays reachable otherwise — that is deliberate (#5505), and
/// the carve-out must not widen into a containment boundary. A same-named file
/// elsewhere is likewise none of the policy's business.
#[test]
fn account_dir_carve_out_does_not_capture_unrelated_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let account = tmp.path().join("users").join("u1");
    let ws = account.join("workspace");
    let action = tmp.path().join("projects");
    std::fs::create_dir_all(&ws).expect("create workspace");
    std::fs::create_dir_all(&action).expect("create action dir");
    let policy = SecurityPolicy {
        workspace_dir: ws,
        action_dir: action.clone(),
        ..SecurityPolicy::default()
    };

    assert!(!policy.is_workspace_internal_path(&account.join("notes.txt")));
    assert!(!policy.is_workspace_internal_path(&account.join("checkout")));
    // A project of the user's own that happens to carry one of these names is
    // not account state — the match is scoped to the account dir itself.
    assert!(!policy.is_workspace_internal_path(&action.join("claude_code_settings.json")));
    assert!(!policy.is_workspace_internal_path(&action.join("app").join(".secret_key")));
}

/// End to end through the gate every file tool goes through, with a trusted
/// root over the account dir — the configuration the carve-out exists for.
#[tokio::test]
async fn trusted_root_over_the_account_dir_cannot_reach_the_keyring_key() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let account = tmp.path().join("users").join("u1");
    let ws = account.join("workspace");
    std::fs::create_dir_all(&ws).expect("create workspace");
    let key = account.join(".secret_key");
    std::fs::write(&key, "key material").expect("write key");
    let settings = account.join("claude_code_settings.json");
    std::fs::write(&settings, "{}").expect("write settings");
    let policy = SecurityPolicy {
        workspace_dir: ws,
        action_dir: tmp.path().join("projects"),
        workspace_only: false,
        trusted_roots: vec![TrustedRoot {
            path: account.to_string_lossy().into_owned(),
            access: TrustedAccess::ReadWrite,
        }],
        ..SecurityPolicy::default()
    };

    for path in [&key, &settings] {
        let shown = path.to_string_lossy();
        assert!(!policy.is_path_string_allowed(&shown), "{shown}");
        assert!(policy.validate_path(&shown).await.is_err(), "{shown}");
        assert!(
            policy.validate_parent_path(&shown).await.is_err(),
            "{shown}"
        );
    }
}
