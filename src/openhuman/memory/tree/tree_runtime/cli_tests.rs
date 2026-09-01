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

/// Bind a tree driver for the workspace these subcommands will resolve to.
///
/// The subcommands go through the contract's runtime-tree doors now (#5560), so
/// each one asks `memory::binding` for a provider. With none installed the
/// binding tries to load the compiled TinyMemory module, which in a test
/// process can *block* rather than fail — so every test that reaches a handler
/// has to put one there first.
///
/// The config is resolved exactly the way [`load_config`] resolves it, rather
/// than being constructed here: `OPENHUMAN_WORKSPACE` is set by
/// [`WorkspaceEnvGuard`] and the env overlay is what turns it into the
/// `workspace_dir` the binding is keyed on. Building a `Config::default()` and
/// pointing it at the tempdir would key the binding on a *different* path than
/// the one the CLI then asks for.
fn bind_workspace_driver() {
    let runtime = build_runtime().expect("runtime");
    let config = runtime.block_on(load_config()).expect("config");
    super::super::test_support::bind_tree_driver(&config);
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
fn ingest_status_and_query_run_against_isolated_workspace() {
    let tmp = TempDir::new().unwrap();
    let _workspace = WorkspaceEnvGuard::set(tmp.path());
    bind_workspace_driver();

    assert!(run_ingest(&[
        "ns".to_string(),
        "--content".to_string(),
        "hello world".to_string()
    ])
    .is_ok());
    assert!(run_status(&["ns".to_string()]).is_ok());
    let err = run_query(&["ns".to_string(), "root".to_string()])
        .expect_err("root query should fail before a summarization run creates nodes");
    assert!(err.to_string().contains("not found"));
}

#[test]
fn ingest_reads_from_file_path() {
    let tmp = TempDir::new().unwrap();
    let _workspace = WorkspaceEnvGuard::set(tmp.path());
    bind_workspace_driver();
    let input = tmp.path().join("input.txt");
    std::fs::write(&input, "from file").unwrap();

    let args = vec![
        "ns".to_string(),
        "--file".to_string(),
        input.display().to_string(),
    ];
    assert!(run_ingest(&args).is_ok());
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
fn query_prefers_explicit_node_flag_over_positional_node() {
    let tmp = TempDir::new().unwrap();
    let _workspace = WorkspaceEnvGuard::set(tmp.path());
    bind_workspace_driver();

    let err = run_query(&[
        "ns".to_string(),
        "2024/03/15".to_string(),
        "--node-id".to_string(),
        "2024/03/16".to_string(),
    ])
    .expect_err("missing node should fail");

    assert!(err
        .to_string()
        .contains("node '2024/03/16' not found in namespace 'ns'"));
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

#[test]
fn run_and_rebuild_no_longer_block_on_local_ai_precondition() {
    // #002 FR-007: the summarizer used to hard-error "requires local_ai to
    // be enabled" when local AI was off, which left Build Summary Trees
    // dead for cloud-only setups. It now builds the configured cloud
    // provider instead. The commands may still surface a downstream error
    // (e.g. a network/auth failure when actually calling the cloud model in
    // a test sandbox), but they must NOT fail on the old local-AI
    // precondition. This test asserts that specific regression is gone.
    let tmp = TempDir::new().unwrap();
    let _workspace = WorkspaceEnvGuard::set(tmp.path());
    bind_workspace_driver();

    // Seed a namespace so the commands go through the runtime path
    // rather than failing argument validation.
    assert!(run_ingest(&[
        "ns".to_string(),
        "--content".to_string(),
        "seed".to_string()
    ])
    .is_ok());

    // Whatever the outcome (Ok, or a downstream provider/network error),
    // it must not be the local-AI precondition error.
    if let Err(e) = run_summarize(&["ns".to_string()]) {
        assert!(
            !e.to_string().contains("requires local_ai to be enabled"),
            "run should no longer block on the local_ai precondition: {e:#}"
        );
    }
    if let Err(e) = run_rebuild(&["ns".to_string()]) {
        assert!(
            !e.to_string().contains("requires local_ai to be enabled"),
            "rebuild should no longer block on the local_ai precondition: {e:#}"
        );
    }
}
