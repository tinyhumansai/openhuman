use std::path::{Path, PathBuf};

use super::{workspace_handle, HANDLE_HEX_LEN, HANDLE_PREFIX};

#[test]
fn handle_is_stable_for_the_same_directory() {
    let dir = Path::new("/Users/someone/.openhuman/users/abc/workspace");
    assert_eq!(workspace_handle(dir), workspace_handle(dir));
}

#[test]
fn different_directories_get_different_handles() {
    let a = Path::new("/Users/someone/.openhuman/users/abc/workspace");
    let b = Path::new("/Users/someone/.openhuman/users/def/workspace");
    assert_ne!(workspace_handle(a), workspace_handle(b));
}

#[test]
fn trailing_separator_does_not_change_the_handle() {
    let bare = PathBuf::from(format!(
        "{}tmp{}ws",
        std::path::MAIN_SEPARATOR,
        std::path::MAIN_SEPARATOR
    ));
    let trailing = PathBuf::from(format!("{}{}", bare.display(), std::path::MAIN_SEPARATOR));
    assert_eq!(workspace_handle(&bare), workspace_handle(&trailing));
}

#[test]
fn handle_shape_is_prefix_plus_fixed_width_hex() {
    let handle = workspace_handle(Path::new("/Users/someone/.openhuman/users/abc/workspace"));
    let hex = handle
        .strip_prefix(HANDLE_PREFIX)
        .expect("handle carries its prefix");
    assert_eq!(hex.len(), HANDLE_HEX_LEN);
    assert!(
        hex.chars().all(|c| c.is_ascii_hexdigit()),
        "handle body must be hex, got {hex}"
    );
}

/// The whole point of the handle: it is what goes on the wire *instead of*
/// the path, so no component of the path may survive into it. A regression
/// that forwarded the path — or appended it "for debugging" — would leak a
/// home directory into the Event Log's NDJSON export.
#[test]
fn handle_leaks_no_component_of_the_path() {
    let handle = workspace_handle(Path::new(
        "/Users/rumpelstiltskin/.openhuman/users/abc/workspace",
    ));
    for component in [
        "Users",
        "rumpelstiltskin",
        ".openhuman",
        "users",
        "abc",
        "workspace",
    ] {
        assert!(
            !handle.contains(component),
            "handle {handle} leaked path component {component}"
        );
    }
    assert!(
        !handle.contains(std::path::MAIN_SEPARATOR),
        "handle {handle} contains a path separator"
    );
}

/// A root path normalises to the empty string if the trailing-separator trim
/// is applied blindly, which would collapse `/` and any other
/// all-separator spelling onto one handle *and* hash the empty string. The
/// guard in `normalize` keeps the raw value in that case.
#[test]
fn root_path_still_produces_a_handle() {
    let root = PathBuf::from(std::path::MAIN_SEPARATOR.to_string());
    let handle = workspace_handle(&root);
    assert!(handle.starts_with(HANDLE_PREFIX));
    assert_ne!(handle, workspace_handle(Path::new("")));
}
