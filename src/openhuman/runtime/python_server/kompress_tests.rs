use super::*;

#[test]
fn venv_python_path_is_platform_specific() {
    let p = venv_python_path(Path::new("/tmp/venv"));
    if cfg!(windows) {
        assert!(p.ends_with("Scripts/python.exe") || p.ends_with("Scripts\\python.exe"));
    } else {
        assert_eq!(p, PathBuf::from("/tmp/venv/bin/python"));
    }
}

#[test]
fn provisioned_false_on_clean_config() {
    let mut config = Config::default();
    config.runtime_python.cache_dir = "/nonexistent/tj-test".to_string();
    assert!(!kompress_provisioned(&config));
}

#[test]
fn ready_marker_is_model_specific_and_path_safe() {
    let venv = Path::new("/tmp/openhuman-kompress-test-venv");
    let first = marker_path(venv, "answerdotai/ModernBERT-base");
    let second = marker_path(venv, "other/model");

    assert_ne!(first, second);
    assert_eq!(first.parent(), Some(venv));
    assert!(first
        .file_name()
        .unwrap()
        .to_string_lossy()
        .contains("answerdotai_ModernBERT-base"));
}

// Exercises the provisioning step path (including the GH-4814
// CREATE_NO_WINDOW hook, a no-op off Windows) with a trivial binary so it
// stays covered without a real python toolchain.
#[cfg(unix)]
#[tokio::test]
async fn run_step_runs_a_trivial_binary() {
    let hf = tempfile::tempdir().expect("tempdir");
    run_step(
        Path::new("/bin/echo"),
        &["ok"],
        Duration::from_secs(30),
        hf.path(),
        "echo smoke",
    )
    .await
    .expect("echo step succeeds");
}
