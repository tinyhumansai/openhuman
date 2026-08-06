//! Tests for the git-worktree isolation manager.
//!
//! Each test stands up a real temporary git repository (`git init`) so the
//! `git worktree` plumbing is exercised end-to-end. Tests are skipped (pass
//! trivially) when `git` is not on PATH, so CI without git doesn't hard-fail.

use super::*;
use std::path::Path;
use std::process::Command;

/// `true` when `git` is invokable on this host.
fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git invocation");
    assert!(
        status.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&status.stderr)
    );
}

/// Initialise a temp git repo with one committed file. Returns the tempdir
/// guard (kept alive by the caller) and the repo root path.
fn init_repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();
    run(&root, &["init", "-b", "main"]);
    run(&root, &["config", "user.email", "test@example.com"]);
    run(&root, &["config", "user.name", "Test User"]);
    std::fs::write(root.join("README.md"), "hello\n").unwrap();
    run(&root, &["add", "README.md"]);
    run(&root, &["commit", "-m", "initial"]);
    (tmp, root)
}

#[test]
fn validate_repo_root_rejects_non_repo() {
    if !git_available() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let err = create(tmp.path(), "run-1", BaseRef::Head).unwrap_err();
    assert!(matches!(err, WorktreeError::NotAGitRepo(_)));
}

#[test]
fn create_then_status_reports_clean_worktree() {
    if !git_available() {
        return;
    }
    let (_tmp, root) = init_repo();
    let st = create(&root, "run-1", BaseRef::Head).expect("create");
    assert!(st.path.exists(), "worktree dir should exist");
    assert_eq!(st.branch.as_deref(), Some("worker/run-1"));
    assert!(!st.is_dirty, "fresh worktree is clean");
    assert!(st.changed_files.is_empty());
    assert!(
        st.path.ends_with(Path::new(".claude/worktrees/run-1")),
        "worktree under .claude/worktrees/<run_id>, got {}",
        st.path.display()
    );
}

#[test]
fn list_includes_created_worktree() {
    if !git_available() {
        return;
    }
    let (_tmp, root) = init_repo();
    create(&root, "run-a", BaseRef::Head).expect("create a");
    create(&root, "run-b", BaseRef::Fresh).expect("create b");
    let all = list(&root).expect("list");
    // main worktree + the two we created
    assert!(all.len() >= 3, "expected >=3 worktrees, got {}", all.len());
    let branches: Vec<_> = all.iter().filter_map(|w| w.branch.clone()).collect();
    assert!(branches.iter().any(|b| b == "worker/run-a"));
    assert!(branches.iter().any(|b| b == "worker/run-b"));
}

#[test]
fn status_detects_dirty_changes() {
    if !git_available() {
        return;
    }
    let (_tmp, root) = init_repo();
    let st = create(&root, "run-dirty", BaseRef::Head).expect("create");
    // Touch a tracked file + add an untracked one.
    std::fs::write(st.path.join("README.md"), "changed\n").unwrap();
    std::fs::write(st.path.join("new.txt"), "fresh\n").unwrap();

    let st2 = status(&root, &st.path).expect("status");
    assert!(st2.is_dirty, "worktree with edits must be dirty");
    let names: Vec<String> = st2
        .changed_files
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    assert!(names.iter().any(|n| n.contains("README.md")));
    assert!(names.iter().any(|n| n.contains("new.txt")));
}

#[test]
fn diff_summary_lists_tracked_and_untracked() {
    if !git_available() {
        return;
    }
    let (_tmp, root) = init_repo();
    let st = create(&root, "run-diff", BaseRef::Head).expect("create");
    std::fs::write(st.path.join("README.md"), "changed body\n").unwrap();
    std::fs::write(st.path.join("brand_new.txt"), "x\n").unwrap();

    let summary = diff_summary(&root, &st.path).expect("diff");
    assert!(
        summary.contains("README.md"),
        "diff should mention tracked change: {summary}"
    );
    assert!(
        summary.contains("brand_new.txt") && summary.contains("untracked"),
        "diff should list untracked file: {summary}"
    );
}

#[test]
fn remove_refuses_dirty_without_force() {
    if !git_available() {
        return;
    }
    let (_tmp, root) = init_repo();
    let st = create(&root, "run-keep", BaseRef::Head).expect("create");
    std::fs::write(st.path.join("README.md"), "dirty\n").unwrap();

    let err = remove(&root, &st.path, false).expect_err("must refuse dirty");
    assert!(matches!(err, WorktreeError::DirtyRefused(_)));
    assert!(st.path.exists(), "dirty worktree must NOT be deleted");
}

#[test]
fn remove_force_deletes_dirty_worktree() {
    if !git_available() {
        return;
    }
    let (_tmp, root) = init_repo();
    let st = create(&root, "run-force", BaseRef::Head).expect("create");
    std::fs::write(st.path.join("README.md"), "dirty\n").unwrap();

    remove(&root, &st.path, true).expect("force remove");
    assert!(!st.path.exists(), "force remove deletes the worktree dir");
}

#[test]
fn remove_clean_worktree_succeeds() {
    if !git_available() {
        return;
    }
    let (_tmp, root) = init_repo();
    let st = create(&root, "run-clean", BaseRef::Head).expect("create");
    remove(&root, &st.path, false).expect("clean remove");
    assert!(!st.path.exists(), "clean worktree removed without force");
}

#[test]
fn base_ref_parse_defaults_to_head() {
    assert_eq!(BaseRef::parse(None), BaseRef::Head);
    assert_eq!(BaseRef::parse(Some("head")), BaseRef::Head);
    assert_eq!(BaseRef::parse(Some("HEAD")), BaseRef::Head);
    assert_eq!(BaseRef::parse(Some("fresh")), BaseRef::Fresh);
    assert_eq!(BaseRef::parse(Some(" Fresh ")), BaseRef::Fresh);
    assert_eq!(BaseRef::parse(Some("garbage")), BaseRef::Head);
}

#[test]
fn sanitize_run_id_strips_unsafe_chars() {
    assert_eq!(sanitize_run_id("sub-1234"), "sub-1234");
    assert_eq!(sanitize_run_id("a/b\\c"), "a-b-c");
    assert_eq!(sanitize_run_id("///"), "worker");
    assert_eq!(sanitize_run_id(""), "worker");
}

#[test]
fn detect_overlaps_flags_shared_files() {
    let per_worker = vec![
        (
            "w1".to_string(),
            vec![PathBuf::from("src/a.rs"), PathBuf::from("src/b.rs")],
        ),
        (
            "w2".to_string(),
            vec![PathBuf::from("src/b.rs"), PathBuf::from("src/c.rs")],
        ),
        ("w3".to_string(), vec![PathBuf::from("src/c.rs")]),
    ];
    let overlaps = detect_overlaps(&per_worker);
    // b.rs touched by w1+w2; c.rs touched by w2+w3; a.rs only w1 (no overlap).
    assert_eq!(overlaps.len(), 2);
    assert_eq!(
        overlaps.get(&PathBuf::from("src/b.rs")).unwrap(),
        &vec!["w1".to_string(), "w2".to_string()]
    );
    assert_eq!(
        overlaps.get(&PathBuf::from("src/c.rs")).unwrap(),
        &vec!["w2".to_string(), "w3".to_string()]
    );
    assert!(!overlaps.contains_key(&PathBuf::from("src/a.rs")));
}

#[test]
fn detect_overlaps_empty_when_disjoint() {
    let per_worker = vec![
        ("w1".to_string(), vec![PathBuf::from("a.rs")]),
        ("w2".to_string(), vec![PathBuf::from("b.rs")]),
    ];
    assert!(detect_overlaps(&per_worker).is_empty());
}

#[test]
fn detect_overlaps_ignores_intra_worker_duplicates() {
    // A single worker listing the same file twice must not self-overlap.
    let per_worker = vec![(
        "w1".to_string(),
        vec![PathBuf::from("a.rs"), PathBuf::from("a.rs")],
    )];
    assert!(detect_overlaps(&per_worker).is_empty());
}
