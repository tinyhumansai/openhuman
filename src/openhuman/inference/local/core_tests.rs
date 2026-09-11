use super::*;

#[test]
fn model_artifact_path_includes_models_local_ai_subdirs() {
    let config = Config::default();
    let path = model_artifact_path(&config);
    let path_str = path.to_string_lossy();
    assert!(
        path_str.contains("models"),
        "expected `models` in path: {path_str}"
    );
    assert!(
        path_str.contains("local-ai"),
        "expected `local-ai` subdir in path: {path_str}"
    );
}

#[test]
fn model_artifact_path_ends_with_ollama_suffix() {
    let config = Config::default();
    let path = model_artifact_path(&config);
    assert_eq!(
        path.extension().and_then(|s| s.to_str()),
        Some("ollama"),
        "model artifact must have `.ollama` extension: {}",
        path.display()
    );
}

#[test]
fn model_artifact_path_replaces_colon_in_model_id_with_dash() {
    // Model IDs commonly look like `qwen2:1.5b`; colons are illegal on
    // Windows path components, so we normalise to `-`. This test pins
    // that mapping.
    let config = Config::default();
    let path = model_artifact_path(&config);
    let file = path.file_name().unwrap().to_string_lossy().to_string();
    assert!(!file.contains(':'), "filename must not contain `:`: {file}");
}

#[test]
fn global_returns_same_arc_across_calls() {
    let config = Config::default();
    let a = global(&config);
    let b = global(&config);
    assert!(Arc::ptr_eq(&a, &b), "global() must return a shared Arc");
}
