use super::*;
use crate::storage::layout::test_fixtures::{v1_profile, write};

#[test]
fn user_files_move_and_imported_state_is_deleted() {
    assert_eq!(classify_top("skills").map(|c| c.0), Some(Action::Move));
    assert_eq!(classify_top("attachments").map(|c| c.0), Some(Action::Move));
    assert_eq!(classify_top("artifacts").map(|c| c.0), Some(Action::Move));
    assert_eq!(classify_top("STYLE.md").map(|c| c.0), Some(Action::Move));
    assert_eq!(classify_top("storage").map(|c| c.0), Some(Action::Delete));
    assert_eq!(
        classify_top("session_raw").map(|c| c.0),
        Some(Action::Delete)
    );
    assert_eq!(classify_top("who-knows"), None);
}

#[test]
fn memory_is_split_one_level_down() {
    assert_eq!(classify_memory_child("jobs.json", false).0, Action::Delete);
    assert_eq!(
        classify_memory_child("conversations", true).0,
        Action::Delete
    );
    assert_eq!(classify_memory_child("legacy_claims", true).0, Action::Move);
    // a directory that merely looks like a state file is not one
    assert_eq!(classify_memory_child("x.json", true).0, Action::Move);
}

#[test]
fn scan_classifies_a_v1_workspace_and_totals_it() {
    let fx = v1_profile();
    let plan = scan(&fx.workspace()).unwrap();
    let by = |p: &str| {
        plan.entries
            .iter()
            .find(|e| e.path == p)
            .unwrap_or_else(|| panic!("no {p}"))
    };
    assert_eq!(by("skills").action, Action::Move);
    assert_eq!(by("state").action, Action::Move);
    assert_eq!(by("memory/legacy_claims").action, Action::Move);
    assert_eq!(by("memory/jobs.json").action, Action::Delete);
    assert_eq!(by("memory/conversations").action, Action::Delete);
    assert_eq!(by("storage").action, Action::Delete);
    assert_eq!(by("mystery").action, Action::Unknown);
    assert_eq!(by("notes.tmp").action, Action::Unknown);
    assert!(
        plan.entries.iter().all(|e| e.path != "memory"),
        "memory is classified by child"
    );
    let unknown = plan.totals(Action::Unknown);
    assert_eq!((unknown.entries, unknown.files), (2, 2));
    assert_eq!(unknown.bytes, "???".len() as u64 + "scratch".len() as u64);
    let archive = plan.archive_totals();
    assert_eq!(
        archive.files,
        plan.totals(Action::Move).files + plan.totals(Action::Delete).files
    );
    // sorted by path
    let paths: Vec<_> = plan.entries.iter().map(|e| e.path.clone()).collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted);
}

#[test]
fn a_missing_workspace_is_an_empty_plan() {
    let dir = tempfile::TempDir::new().unwrap();
    assert!(scan(&dir.path().join("workspace"))
        .unwrap()
        .entries
        .is_empty());
}

#[test]
fn measure_counts_files_and_bytes_without_following_symlinks() {
    let dir = tempfile::TempDir::new().unwrap();
    write(&dir.path().join("a/one.txt"), "12345");
    write(&dir.path().join("a/b/two.txt"), "123");
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.path().join("a"), dir.path().join("a/loop")).unwrap();
    let (files, bytes) = measure(&dir.path().join("a"));
    #[cfg(unix)]
    assert_eq!(files, 3, "two files and the symlink itself");
    #[cfg(not(unix))]
    assert_eq!(files, 2);
    assert!(bytes >= 8);
}

#[test]
fn join_splits_the_relative_path() {
    let base = std::path::Path::new("/x/ws");
    assert_eq!(
        join(base, "memory/jobs.json"),
        base.join("memory").join("jobs.json")
    );
}
