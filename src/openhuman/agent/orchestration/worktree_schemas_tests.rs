use super::*;
use std::process::Command;

/// `true` when `git` is invokable on this host. Tests that need real
/// `git worktree` plumbing skip (pass trivially) when it's absent, so a
/// git-less CI image doesn't hard-fail — same convention as
/// `worktree_tests.rs`.
fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git invocation");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Temp git repo with one commit. Returns the guard (kept alive by the
/// caller) and the repo root.
fn init_repo() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();
    git(&root, &["init", "-b", "main"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "Test User"]);
    std::fs::write(root.join("README.md"), "hello\n").unwrap();
    git(&root, &["add", "README.md"]);
    git(&root, &["commit", "-m", "initial"]);
    (tmp, root)
}

#[test]
fn list_view_degrades_to_empty_for_non_git_root() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    // A plain (non-git) directory must yield an empty list, not an error —
    // the panel shows "no worktrees" rather than surfacing a failure.
    let v = list_view(tmp.path(), "cid").expect("non-git root degrades cleanly");
    assert_eq!(v["worktrees"], json!([]));
    assert_eq!(v["overlaps"], json!([]));
}

#[test]
fn list_view_surfaces_managed_worktree() {
    if !git_available() {
        return;
    }
    let (_tmp, root) = init_repo();
    let st = worktree::create(&root, "run-1", worktree::BaseRef::Head).expect("create");
    assert!(
        is_managed_worktree(&st.path),
        "created under .claude/worktrees"
    );

    let v = list_view(&root, "cid").expect("list ok");
    let worktrees = v["worktrees"].as_array().expect("array");
    assert_eq!(worktrees.len(), 1, "the one managed worktree is listed");
    // A fresh worktree is clean → no overlaps.
    assert_eq!(v["overlaps"], json!([]));
}

/// Drives the full `handle_list` async handler (the public RPC entry point)
/// through `repo_root` → config load, anchored on a non-git
/// `OPENHUMAN_ACTION_DIR`. Confirms the panel-facing path degrades to an
/// empty list rather than erroring. Holds `TEST_ENV_LOCK` because the env
/// override is process-global.
#[tokio::test]
async fn handle_list_degrades_for_non_git_action_dir() {
    let _env = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let action_dir = tmp.path().join("actions");
    std::fs::create_dir_all(&action_dir).unwrap();

    // SAFETY: env writes are serialized by TEST_ENV_LOCK above.
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
        std::env::set_var("OPENHUMAN_ACTION_DIR", &action_dir);
    }

    let out = handle_list(Map::new())
        .await
        .expect("list handler degrades cleanly for a non-git action_dir");
    // `into_cli_compatible_json` returns the bare value when there are no
    // logs; the list payload is therefore at the top level.
    let payload = out.get("result").unwrap_or(&out);
    assert_eq!(payload["worktrees"], json!([]));
    assert_eq!(payload["overlaps"], json!([]));

    unsafe {
        std::env::remove_var("OPENHUMAN_ACTION_DIR");
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[test]
fn status_and_diff_views_round_trip_a_worktree() {
    if !git_available() {
        return;
    }
    let (_tmp, root) = init_repo();
    let st = worktree::create(&root, "run-2", worktree::BaseRef::Head).expect("create");

    // `WorktreeStatus` serializes `rename_all = "camelCase"` → `isDirty`.
    let status = status_view(&root, &st.path, "cid").expect("status ok");
    assert_eq!(status["isDirty"], json!(false), "fresh worktree is clean");

    // A clean worktree diffs to an empty summary.
    let diff = diff_view(&root, &st.path, "cid").expect("diff ok");
    assert_eq!(diff["summary"], json!(""));
}

#[test]
fn remove_view_clears_a_clean_worktree() {
    if !git_available() {
        return;
    }
    let (_tmp, root) = init_repo();
    let st = worktree::create(&root, "run-3", worktree::BaseRef::Head).expect("create");
    assert!(st.path.exists());

    let removed = remove_view(&root, &st.path, false, "cid").expect("remove ok");
    assert_eq!(removed["removed"], json!(true));
    assert!(!st.path.exists(), "worktree dir gone after remove");
}

#[test]
fn status_view_errors_on_unknown_path() {
    if !git_available() {
        return;
    }
    let (_tmp, root) = init_repo();
    let bogus = root.join(".claude/worktrees/never-created");
    assert!(status_view(&root, &bogus, "cid").is_err());
}

#[test]
fn correlation_id_is_eight_hex_chars() {
    let id = new_correlation_id();
    assert_eq!(id.len(), 8);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn registered_controllers_match_schemas() {
    let schemas = all_controller_schemas();
    let registered = all_registered_controllers();
    assert_eq!(schemas.len(), registered.len());
    assert_eq!(schemas.len(), 4);
    assert!(schemas.iter().all(|s| s.namespace == "worktree"));
    assert_eq!(schema_for("worktree_list").function, "list");
    assert_eq!(schema_for("worktree_status").function, "status");
    assert_eq!(schema_for("worktree_diff").function, "diff");
    assert_eq!(schema_for("worktree_remove").function, "remove");
}

#[test]
fn managed_worktree_filter() {
    assert!(is_managed_worktree(Path::new(
        "/home/u/proj/.claude/worktrees/worker-abc"
    )));
    assert!(!is_managed_worktree(Path::new("/home/u/proj")));
    assert!(!is_managed_worktree(Path::new("/home/u/proj/.claude")));
}

#[test]
fn require_managed_worktree_path_enforces_absolute_and_managed() {
    let mut p = Map::new();
    // Missing / blank path is rejected.
    assert!(require_managed_worktree_path(&p).is_err());
    p.insert("path".into(), Value::String("  ".into()));
    assert!(require_managed_worktree_path(&p).is_err());

    // A relative path is rejected even when it looks managed.
    p.insert(
        "path".into(),
        Value::String(".claude/worktrees/worker-x".into()),
    );
    assert!(require_managed_worktree_path(&p).is_err());

    // An absolute but unmanaged path (e.g. the main checkout) is rejected —
    // worktree_remove must never target it.
    p.insert("path".into(), Value::String("/home/u/proj".into()));
    assert!(require_managed_worktree_path(&p).is_err());

    // An absolute, managed worker checkout is accepted.
    let ok = "/home/u/proj/.claude/worktrees/worker-x";
    p.insert("path".into(), Value::String(ok.into()));
    assert_eq!(
        require_managed_worktree_path(&p).unwrap(),
        PathBuf::from(ok)
    );
}

#[test]
fn overlaps_detected_across_branches() {
    let worktrees = vec![
        WorktreeStatus {
            path: PathBuf::from("/r/.claude/worktrees/a"),
            branch: Some("worker/a".into()),
            is_dirty: true,
            changed_files: vec![PathBuf::from("src/lib.rs"), PathBuf::from("a.rs")],
        },
        WorktreeStatus {
            path: PathBuf::from("/r/.claude/worktrees/b"),
            branch: Some("worker/b".into()),
            is_dirty: true,
            changed_files: vec![PathBuf::from("src/lib.rs")],
        },
    ];
    let overlaps = overlaps_json(&worktrees);
    assert_eq!(overlaps.len(), 1);
    assert_eq!(overlaps[0]["file"], json!("src/lib.rs"));
    assert_eq!(overlaps[0]["branches"], json!(["worker/a", "worker/b"]));
}
