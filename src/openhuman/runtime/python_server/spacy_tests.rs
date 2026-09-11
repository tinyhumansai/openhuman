use super::*;

#[test]
fn cache_root_honours_runtime_python_cache_dir() {
    let mut config = Config::default();
    config.runtime_python.cache_dir = "/tmp/openhuman-python".to_string();
    assert_eq!(
        python_server_cache_root(&config),
        PathBuf::from("/tmp/openhuman-python").join("runtime-python-server")
    );
}

#[test]
fn legacy_configured_cache_is_considered_provisioned() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.runtime_python.cache_dir = temp.path().to_string_lossy().to_string();
    let legacy_venv = temp.path().join("memory-nlp").join("spacy-venv");
    std::fs::create_dir_all(legacy_venv.join(if cfg!(windows) { "Scripts" } else { "bin" }))
        .unwrap();
    std::fs::write(
        spacy_ready_marker_path(&legacy_venv),
        format!("{SPACY_READY_MARKER_VERSION}\n3.11.0"),
    )
    .unwrap();
    std::fs::write(venv_python_path(&legacy_venv), "").unwrap();

    assert!(spacy_provisioned(&config));
}

#[test]
fn stale_marker_venv_is_not_ready_so_it_reprovisions() {
    // Regression for GH-4687: a venv provisioned before the `click` pin
    // carries the ready marker but may be missing `click`. Its old-schema
    // marker must not satisfy readiness, so `ensure_spacy` re-provisions
    // (re-running pip with the `click` pin) instead of short-circuiting and
    // continuing to fail with `ModuleNotFoundError: click`.
    let temp = tempfile::tempdir().unwrap();
    let venv = temp.path().join("spacy-venv");
    std::fs::create_dir_all(venv.join(if cfg!(windows) { "Scripts" } else { "bin" })).unwrap();
    std::fs::write(venv_python_path(&venv), "").unwrap();

    // Pre-#4687 marker wrote only the python version — no schema tag.
    std::fs::write(spacy_ready_marker_path(&venv), "3.11.5").unwrap();
    assert!(
        !spacy_venv_ready(&venv),
        "stale-marker venv must be re-provisioned, not treated as ready"
    );

    // Current-schema marker satisfies readiness.
    std::fs::write(
        spacy_ready_marker_path(&venv),
        format!("{SPACY_READY_MARKER_VERSION}\n3.11.5"),
    )
    .unwrap();
    assert!(
        spacy_venv_ready(&venv),
        "current-schema marker venv is ready"
    );
}

#[test]
fn pip_install_args_include_click_dependency() {
    // Regression for GH-4687: `click` must be an explicit venv dependency so
    // it is never dropped from the packaged runtime on Windows, where its
    // absence breaks `import spacy` with `ModuleNotFoundError: click`.
    let args = spacy_pip_install_args();
    assert!(args.contains(&"click"), "click must be installed: {args:?}");
    assert!(args.contains(&"spacy"), "spacy must be installed: {args:?}");
    assert_eq!(&args[..5], &["-m", "pip", "install", "--upgrade", "pip"]);
}

#[test]
fn spacy_response_parses() {
    let response: SpacyResponse = serde_json::from_str(
        r#"{"entities":[{"text":"Alice","label":"PERSON","start":0,"end":5}],"nouns":["migration"]}"#,
    )
    .unwrap();
    assert_eq!(response.entities[0].label, "PERSON");
    assert_eq!(response.nouns, vec!["migration"]);
}

// Exercises the venv provisioning step path (including the GH-4814
// CREATE_NO_WINDOW hook, a no-op off Windows) with a trivial binary so it
// stays covered without a real python toolchain.
#[cfg(unix)]
#[tokio::test]
async fn run_step_runs_a_trivial_binary() {
    run_step(
        Path::new("/bin/echo"),
        &["ok"],
        Duration::from_secs(30),
        "echo smoke",
    )
    .await
    .expect("echo step succeeds");
}
