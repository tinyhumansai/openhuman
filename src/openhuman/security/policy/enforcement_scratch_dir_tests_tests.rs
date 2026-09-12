use super::{ensure_openhuman_scratch_dir, openhuman_scratch_dir};

#[test]
fn scratch_dir_is_namespaced_on_every_platform() {
    // Always the dedicated `openhuman` scratch namespace — never a bare
    // temp root, so only this subdir is ever granted as a trusted root.
    let dir = openhuman_scratch_dir();
    assert_eq!(dir.file_name().and_then(|s| s.to_str()), Some("openhuman"));
    #[cfg(not(windows))]
    assert_eq!(dir, std::path::PathBuf::from("/tmp/openhuman"));
}

#[test]
fn ensure_scratch_dir_creates_and_returns_it() {
    // Idempotent: creates the dir, returns its path, and it exists after.
    let ensured = ensure_openhuman_scratch_dir();
    let expected = openhuman_scratch_dir();
    assert_eq!(ensured.as_deref(), Some(expected.as_path()));
    assert!(expected.is_dir());
}
