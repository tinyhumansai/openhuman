use super::*;

#[test]
fn empty_request_uses_the_global_folder() {
    assert_eq!(validate_working_dir(None).unwrap(), None);
    assert_eq!(validate_working_dir(Some("   ")).unwrap(), None);
}

#[cfg(feature = "security-module")]
#[test]
fn existing_absolute_directory_is_canonicalized() {
    let _port = crate::modules::security::gap::install_port(std::sync::Arc::new(
        crate::modules::security::gap::stub::StubPort::with_credential_floor(),
    ));
    let temp = tempfile::tempdir().unwrap();
    let nested = temp.path().join("project");
    std::fs::create_dir(&nested).unwrap();
    let raw = format!("{}/./", nested.display());
    let validated = validate_working_dir(Some(&raw)).unwrap().unwrap();
    assert_eq!(PathBuf::from(validated), nested.canonicalize().unwrap());
}

#[cfg(feature = "security-module")]
#[test]
fn relative_missing_file_and_forbidden_paths_are_rejected() {
    let _port = crate::modules::security::gap::install_port(std::sync::Arc::new(
        crate::modules::security::gap::stub::StubPort::with_credential_floor(),
    ));
    assert!(validate_working_dir(Some("relative/dir")).is_err());

    let temp = tempfile::tempdir().unwrap();
    assert!(
        validate_working_dir(Some(&temp.path().join("missing").display().to_string())).is_err()
    );

    let file = temp.path().join("file.txt");
    std::fs::write(&file, "x").unwrap();
    assert!(validate_working_dir(Some(&file.display().to_string())).is_err());

    let ssh = temp.path().join(".ssh");
    std::fs::create_dir(&ssh).unwrap();
    assert!(validate_working_dir(Some(&ssh.display().to_string())).is_err());
}

#[cfg(feature = "security-module")]
#[test]
fn a_working_folder_is_refused_when_the_floor_cannot_be_asked() {
    let _port = crate::modules::security::gap::install_port(std::sync::Arc::new(
        crate::modules::security::gap::stub::StubPort::unavailable(),
    ));
    let temp = tempfile::tempdir().unwrap();
    assert!(validate_working_dir(Some(&temp.path().display().to_string())).is_err());
}
