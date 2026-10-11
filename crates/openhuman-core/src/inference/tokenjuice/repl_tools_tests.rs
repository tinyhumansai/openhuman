use super::*;
use serde_json::json;
use std::sync::Mutex;
use tinyjuice_bus::repl::ReplOutput;

#[derive(Default)]
struct RecordingSource(Mutex<Vec<QueryRequest>>);

#[async_trait]
impl QuerySource for RecordingSource {
    async fn query(&self, request: QueryRequest) -> Result<QueryResponse, String> {
        self.0.lock().unwrap().push(request);
        Ok(Ok(ReplOutput::Text {
            text: "needle in the middle".into(),
        }))
    }
}

struct FailingSource;
#[async_trait]
impl QuerySource for FailingSource {
    async fn query(&self, _: QueryRequest) -> Result<QueryResponse, String> {
        Err("module unavailable".into())
    }
}

fn tool(tools: &[Box<dyn Tool>], name: &str) -> usize {
    tools.iter().position(|t| t.name() == name).unwrap()
}
fn result_text(result: &ToolResult) -> String {
    result.output()
}
fn log_body() -> String {
    "# Report\nrow\nERROR: needle in the middle\n".repeat(100)
}

#[test]
fn declarations_preserve_names_schemas_and_read_only_caps() {
    let tools = repl_tools();
    assert_eq!(
        tools.iter().map(|t| t.name()).collect::<Vec<_>>(),
        REPL_TOOL_NAMES
    );
    for (tool, declaration) in tools
        .iter()
        .zip(tinyjuice_bus::tools::repl_tool_declarations())
    {
        assert_eq!(tool.parameters_schema(), declaration.parameters);
        assert_eq!(tool.description(), declaration.description);
        assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
        assert!(tool.is_concurrency_safe(&json!({})));
        assert!(!tool.external_effect());
        assert_eq!(tool.max_result_size_chars(), Some(16_000));
    }
}

#[tokio::test]
async fn a_cached_handle_is_queried_without_retrieving_the_original() {
    let source = Arc::new(RecordingSource::default());
    let tools = repl_tools_with(source.clone(), ReplLimits::default());
    let result = tools[0]
        .execute(json!({"handle":"⟦tj:abc123⟧", "query":"needle"}))
        .await
        .unwrap();
    assert!(!result.is_error);
    let requests = source.0.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].target,
        QueryTarget::Handle {
            token: "abc123".into()
        }
    );
    assert!(matches!(&requests[0].op, ReplOp::Find {query, ..} if query == "needle"));
}

#[tokio::test]
async fn authorized_artifacts_send_content_without_a_path() {
    let (_tmp, dir, file) = artifacts_fixture();
    let source = Arc::new(RecordingSource::default());
    let tools = repl_tools_with_artifacts(source.clone(), ReplLimits::default(), Some(dir));
    let result = tools[0]
        .execute(json!({"handle":file.to_string_lossy(), "query":"needle"}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert_eq!(
        source.0.lock().unwrap()[0].target,
        QueryTarget::Content {
            content: log_body()
        }
    );
}

#[tokio::test]
async fn rejected_paths_and_invalid_operations_never_reach_the_module() {
    let source = Arc::new(RecordingSource::default());
    let tools = repl_tools_with(source.clone(), ReplLimits::default());
    for args in [
        json!({"handle":"../secret", "query":"x"}),
        json!({"handle":"abc123"}),
        json!({}),
    ] {
        assert!(tools[0].execute(args).await.unwrap().is_error);
    }
    assert!(source.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_unavailable_module_is_a_tool_error() {
    let tools = repl_tools_with(Arc::new(FailingSource), ReplLimits::default());
    let result = tools[0]
        .execute(json!({"handle":"abc123", "query":"x"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("module unavailable"));
}

#[test]
fn handle_normalization_only_accepts_alphanumeric_tokens() {
    assert_eq!(normalize_handle(" abc123 "), Some("abc123"));
    assert_eq!(normalize_handle("⟦tj:abc123⟧"), Some("abc123"));
    assert_eq!(normalize_handle("../x"), None);
    assert_eq!(normalize_handle(&"f".repeat(MAX_HANDLE_LEN + 1)), None);
}

/// A workspace whose tool-results dir holds one persisted shell output, the
/// way `ToolResultArtifactStore::detached` writes it.
fn artifacts_fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = crate::security::policy::tool_result_artifacts_dir(tmp.path());
    let file = dir.join("sess1").join("shell").join("call_abc123.txt");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, log_body()).unwrap();
    (tmp, dir, file)
}

fn tools_with_artifacts(dir: std::path::PathBuf) -> Vec<Box<dyn Tool>> {
    repl_tools_with_artifacts(
        Arc::new(RecordingSource::default()),
        ReplLimits::default(),
        Some(dir),
    )
}

#[tokio::test]
async fn an_artifact_path_in_the_handle_slot_is_read_from_the_tool_results_dir() {
    // Production: the model copies `artifact_path` from the oversized-output
    // envelope into `handle` and got "invalid handle".
    let (_tmp, dir, file) = artifacts_fixture();
    let tools = tools_with_artifacts(dir);

    let found = tools[tool(&tools, "juice_find")]
        .execute(json!({ "handle": file.to_string_lossy(), "query": "needle" }))
        .await
        .unwrap();
    assert!(!found.is_error, "{}", result_text(&found));
    assert!(result_text(&found).contains("needle in the middle"));

    let summary = tools[tool(&tools, "juice_summarize")]
        .execute(json!({ "handle": file.to_string_lossy() }))
        .await
        .unwrap();
    assert!(!summary.is_error, "{}", result_text(&summary));

    // The legacy relative pointer form resolves against the same dir.
    let relative = format!(
        "artifacts/tool-results/{}",
        file.strip_prefix(dir_of(&file, 3))
            .unwrap()
            .to_string_lossy()
    );
    let by_relative = tools[tool(&tools, "juice_find")]
        .execute(json!({ "handle": relative, "query": "needle" }))
        .await
        .unwrap();
    assert!(!by_relative.is_error, "{}", result_text(&by_relative));
}

/// `path` with its last `levels` components removed.
fn dir_of(path: &std::path::Path, levels: usize) -> std::path::PathBuf {
    let mut p = path.to_path_buf();
    for _ in 0..levels {
        p.pop();
    }
    p
}

#[tokio::test]
async fn an_artifact_path_outside_the_tool_results_dir_is_refused() {
    let (tmp, dir, _file) = artifacts_fixture();
    let secret = tmp.path().join("secret.txt");
    std::fs::write(&secret, "needle secret").unwrap();
    let tools = tools_with_artifacts(dir.clone());
    let find = &tools[tool(&tools, "juice_find")];

    let traversal = dir
        .join("sess1")
        .join("..")
        .join("..")
        .join("..")
        .join("secret.txt");
    for bad in [
        secret.to_string_lossy().into_owned(),
        traversal.to_string_lossy().into_owned(),
        "artifacts/tool-results/../../secret.txt".to_string(),
        "/etc/passwd".to_string(),
    ] {
        let res = find
            .execute(json!({ "handle": bad, "query": "needle" }))
            .await
            .unwrap();
        assert!(res.is_error, "{bad} must be refused");
        assert!(
            !result_text(&res).contains("needle secret"),
            "{bad} leaked content"
        );
        assert!(
            result_text(&res).contains("file_read"),
            "{}",
            result_text(&res)
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_inside_the_tool_results_dir_cannot_escape_it() {
    let (tmp, dir, _file) = artifacts_fixture();
    let secret = tmp.path().join("secret.txt");
    std::fs::write(&secret, "needle secret").unwrap();
    let link = dir.join("sess1").join("shell").join("call_link.txt");
    std::os::unix::fs::symlink(&secret, &link).unwrap();
    let tools = tools_with_artifacts(dir);
    let res = tools[tool(&tools, "juice_find")]
        .execute(json!({ "handle": link.to_string_lossy(), "query": "needle" }))
        .await
        .unwrap();
    assert!(res.is_error);
    assert!(!result_text(&res).contains("needle secret"));
}

#[test]
fn an_oversized_artifact_is_refused_before_it_is_read() {
    let (_tmp, dir, file) = artifacts_fixture();
    let err = read_tool_result_artifact(&dir, &file.to_string_lossy(), 16).unwrap_err();
    assert!(err.contains("file_read"), "{err}");
    let ok = read_tool_result_artifact(&dir, &file.to_string_lossy(), 10 * 1024 * 1024).unwrap();
    assert!(ok.contains("needle in the middle"));
}

#[tokio::test]
async fn a_call_id_or_other_non_handle_gets_an_actionable_message() {
    let (_tmp, dir, _file) = artifacts_fixture();
    for tools in [
        tools_with_artifacts(dir),
        repl_tools_with(Arc::new(RecordingSource::default()), ReplLimits::default()),
    ] {
        let find = &tools[tool(&tools, "juice_find")];
        for bad in ["call_abc123", "toolu_01XyZ", "a b"] {
            let res = find
                .execute(json!({ "handle": bad, "query": "x" }))
                .await
                .unwrap();
            assert!(res.is_error, "{bad} must be rejected");
            let msg = result_text(&res);
            assert!(msg.contains("32"), "names the handle shape: {msg}");
            assert!(msg.contains("file_read"), "points at file_read: {msg}");
        }
    }
}

#[tokio::test]
async fn an_artifact_path_without_a_configured_dir_points_at_file_read() {
    let (_tmp, _dir, file) = artifacts_fixture();
    let tools = repl_tools_with(Arc::new(RecordingSource::default()), ReplLimits::default());
    let res = tools[tool(&tools, "juice_find")]
        .execute(json!({ "handle": file.to_string_lossy(), "query": "needle" }))
        .await
        .unwrap();
    assert!(res.is_error);
    assert!(result_text(&res).contains("file_read"));
}

/// A workspace reached through a symlinked component (macOS `/var` ->
/// `/private/var`) still accepts the canonical absolute `artifact_path` the
/// store hands out, while the canonical containment check keeps refusing
/// anything outside the tool-results dir.
#[cfg(unix)]
#[test]
fn a_canonical_artifact_path_is_accepted_when_the_workspace_is_a_symlink() {
    let tmp = tempfile::TempDir::new().unwrap();
    let real = tmp.path().join("real");
    std::fs::create_dir_all(&real).unwrap();
    let alias = tmp.path().join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();

    let dir = crate::security::policy::tool_result_artifacts_dir(&alias);
    let file = dir.join("sess1").join("shell").join("call_abc123.txt");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, log_body()).unwrap();
    let canonical_file = std::fs::canonicalize(&file).unwrap();
    assert!(
        !canonical_file.starts_with(&dir),
        "fixture must differ lexically"
    );

    let ok = read_tool_result_artifact(&dir, &canonical_file.to_string_lossy(), 1 << 20)
        .expect("canonical artifact path must be readable");
    assert!(ok.contains("needle in the middle"));

    let secret = real.join("secret.txt");
    std::fs::write(&secret, "needle secret").unwrap();
    let canonical_secret = std::fs::canonicalize(&secret).unwrap();
    let err =
        read_tool_result_artifact(&dir, &canonical_secret.to_string_lossy(), 1 << 20).unwrap_err();
    assert!(!err.contains("needle secret"), "{err}");
}
