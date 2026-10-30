use super::*;

#[tokio::test]
async fn malformed_operation_and_path_never_fall_back_to_workspace_wide_scan() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("visible.rs"), "fn visible() {}\n").unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    for malformed in [
        json!([]),
        json!(null),
        json!({"operation": null}),
        json!({"operation": false}),
        json!({"operation": "sacn"}),
        json!({"path": null}),
        json!({"path": 7}),
        json!({"path": ""}),
        json!({"operation": "scan", "path": {"name": "visible.rs"}}),
        json!({"operation": "scan", "query": ["visible"]}),
        json!({"operation": "scan", "query": null}),
    ] {
        assert!(
            tool.execute(malformed.clone()).await.is_err(),
            "malformed request must fail closed, not scan root: {malformed}"
        );
    }
    // A correctly omitted optional operation/path retains documented defaults.
    let valid = run(&tool, json!({})).await;
    assert_eq!(valid["operation"], "scan");
    assert_eq!(valid["files"][0]["path"], "visible.rs");
}

#[tokio::test]
async fn malformed_numeric_and_filter_arguments_do_not_silently_use_defaults() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("visible.rs"), "fn visible() {}\n").unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    for malformed in [
        json!({"max_files": "1"}),
        json!({"max_files": 0}),
        json!({"max_files": 33}),
        json!({"max_entries": -1}),
        json!({"max_edges": 0}),
        json!({"max_results": 6}),
        json!({"start_line": "1"}),
        json!({"start_column": -1}),
        json!({"operation":"graph", "focus_path": 12}),
        json!({"operation":"scan", "after_path": false}),
        json!({"operation":"verify", "path":"visible.rs", "expected_sha256": null}),
    ] {
        assert!(
            tool.execute(malformed.clone()).await.is_err(),
            "invalid argument should be refused before reading: {malformed}"
        );
    }
}

#[tokio::test]
async fn misspelled_tool_fields_do_not_turn_narrow_requests_into_root_scans() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("visible.rs"), "fn visible() {}\n").unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    for typo in [
        json!({"op": "verify", "path": "visible.rs"}),
        json!({"operation": "scan", "paht": "visible.rs"}),
        json!({"operation": "scan", "query_text": "visible"}),
        json!({"operation": "scan", "limit": 1}),
        json!({"operation": "graph", "focus": "visible.rs"}),
    ] {
        assert!(
            tool.execute(typo.clone()).await.is_err(),
            "unknown key must not broaden source access: {typo}"
        );
    }
    let schema = tool.parameters_schema();
    assert_eq!(schema["additionalProperties"], false);
}

#[tokio::test]
async fn rejected_unknown_argument_keys_are_not_echoed_into_model_context() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let sensitive_key = format!("SECRET_ACCESS_TOKEN_{}", "x".repeat(5000));
    let mut args = json!({"operation":"scan"});
    args.as_object_mut()
        .unwrap()
        .insert(sensitive_key.clone(), Value::Null);
    let message = tool.execute(args).await.unwrap_err().to_string();
    assert!(!message.contains("SECRET_ACCESS_TOKEN"));
    assert!(!message.contains(&sensitive_key));
    assert!(
        message.len() < 256,
        "error must stay bounded and non-echoing"
    );
}

#[tokio::test]
async fn oversized_string_arguments_fail_before_attempting_any_path_resolution() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    for (field, request) in [
        ("path", json!({"operation":"scan","path":"x".repeat(4097)})),
        (
            "query",
            json!({
                "operation":"scan", "path":"missing-source.rs", "query":"a".repeat(129)
            }),
        ),
        (
            "after_path",
            json!({
                "operation":"scan", "path":"missing-source.rs", "after_path":"x".repeat(181)
            }),
        ),
        (
            "focus_path",
            json!({
                "operation":"graph", "path":"missing-source.rs", "focus_path":"x".repeat(181)
            }),
        ),
        (
            "expected_sha256",
            json!({
                "operation":"verify", "path":"missing-source.rs", "expected_sha256":"f".repeat(65)
            }),
        ),
    ] {
        let message = tool.execute(request).await.unwrap_err().to_string();
        assert!(
            message.contains(field),
            "expected early {field} validation, got: {message}"
        );
        assert!(
            !message.contains("No such file"),
            "filesystem was consulted before validation"
        );
    }
}
