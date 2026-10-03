use super::*;
use std::fs;
use tempfile::tempdir;

#[test]
fn resolve_workspace_path_accepts_existing_relative_file_inside_workspace() {
    let workspace = tempdir().unwrap();
    let docs = workspace.path().join("docs");
    fs::create_dir_all(&docs).unwrap();
    let file = docs.join("note.md");
    fs::write(&file, "hello").unwrap();

    let resolved = resolve_workspace_path(workspace.path(), "docs/note.md").unwrap();

    assert_eq!(resolved, file.canonicalize().unwrap());
}

#[test]
fn resolve_workspace_path_rejects_parent_directory_escape() {
    let workspace = tempdir().unwrap();

    let err = resolve_workspace_path(workspace.path(), "../secret.txt").unwrap_err();

    assert!(err.contains("workspace"), "unexpected error: {err}");
}

#[test]
fn resolve_workspace_path_rejects_absolute_paths() {
    let workspace = tempdir().unwrap();

    let err = resolve_workspace_path(workspace.path(), "/etc/passwd").unwrap_err();

    assert!(err.contains("relative"), "unexpected error: {err}");
}

#[test]
fn resolve_workspace_path_rejects_uri_scheme_prefix() {
    let workspace = tempdir().unwrap();

    let err = resolve_workspace_path(workspace.path(), "file://etc/passwd").unwrap_err();

    assert!(err.contains("relative"), "unexpected error: {err}");
}

#[test]
fn resolve_workspace_path_accepts_colons_after_first_segment() {
    let workspace = tempdir().unwrap();
    let docs = workspace.path().join("docs");
    fs::create_dir_all(&docs).unwrap();
    let file = docs.join("2026:05.md");
    fs::write(&file, "dated").unwrap();

    let resolved = resolve_workspace_path(workspace.path(), "docs/2026:05.md").unwrap();

    assert_eq!(resolved, file.canonicalize().unwrap());
}

#[test]
fn resolve_workspace_path_errors_do_not_expose_workspace_root() {
    let workspace = tempdir().unwrap();

    let err = resolve_workspace_path(workspace.path(), "docs/missing.md").unwrap_err();

    assert!(err.contains("docs/missing.md"), "unexpected error: {err}");
    assert!(
        !err.contains(&workspace.path().display().to_string()),
        "error leaked workspace root: {err}"
    );
}

#[test]
fn preview_workspace_text_from_root_reads_utf8_text() {
    let workspace = tempdir().unwrap();
    fs::write(workspace.path().join("readme.md"), "# Hello").unwrap();

    let preview = preview_workspace_text_from_root(workspace.path(), "readme.md", 1024).unwrap();

    assert_eq!(preview.path, "readme.md");
    assert_eq!(preview.contents, "# Hello");
    assert!(!preview.truncated);
    assert_eq!(preview.size_bytes, 7);
}

#[test]
fn preview_workspace_text_from_root_truncates_large_text() {
    let workspace = tempdir().unwrap();
    fs::write(workspace.path().join("large.md"), "0123456789").unwrap();

    let preview = preview_workspace_text_from_root(workspace.path(), "large.md", 4).unwrap();

    assert_eq!(preview.contents, "0123");
    assert!(preview.truncated);
    assert_eq!(preview.size_bytes, 10);
}

#[test]
fn preview_workspace_text_from_root_errors_do_not_expose_workspace_root() {
    let workspace = tempdir().unwrap();
    fs::create_dir_all(workspace.path().join("docs")).unwrap();

    let err = preview_workspace_text_from_root(workspace.path(), "docs", 1024).unwrap_err();

    assert!(err.contains("docs"), "unexpected error: {err}");
    assert!(
        !err.contains(&workspace.path().display().to_string()),
        "error leaked workspace root: {err}"
    );
}

#[test]
fn resolve_workspace_path_resolves_nested_directory_inside_workspace() {
    let workspace = tempdir().unwrap();
    let docs = workspace.path().join("notes").join("content");
    fs::create_dir_all(&docs).unwrap();

    let resolved = resolve_workspace_path(workspace.path(), "notes/content").unwrap();

    let canonical_root = fs::canonicalize(workspace.path()).unwrap();
    assert!(
        resolved.starts_with(&canonical_root),
        "resolved path escaped workspace root: {} not under {}",
        resolved.display(),
        canonical_root.display()
    );
    assert_eq!(resolved, docs.canonicalize().unwrap());
}

#[test]
fn resolve_workspace_path_rejects_empty_whitespace_input() {
    let workspace = tempdir().unwrap();

    let err = resolve_workspace_path(workspace.path(), "   ").unwrap_err();

    assert!(err.contains("empty"), "unexpected error: {err}");
}

#[cfg(unix)]
#[test]
fn resolve_workspace_path_rejects_symlink_escape() {
    use std::os::unix::fs::symlink;

    let workspace = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let outside_file = outside.path().join("secret.txt");
    fs::write(&outside_file, "secret").unwrap();
    symlink(&outside_file, workspace.path().join("secret-link")).unwrap();

    let err = resolve_workspace_path(workspace.path(), "secret-link").unwrap_err();

    assert!(err.contains("workspace"), "unexpected error: {err}");
}
