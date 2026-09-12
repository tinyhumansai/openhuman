use super::*;
use tempfile::TempDir;

/// A workspace root that exists on disk, so `canonicalize()` succeeds for the
/// root itself and the tests below isolate the *candidate* path's behaviour.
fn workspace() -> TempDir {
    TempDir::new().expect("workspace tempdir")
}

// ── the regression this file exists for (openhuman#6085) ────────────────
//
// Before the lexical component walk, `resolve_workspace_relative` rejected a
// traversal only when `canonicalize()` happened to resolve it somewhere
// outside the root — which requires the target to EXIST. A traversal at a
// path that does not exist failed open: `canonicalize()` errored,
// `unwrap_or_else` restored the un-normalised candidate, and the
// component-wise `starts_with(root)` passed because `<root>/../../x` really
// does start with `<root>`. The caller then answered with its own `stat`
// error ("No such file or directory") instead of a refusal, so the guard's
// failure was invisible.

#[test]
fn parent_dir_traversal_to_a_missing_path_is_rejected() {
    let ws = workspace();
    // The distinguishing case: nothing exists at the far end, so
    // `canonicalize()` cannot save the guard. This is the exact input from
    // the issue's reproduction.
    let err = resolve_workspace_relative(ws.path(), "../../nonexistent-e2e-probe")
        .expect_err("a `..` traversal must be refused even when the target does not exist");
    assert!(
        err.contains("escapes workspace root"),
        "the refusal must name the escape, not surface a filesystem error: {err}"
    );
}

#[test]
fn parent_dir_traversal_to_an_existing_path_is_rejected() {
    let ws = workspace();
    // The case that already worked, kept so a fix cannot trade one for the
    // other: `/etc/passwd` exists on every POSIX host, so canonicalize()
    // resolves it out of the root.
    let err = resolve_workspace_relative(ws.path(), "../../../../../../etc/passwd")
        .expect_err("a `..` traversal to an existing file must be refused");
    assert!(
        err.contains("escapes workspace root"),
        "unexpected error: {err}"
    );
}

#[test]
fn a_single_parent_dir_component_anywhere_is_rejected() {
    let ws = workspace();
    std::fs::create_dir_all(ws.path().join("a/b")).expect("nested dirs");
    // Lands back inside the root, so the prefix check alone would allow it.
    // The doc contract is "any relative path containing a `..` component",
    // not "any path that ends up outside", so this is refused too.
    let err = resolve_workspace_relative(ws.path(), "a/../b")
        .expect_err("a `..` component must be refused even when it resolves back inside");
    assert!(
        err.contains("escapes workspace root"),
        "unexpected error: {err}"
    );
}

#[test]
fn a_leading_slash_is_trimmed_but_a_traversal_behind_it_is_not() {
    let ws = workspace();
    // `trim_start_matches('/')` strips the leading slash on POSIX, so this
    // arrives as a normal relative path and is allowed — assert the POSIX
    // behaviour we actually have rather than pretending otherwise.
    let ok = resolve_workspace_relative(ws.path(), "/inside.txt")
        .expect("a leading slash is trimmed and the remainder treated as relative");
    assert!(
        ok.starts_with(
            ws.path()
                .canonicalize()
                .unwrap_or_else(|_| ws.path().to_path_buf())
        ),
        "the trimmed path must stay under the workspace root: {}",
        ok.display()
    );
    // A traversal that survives the trim is still refused.
    let err = resolve_workspace_relative(ws.path(), "/../escape.txt")
        .expect_err("a traversal behind a leading slash must still be refused");
    assert!(
        err.contains("escapes workspace root"),
        "unexpected error: {err}"
    );
}

// ── paths that must keep working ────────────────────────────────────────

#[test]
fn an_ordinary_relative_path_still_resolves_under_the_root() {
    let ws = workspace();
    std::fs::create_dir_all(ws.path().join("state")).expect("state dir");
    std::fs::write(ws.path().join("state/costs.jsonl"), b"{}").expect("seed file");

    let resolved = resolve_workspace_relative(ws.path(), "state/costs.jsonl")
        .expect("an ordinary relative path must resolve");
    assert!(
        resolved.ends_with("state/costs.jsonl"),
        "got {}",
        resolved.display()
    );
    assert!(resolved.exists(), "the resolved path must be the real file");
}

#[test]
fn a_path_that_does_not_exist_yet_still_resolves() {
    let ws = workspace();
    // `list_workspace_files` / `read_workspace_file` stat after resolving, so
    // resolution itself must not require existence — otherwise the callers
    // lose their own "not found" error.
    let resolved = resolve_workspace_relative(ws.path(), "not/created/yet.txt")
        .expect("resolution must not depend on the target existing");
    assert!(
        resolved.ends_with("not/created/yet.txt"),
        "got {}",
        resolved.display()
    );
    assert!(!resolved.exists());
}

#[test]
fn a_leading_current_dir_component_is_allowed() {
    let ws = workspace();
    // `./x` is harmless and some callers pass it; the walk lets `CurDir`
    // through deliberately.
    let resolved = resolve_workspace_relative(ws.path(), "./state.json")
        .expect("`./` must not be treated as an escape");
    assert!(
        resolved.ends_with("state.json"),
        "got {}",
        resolved.display()
    );
}

// ── the check the component walk cannot make ────────────────────────────

#[cfg(unix)]
#[test]
fn a_symlink_pointing_out_of_the_workspace_is_still_rejected() {
    let ws = workspace();
    let outside = TempDir::new().expect("outside tempdir");
    let secret = outside.path().join("secret.txt");
    std::fs::write(&secret, b"not yours").expect("seed outside file");
    std::os::unix::fs::symlink(&secret, ws.path().join("link.txt")).expect("symlink");

    // No `..` anywhere, so the lexical walk sees nothing wrong. This is why
    // the canonicalize + starts_with pair is kept underneath it.
    let err = resolve_workspace_relative(ws.path(), "link.txt")
        .expect_err("a symlink escaping the workspace must still be refused");
    assert!(
        err.contains("escapes workspace root"),
        "unexpected error: {err}"
    );
}
