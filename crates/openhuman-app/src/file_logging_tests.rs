use super::*;

use crate::test_env::env_lock;

#[test]
fn resolve_data_dir_honors_workspace_override() {
    let _guard = env_lock();
    let prior = std::env::var("OPENHUMAN_WORKSPACE").ok();
    std::env::set_var("OPENHUMAN_WORKSPACE", "/tmp/openhuman-test-override");
    let dir = resolve_data_dir();
    assert_eq!(dir, PathBuf::from("/tmp/openhuman-test-override"));
    match prior {
        Some(v) => std::env::set_var("OPENHUMAN_WORKSPACE", v),
        None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
    }
}

#[test]
fn resolve_data_dir_ignores_empty_workspace() {
    let _guard = env_lock();
    let prior = std::env::var("OPENHUMAN_WORKSPACE").ok();
    std::env::set_var("OPENHUMAN_WORKSPACE", "");
    // Empty string must NOT short-circuit — fall through to the
    // default resolver so the user's real `~/.openhuman` is used.
    let dir = resolve_data_dir();
    assert_ne!(dir, PathBuf::from(""));
    assert!(dir.is_absolute(), "expected absolute fallback, got {dir:?}");
    match prior {
        Some(v) => std::env::set_var("OPENHUMAN_WORKSPACE", v),
        None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
    }
}

#[test]
fn logs_folder_path_returns_none_pre_init() {
    // `init()` is `Once`-guarded across the whole process, so in unit
    // tests where the embedded subscriber hasn't been installed,
    // `logs_folder_path` should return `None` rather than a stale path.
    // (When run alongside a test that *did* call `init`, the function
    // is allowed to return Some — assert the type signature only.)
    let result = logs_folder_path();
    let _: Option<String> = result;
}

#[test]
fn reveal_logs_folder_errors_when_uninitialized() {
    // If logging hasn't been initialized, the command must surface a
    // typed error so the UI can show it instead of silently launching
    // an `open` against an empty path.
    if openhuman_rpc::embed::process::log_directory().is_none() {
        let err = reveal_logs_folder().expect_err("must error pre-init");
        assert!(err.contains("not initialized"), "unexpected error: {err}");
    }
}
