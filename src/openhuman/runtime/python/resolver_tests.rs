use super::*;

#[test]
fn parses_standard_python_version() {
    assert_eq!(
        parse_python_version("Python 3.12.4"),
        Some(PythonVersion {
            major: 3,
            minor: 12,
            patch: 4
        })
    );
}

#[test]
fn parses_without_python_prefix() {
    assert_eq!(
        parse_python_version("3.12.0"),
        Some(PythonVersion {
            major: 3,
            minor: 12,
            patch: 0
        })
    );
}

#[test]
fn parses_patchless_version_as_zero() {
    assert_eq!(
        parse_python_version("Python 3.12"),
        Some(PythonVersion {
            major: 3,
            minor: 12,
            patch: 0
        })
    );
}

#[test]
fn rejects_invalid_versions() {
    assert_eq!(parse_python_version("Python three.twelve"), None);
    assert_eq!(parse_python_version(""), None);
}

#[cfg(unix)]
#[test]
fn detects_preferred_python_from_custom_path() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("python3.12");
    fs::write(&script, "#!/bin/sh\necho 'Python 3.12.7'\n").expect("write script");
    let mut perms = fs::metadata(&script).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script, perms).expect("chmod");

    let path_var = OsString::from(dir.path().display().to_string());
    let found = detect_system_python_in_path("3.12.0", Some("python3.12"), Some(&path_var))
        .expect("python should resolve");

    assert_eq!(found.version, "3.12.7");
    assert_eq!(found.path, script);
}

#[cfg(unix)]
#[test]
fn rejects_python_below_minimum() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("python3");
    fs::write(&script, "#!/bin/sh\necho 'Python 3.11.9'\n").expect("write script");
    let mut perms = fs::metadata(&script).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script, perms).expect("chmod");

    let path_var = OsString::from(dir.path().display().to_string());
    let found = detect_system_python_in_path("3.12.0", None, Some(&path_var));
    assert!(found.is_none(), "3.11 must be rejected");
}

#[test]
fn candidate_commands_include_minimum_specific_binary() {
    let candidates = candidate_commands(
        None,
        PythonVersion {
            major: 3,
            minor: 13,
            patch: 0,
        },
    );
    assert_eq!(candidates[0], "python3.13");
    assert!(candidates.iter().any(|candidate| candidate == "python3"));
}
