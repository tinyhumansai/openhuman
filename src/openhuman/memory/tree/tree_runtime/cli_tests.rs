use std::ffi::OsString;
use std::path::PathBuf;

use tempfile::TempDir;

use crate::openhuman::config::TEST_ENV_LOCK;

use super::*;

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

struct WorkspaceEnvGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous: Option<OsString>,
}

impl WorkspaceEnvGuard {
    fn set(path: &std::path::Path) -> Self {
        let lock = lock_env();
        let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
        std::env::set_var("OPENHUMAN_WORKSPACE", path);
        Self {
            _lock: lock,
            previous,
        }
    }
}

impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var("OPENHUMAN_WORKSPACE", previous);
        } else {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }

    fn remove(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        std::env::remove_var(key);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

#[test]
fn is_help_matches_supported_aliases() {
    assert!(is_help("-h"));
    assert!(is_help("--help"));
    assert!(is_help("help"));
    assert!(!is_help("run"));
}

#[test]
fn parse_opts_collects_known_flags_and_rest_args() {
    let args = vec![
        "--content".to_string(),
        "hello".to_string(),
        "--file".to_string(),
        "notes.md".to_string(),
        "--node-id".to_string(),
        "2024/03/15".to_string(),
        "--verbose".to_string(),
        "namespace".to_string(),
    ];
    let (opts, rest) = parse_opts(&args).unwrap();
    assert!(opts.verbose);
    assert_eq!(opts.content.as_deref(), Some("hello"));
    assert_eq!(opts.file.as_deref(), Some("notes.md"));
    assert_eq!(opts.node_id.as_deref(), Some("2024/03/15"));
    assert_eq!(rest, vec!["namespace".to_string()]);
}

#[test]
fn parse_opts_errors_when_flag_value_is_missing() {
    let err = match parse_opts(&["--content".to_string()]) {
        Ok(_) => panic!("missing --content value should fail"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("missing value for --content"));

    let err = match parse_opts(&["--file".to_string()]) {
        Ok(_) => panic!("missing --file value should fail"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("missing value for --file"));

    let err = match parse_opts(&["--node-id".to_string()]) {
        Ok(_) => panic!("missing --node-id value should fail"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("missing value for --node-id"));
}

#[test]
fn top_level_command_help_and_unknown_subcommand_behave() {
    assert!(run_tree_summarizer_command(&[]).is_ok());
    assert!(run_tree_summarizer_command(&["--help".to_string()]).is_ok());

    let err = run_tree_summarizer_command(&["bogus".to_string()])
        .expect_err("unknown subcommand should fail");
    assert!(err
        .to_string()
        .contains("unknown tree-summarizer subcommand"));
}

#[test]
fn subcommand_argument_validation_errors_without_running_runtime() {
    let err =
        run_ingest(&["ns".to_string()]).expect_err("ingest without content or file should fail");
    assert!(err
        .to_string()
        .contains("either --content or --file is required"));

    let err = run_ingest(&["ns".to_string(), "--content".to_string(), "   ".to_string()])
        .expect_err("blank content should fail");
    assert!(err.to_string().contains("content is empty"));
}

#[test]
fn help_paths_for_subcommands_return_ok() {
    assert!(run_ingest(&["--help".to_string()]).is_ok());
    assert!(run_summarize(&["--help".to_string()]).is_ok());
    assert!(run_query(&["--help".to_string()]).is_ok());
    assert!(run_status(&["--help".to_string()]).is_ok());
    assert!(run_rebuild(&["--help".to_string()]).is_ok());
}

#[test]
fn ingest_prefers_file_input_and_surfaces_read_errors() {
    let tmp = TempDir::new().unwrap();
    let _workspace = WorkspaceEnvGuard::set(tmp.path());
    let missing = tmp.path().join("missing.txt");

    let args = vec![
        "ns".to_string(),
        "--content".to_string(),
        "fallback text".to_string(),
        "--file".to_string(),
        missing.display().to_string(),
    ];
    let err = run_ingest(&args).expect_err("missing file should win over inline content");
    assert!(err.to_string().contains("failed to read"));
    assert!(err.to_string().contains("missing.txt"));
}

#[test]
fn run_summarize_errors_cleanly_without_provider() {
    // With no local AI and no cloud opt-in (default), `run` returns a clean
    // actionable error rather than panicking or giving an opaque failure.
    // Users must enable local AI (Ollama) or set cloud_summarization_opt_in
    // in config (or via OPENHUMAN_MEMORY_TREE_CLOUD_SUMMARIZATION=true).
    let tmp = TempDir::new().unwrap();
    let _workspace = WorkspaceEnvGuard::set(tmp.path());

    let err = run_summarize(&["fresh-ns".to_string()])
        .expect_err("should error without any summarization provider");
    let msg = err.to_string();
    assert!(
        msg.contains("no summarization provider"),
        "error should name the missing provider: {msg}"
    );
}

#[test]
fn load_config_uses_isolated_workspace_and_env_overrides() {
    let tmp = TempDir::new().unwrap();
    let _workspace = WorkspaceEnvGuard::set(tmp.path());
    let _model = EnvVarGuard::set("OPENHUMAN_MODEL", "custom-model");
    let _language = EnvVarGuard::set("OPENHUMAN_OUTPUT_LANGUAGE", "fr-CA");

    let runtime = build_runtime().expect("runtime");
    let config = runtime.block_on(load_config()).expect("config");

    let expected_config_path: PathBuf = tmp.path().join("config.toml");
    assert_eq!(config.config_path, expected_config_path);
    assert_eq!(config.workspace_dir, tmp.path().join("workspace"));
    assert_eq!(config.default_model.as_deref(), Some("custom-model"));
    assert_eq!(config.output_language.as_deref(), Some("fr-CA"));
}

#[test]
fn init_logging_sets_default_rust_log_only_when_needed() {
    let _lock = lock_env();

    {
        let _rust_log = EnvVarGuard::remove("RUST_LOG");
        init_logging(false);
        assert_eq!(std::env::var("RUST_LOG").ok().as_deref(), Some("warn"));
    }

    {
        let _rust_log = EnvVarGuard::remove("RUST_LOG");
        init_logging(true);
        assert!(std::env::var_os("RUST_LOG").is_none());
    }

    {
        let _rust_log = EnvVarGuard::set("RUST_LOG", "debug");
        init_logging(false);
        assert_eq!(std::env::var("RUST_LOG").ok().as_deref(), Some("debug"));
    }
}
