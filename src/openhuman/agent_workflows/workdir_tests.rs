//! Tests for working-directory context rendering.

use super::*;
use std::process::Command;
use tempfile::TempDir;

fn git(dir: &std::path::Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    assert!(ok, "git {args:?} failed");
}

#[test]
fn empty_providers_render_empty() {
    let dir = TempDir::new().unwrap();
    assert_eq!(working_dir_context(dir.path(), &[]), "");
}

#[test]
fn unknown_provider_is_flagged() {
    let dir = TempDir::new().unwrap();
    let out = working_dir_context(dir.path(), &["nope".to_string()]);
    assert!(out.contains("unknown context provider: nope"));
}

#[test]
fn git_provider_reports_branch_and_clean_state() {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "t@t.test"]);
    git(dir.path(), &["config", "user.name", "Tester"]);
    git(dir.path(), &["commit", "--allow-empty", "-q", "-m", "init"]);

    let out = working_dir_context(dir.path(), &["git".to_string()]);
    assert!(out.contains("Working directory context"));
    assert!(out.contains("git branch:"));
    assert!(out.contains("git dirty: false"));
    assert!(out.contains("recent commits:"));
}

#[test]
fn git_provider_detects_dirty_tree() {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "t@t.test"]);
    git(dir.path(), &["config", "user.name", "Tester"]);
    std::fs::write(dir.path().join("new.txt"), "hi").unwrap();

    let out = working_dir_context(dir.path(), &["git".to_string()]);
    assert!(out.contains("git dirty: true"));
    assert!(out.contains("git status:"));
}
