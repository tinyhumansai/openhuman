use super::*;

#[tokio::test]
async fn graph_operation_reuses_scoped_read_policy_and_records_source_hashes() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("src")).unwrap();
    std::fs::write(project.path().join("src/main.ts"), "import './helper';\n").unwrap();
    std::fs::write(
        project.path().join("src/helper.ts"),
        "export const ok = true;\n",
    )
    .unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let graph = run(
        &tool,
        json!({"operation":"graph","focus_path":"src/helper.ts"}),
    )
    .await;
    assert_eq!(graph["operation"], "graph");
    assert_eq!(graph["scan_complete"], true);
    assert_eq!(graph["catalog_complete"], true);
    assert_eq!(graph["scope_is_workspace"], true);
    assert!(graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|node| node["source_hash"]
            .as_str()
            .is_some_and(|hash| hash.len() == 64)));
    assert_eq!(graph["edges"][0]["status"], "resolved");
    assert_eq!(graph["edges"][0]["to"], "src/helper.ts");
    assert_eq!(graph["impact"], json!(["src/main.ts"]));
    assert_eq!(graph["impact_exhaustive"], false);
    assert_eq!(graph["semantic_graph"], false);
}

#[tokio::test]
async fn graph_never_indexes_credential_files_or_linked_external_source() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("main.ts"), "import './linked';\n").unwrap();
    std::fs::write(project.path().join(".env"), "TOP_SECRET=forbidden").unwrap();
    std::fs::write(
        outside.path().join("linked.ts"),
        "export const secret = 123;\n",
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        outside.path().join("linked.ts"),
        project.path().join("linked.ts"),
    )
    .unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let graph = run(&tool, json!({"operation":"graph"})).await;
    assert_eq!(graph["edges"][0]["status"], "unresolved");
    assert!(!graph.to_string().contains("forbidden"));
    assert!(!graph.to_string().contains("linked.ts"));
}

#[tokio::test]
async fn graph_inherits_scan_limits_and_does_not_claim_complete_impact() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    for index in 0..5 {
        std::fs::write(
            project.path().join(format!("{index}.ts")),
            "export const value = 1;\n",
        )
        .unwrap();
    }
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let graph = run(&tool, json!({"operation":"graph","max_files":2})).await;
    assert_eq!(graph["scan_complete"], false);
    assert_eq!(graph["edges_complete"], false);
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(graph["impact_exhaustive"], false);
}

#[tokio::test]
async fn partial_catalog_must_not_confirm_unique_dependencies_or_impact() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("a.ts"), "import './b';\n").unwrap();
    std::fs::write(project.path().join("b.ts"), "export const b = 1;\n").unwrap();
    std::fs::write(project.path().join("c.ts"), "export const c = 1;\n").unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let result = run(
        &tool,
        json!({
            "operation":"graph","max_files":2,"focus_path":"b.ts"
        }),
    )
    .await;
    assert_eq!(result["scan_complete"], false);
    assert_eq!(result["edges"][0]["status"], "catalog_incomplete");
    assert_eq!(result["edges"][0]["observed_candidate"], "b.ts");
    assert!(result["edges"][0]["to"].is_null());
    assert_eq!(result["impact"], json!([]));
    assert_eq!(result["impact_omitted_reason"], "incomplete_catalog");
}

#[tokio::test]
async fn scoped_graph_reports_projection_scope_without_claiming_repository_coverage() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("pkg")).unwrap();
    std::fs::write(project.path().join("pkg/main.ts"), "import './helper';\n").unwrap();
    std::fs::write(
        project.path().join("pkg/helper.ts"),
        "export const local = 1;\n",
    )
    .unwrap();
    std::fs::write(
        project.path().join("outside.ts"),
        "export const external = 1;\n",
    )
    .unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let result = run(
        &tool,
        json!({"operation":"graph","path":"pkg","focus_path":"pkg/helper.ts"}),
    )
    .await;
    assert_eq!(result["scan_complete"], false);
    assert_eq!(result["catalog_complete"], false);
    assert_eq!(result["scope_path"], "pkg");
    assert_eq!(result["scope_is_workspace"], false);
    assert!(result["truncated_reason"].is_null());
    assert_eq!(result["edges"][0]["status"], "catalog_incomplete");
    assert_eq!(result["edges"][0]["observed_candidate"], "pkg/helper.ts");
    assert!(result["edges"][0]["to"].is_null());
    assert_eq!(result["edges_complete"], false);
    assert_eq!(result["impact"], json!([]));
    assert_eq!(result["impact_omitted_reason"], "incomplete_catalog");
    assert_eq!(result["impact_exhaustive"], false);
    assert_eq!(result["nodes"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn paginated_graph_never_marks_last_page_as_complete_repository_catalog() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("a.ts"), "import './z';\n").unwrap();
    std::fs::write(project.path().join("z.ts"), "export const z = 1;\n").unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let graph = run(
        &tool,
        json!({
            "operation":"graph","after_path":"a.ts","focus_path":"z.ts"
        }),
    )
    .await;
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(graph["catalog_complete"], false);
    assert_eq!(graph["impact"], json!([]));
    assert_eq!(graph["impact_omitted_reason"], "incomplete_catalog");
    assert_eq!(graph["scan_complete"], false);
    assert_eq!(graph["after_path"], "a.ts");
}

#[tokio::test]
async fn large_partial_graph_remains_valid_json_after_enrichment() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    for index in 0..32 {
        let stem = format!("{}_{index:02}", "a".repeat(120));
        let content = (0..32)
            .map(|other| format!("import './{}_{other:02}';\n", "a".repeat(120)))
            .collect::<String>();
        std::fs::write(project.path().join(format!("{stem}.ts")), content).unwrap();
    }
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let result = tool
        .execute(json!({"operation":"graph","max_files":16}))
        .await
        .unwrap();
    let raw = result.output();
    assert!(
        raw.len() <= 28_000,
        "graph exceeds safe JSON response budget: {}",
        raw.len()
    );
    let graph: Value = serde_json::from_str(&raw).expect("complete JSON graph");
    assert_eq!(graph["catalog_complete"], false);
    assert_eq!(graph["edges_complete"], false);
    assert_eq!(graph["impact"], json!([]));
}

#[tokio::test]
async fn graph_does_not_treat_unused_literal_query_as_truncated_file_evidence() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let imports = "import './b';\n".repeat(5);
    std::fs::write(project.path().join("a.ts"), imports).unwrap();
    std::fs::write(project.path().join("b.ts"), "export const b = 1;\n").unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let response = run(&tool, json!({"operation":"graph","query":"import"})).await;
    assert_eq!(
        response["scan_complete"], true,
        "graph operation must not invoke literal scan match truncation"
    );
    assert_eq!(response["catalog_complete"], true);
    assert_eq!(response["edges"][0]["status"], "resolved");
    assert_eq!(response["edges"][0]["to"], "b.ts");
}
