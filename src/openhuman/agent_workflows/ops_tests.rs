//! Tests for workflow create/read/uninstall against a temp root.

use super::*;
use tempfile::TempDir;

#[test]
fn create_then_read_round_trip() {
    let root = TempDir::new().unwrap();
    let created = create_workflow_in_root(
        root.path(),
        "Bug Triage",
        "handle bugs",
        "a bug is reported",
    )
    .unwrap();
    assert_eq!(created.dir_name, "bug-triage");
    assert_eq!(created.name, "Bug Triage");
    assert!(created.phases.contains_key("on_pick_up_task"));

    let read = read_workflow_in_root(root.path(), "bug-triage").unwrap();
    assert_eq!(read.name, "Bug Triage");
    assert_eq!(read.when_to_use, "a bug is reported");
}

#[test]
fn create_rejects_duplicate() {
    let root = TempDir::new().unwrap();
    create_workflow_in_root(root.path(), "dup", "d", "w").unwrap();
    let err = create_workflow_in_root(root.path(), "dup", "d", "w").unwrap_err();
    assert!(err.contains("already exists"), "err: {err}");
}

#[test]
fn create_rejects_empty_slug() {
    let root = TempDir::new().unwrap();
    let err = create_workflow_in_root(root.path(), "!!!", "d", "w").unwrap_err();
    assert!(err.contains("alphanumeric"), "err: {err}");
}

#[test]
fn read_missing_workflow_errors() {
    let root = TempDir::new().unwrap();
    let err = read_workflow_in_root(root.path(), "ghost").unwrap_err();
    assert!(err.contains("not found"), "err: {err}");
}

#[test]
fn uninstall_removes_workflow() {
    let root = TempDir::new().unwrap();
    create_workflow_in_root(root.path(), "temp", "d", "w").unwrap();
    assert!(uninstall_workflow_in_root(root.path(), "temp").unwrap());
    assert!(read_workflow_in_root(root.path(), "temp").is_err());
}

#[test]
fn uninstall_rejects_traversal_ids() {
    let root = TempDir::new().unwrap();
    for bad in ["..", "../escape", "a/b", "a\\b", ""] {
        let err = uninstall_workflow_in_root(root.path(), bad).unwrap_err();
        assert!(
            err.contains("invalid workflow id") || err.contains("must not be empty"),
            "id={bad:?} err={err}"
        );
    }
}

#[test]
fn slugify_normalizes_names() {
    assert_eq!(slugify_for_test("Bug Triage!"), "bug-triage");
    assert_eq!(slugify_for_test("  multiple   spaces  "), "multiple-spaces");
    assert_eq!(slugify_for_test("ALL_CAPS"), "all-caps");
}
