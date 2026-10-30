use super::*;

#[tokio::test]
async fn verify_rejects_drift_and_never_returns_stale_lines() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let path = project.path().join("lib.rs");
    std::fs::write(&path, "line one\nline two\nline three\n").unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let scan = run(&tool, json!({"operation": "scan"})).await;
    let digest = scan["files"][0]["source_hash"].as_str().unwrap();
    let verified = run(
        &tool,
        json!({
            "operation": "verify", "path": "lib.rs",
            "expected_sha256": digest, "start_line": 2, "end_line": 3
        }),
    )
    .await;
    assert_eq!(verified["verified"], true);
    assert_eq!(verified["lines"][0]["line"], 2);
    assert_eq!(verified["lines"][0]["text"], "line two");

    std::fs::write(&path, "edited behind your back\n").unwrap();
    let err = tool
        .execute(json!({
            "operation": "verify", "path": "lib.rs", "expected_sha256": digest
        }))
        .await
        .expect_err("stale proof must be refused");
    assert!(err.to_string().contains("changed"));
}

#[tokio::test]
async fn verify_requires_a_full_digest_and_bounded_line_range() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("test.rs"), "one\n").unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    assert!(tool
        .execute(json!({"operation":"verify","path":"test.rs"}))
        .await
        .is_err());
    assert!(tool
        .execute(json!({
            "operation":"verify","path":"test.rs","expected_sha256":"nonsense"
        }))
        .await
        .is_err());
    assert!(tool
        .execute(json!({
            "operation":"verify","path":"test.rs","expected_sha256":"0".repeat(64),
            "start_line": 80, "end_line": 20
        }))
        .await
        .is_err());
}

#[tokio::test]
async fn verify_serializes_long_escaped_lines_with_resume_cursor() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let line = format!("MATCH {}\n", "\u{0007}".repeat(155));
    std::fs::write(project.path().join("file.rs"), line.repeat(80)).unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let scan = run(&tool, json!({"operation":"scan"})).await;
    let sha = scan["files"][0]["source_hash"].as_str().unwrap();
    let output = run(
        &tool,
        json!({
            "operation":"verify","path":"file.rs","expected_sha256":sha,
            "start_line":1,"end_line":80
        }),
    )
    .await;
    assert!(output.to_string().len() <= 26_000);
    assert_eq!(output["verified"], true);
    assert_eq!(output["lines_complete"], false);
    let last = output["lines"].as_array().unwrap().last().unwrap()["line"]
        .as_u64()
        .unwrap();
    assert_eq!(output["next_line"], last + 1);
}

#[tokio::test]
async fn locate_ranks_hash_anchored_source_lines_without_a_remote_model() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("src")).unwrap();
    std::fs::write(
        project.path().join("src/session_manager.ts"),
        "export function assertSessionFresh(s: Session) { return s.expiresAt > Date.now(); }\n",
    )
    .unwrap();
    std::fs::write(
        project.path().join("src/billing.ts"),
        "export function chargeInvoice() { return 1; }\n",
    )
    .unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let result = run(
        &tool,
        json!({"operation":"locate","query":"session freshness","path":"."}),
    )
    .await;
    assert_eq!(result["operation"], "locate");
    assert_eq!(result["results"][0]["path"], "src/session_manager.ts");
    assert_eq!(result["results"][0]["evidence"][0]["line"], 1);
    assert_eq!(
        result["results"][0]["source_hash"].as_str().unwrap().len(),
        64
    );
    assert_eq!(result["ranking_kind"], "local_lexical_heuristic");
    assert_eq!(result["source_of_truth_verified"], false);
    assert_eq!(result["catalog_complete"], true);
}

#[tokio::test]
async fn locate_is_explicitly_partial_when_file_budget_or_cursor_restricts_catalog() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    for index in 0..5 {
        std::fs::write(
            project.path().join(format!("{index}.rs")),
            "fn authentication_check() {}\n",
        )
        .unwrap();
    }
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let result = run(
        &tool,
        json!({
            "operation":"locate","query":"authentication check","max_files":2
        }),
    )
    .await;
    assert_eq!(result["catalog_complete"], false);
    assert_eq!(result["ranking_exhaustive"], false);
    assert_eq!(result["truncated_reason"], "max_files");
    assert!(!result["results"].as_array().unwrap().is_empty());

    let page = run(
        &tool,
        json!({
            "operation":"locate","query":"authentication check","after_path":"1.rs"
        }),
    )
    .await;
    assert_eq!(page["catalog_complete"], false);
    assert_eq!(page["ranking_exhaustive"], false);
}

#[tokio::test]
async fn locate_refuses_blank_queries_and_never_indexes_hidden_credentials() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("main.rs"),
        "fn authentication_check() {}\n",
    )
    .unwrap();
    std::fs::write(
        project.path().join(".env"),
        "authentication_check_secret = do_not_read",
    )
    .unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    assert!(tool
        .execute(json!({"operation":"locate","query":"  "}))
        .await
        .is_err());
    let result = run(
        &tool,
        json!({"operation":"locate","query":"authentication check"}),
    )
    .await;
    assert!(result.to_string().find(".env").is_none());
    assert_eq!(result["results"][0]["path"], "main.rs");
}

#[tokio::test]
async fn verification_can_reopen_the_exact_match_window_from_a_locate_result() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("long.rs"),
        format!("{}authSessionFresh = true;\n", "🙂".repeat(220)),
    )
    .unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let located = run(
        &tool,
        json!({
            "operation":"locate","query":"auth session fresh"
        }),
    )
    .await;
    let evidence = &located["results"][0]["evidence"][0];
    let expected = located["results"][0]["source_hash"].as_str().unwrap();
    let result = run(
        &tool,
        json!({
            "operation":"verify", "path":"long.rs", "expected_sha256":expected,
            "start_line":1, "end_line":1,
            "start_column":evidence["column_start"]
        }),
    )
    .await;
    assert_eq!(result["verified"], true);
    assert_eq!(result["lines"][0]["column_start"], evidence["column_start"]);
    assert_eq!(result["lines"][0]["text"], evidence["text"]);
    assert!(result["lines"][0]["text"]
        .as_str()
        .unwrap()
        .contains("authSessionFresh"));
}

#[tokio::test]
async fn verification_column_cursor_has_strict_bounds_and_defaults_to_first_column() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("chars.rs"), "🙂abc\n").unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let catalog = run(&tool, json!({"operation":"scan"})).await;
    let hash = catalog["files"][0]["source_hash"].as_str().unwrap();
    let default = run(
        &tool,
        json!({
            "operation":"verify", "path":"chars.rs", "expected_sha256":hash,
            "start_line":1, "end_line":1
        }),
    )
    .await;
    assert_eq!(default["lines"][0]["text"], "🙂abc");
    assert_eq!(default["lines"][0]["column_start"], 1);
    let middle = run(
        &tool,
        json!({
            "operation":"verify", "path":"chars.rs", "expected_sha256":hash,
            "start_column":2, "start_line":1, "end_line":1
        }),
    )
    .await;
    assert_eq!(middle["lines"][0]["text"], "abc");
    for column in [0_u64, 256 * 1024 + 2] {
        assert!(
            tool.execute(json!({
                "operation":"verify", "path":"chars.rs", "expected_sha256":hash,
                "start_column":column
            }))
            .await
            .is_err(),
            "invalid one-based cursor {column}"
        );
    }
}

#[cfg(unix)]
#[test]
fn file_descriptor_identity_detects_atomic_source_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("source.rs");
    let replacement = dir.path().join("replacement.rs");
    std::fs::write(&original, "fn original() {}\n").unwrap();
    std::fs::write(&replacement, "fn substituted() {}\n").unwrap();
    let before = std::fs::metadata(&original).unwrap();
    let same = std::fs::metadata(&original).unwrap();
    assert!(same_file_identity(&before, &same));
    let different = std::fs::metadata(&replacement).unwrap();
    assert!(!same_file_identity(&before, &different));
    std::fs::rename(&replacement, &original).unwrap();
    let after = std::fs::metadata(&original).unwrap();
    assert!(
        !same_file_identity(&before, &after),
        "atomically replacing a validated file must invalidate its provenance"
    );
}
