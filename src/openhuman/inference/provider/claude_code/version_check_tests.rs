use super::*;

#[test]
fn parses_typical_output() {
    assert_eq!(
        parse_version("2.0.4 (Claude Code)\n").as_deref(),
        Some("2.0.4")
    );
}

#[test]
fn rejects_non_numeric_prefix() {
    assert_eq!(parse_version("claude version 2.0.4"), None);
}

#[test]
fn version_compare() {
    assert!(version_lt("1.9.9", "2.0.0"));
    assert!(version_lt("2.0.0", "2.0.1"));
    assert!(!version_lt("2.0.0", "2.0.0"));
    assert!(!version_lt("2.1.0", "2.0.9"));
}

#[test]
fn version_compare_strips_prerelease() {
    assert!(!version_lt("2.0.0-rc.1", "2.0.0"));
}

#[test]
fn first_existing_skips_missing_candidates_and_returns_first_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("missing/claude");
    let real = dir.path().join("claude");
    std::fs::write(&real, b"#!/bin/sh\n").expect("write fake binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755))
            .expect("set permissions");
    }

    assert_eq!(first_existing(std::slice::from_ref(&missing)), None);
    assert_eq!(first_existing(&[missing, real.clone()]), Some(real));
}

#[cfg(unix)]
#[test]
fn first_existing_skips_non_executable_files() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let non_executable = dir.path().join("not-executable");
    let executable = dir.path().join("executable");
    std::fs::write(&non_executable, b"#!/bin/sh\n").expect("write file");
    std::fs::write(&executable, b"#!/bin/sh\n").expect("write file");
    std::fs::set_permissions(&non_executable, std::fs::Permissions::from_mode(0o644))
        .expect("set permissions");
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))
        .expect("set permissions");

    assert_eq!(
        first_existing(&[non_executable, executable.clone()]),
        Some(executable)
    );
}

#[test]
fn well_known_candidates_put_native_install_first() {
    let candidates = well_known_candidates();
    assert!(!candidates.is_empty());
    match dirs::home_dir() {
        Some(home) => assert_eq!(candidates[0], home.join(".local/bin/claude")),
        None => assert_eq!(candidates[0], std::path::Path::new("/usr/local/bin/claude")),
    }
}
