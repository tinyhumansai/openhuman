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

fn roots(saas: bool) -> Vec<String> {
    let tmp = tempfile::tempdir().unwrap();
    let policy = crate::security::SecurityPolicy::from_config_with(
        saas,
        &crate::config::AutonomyConfig::default(),
        &tmp.path().join("ws"),
        &tmp.path().join("act"),
    );
    policy
        .trusted_roots
        .iter()
        .map(|r| r.path.clone())
        .collect()
}

#[test]
fn saas_policy_skips_the_shared_projects_and_scratch_grants() {
    // `OPENHUMAN_PROJECTS_DIR` is replaced by other tests under this lock.
    let _env = crate::config::TEST_ENV_LOCK.blocking_lock();
    let projects = crate::config::default_projects_dir()
        .to_string_lossy()
        .to_string();
    let scratch = openhuman_scratch_dir().to_string_lossy().to_string();
    let saas = roots(true);
    assert!(
        !saas.contains(&projects),
        "projects home must not be granted in SaaS"
    );
    assert!(
        !saas.contains(&scratch),
        "/tmp/openhuman must not be granted in SaaS"
    );

    let scratch_available = ensure_openhuman_scratch_dir().is_some();
    let single = roots(false);
    assert!(single.contains(&projects));
    // Granted exactly when the dir can be created safely (not a symlink,
    // hardenable permissions): an unsafe path must not be granted.
    assert_eq!(single.contains(&scratch), scratch_available);
}
