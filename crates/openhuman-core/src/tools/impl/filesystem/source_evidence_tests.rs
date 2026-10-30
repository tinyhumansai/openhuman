use super::*;
use crate::security::policy::{TrustedAccess, TrustedRoot};
use crate::security::SecurityPolicy;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use tinytools::{Tool, ToolCallOptions, ToolRunContext, WorkspaceDescriptor};

fn policy(home: &Path, source: &Path) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        enabled: true,
        workspace_dir: home.to_path_buf(),
        action_dir: source.to_path_buf(),
        workspace_only: true,
        trusted_roots: vec![TrustedRoot {
            path: source.to_string_lossy().to_string(),
            access: TrustedAccess::Read,
        }],
        ..SecurityPolicy::default()
    })
}

async fn run(tool: &SourceEvidenceTool, args: Value) -> Value {
    let output = tool.execute(args).await.expect("source evidence execution");
    assert!(!output.is_error, "{}", output.output());
    serde_json::from_str(&output.output()).expect("JSON evidence envelope")
}

struct TestWorkspace(WorkspaceDescriptor);
impl ToolRunContext for TestWorkspace {
    fn workspace(&self) -> Option<&WorkspaceDescriptor> {
        Some(&self.0)
    }
}

#[tokio::test]
async fn scan_has_deterministic_generation_and_line_anchored_evidence() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("b.rs"), "fn beta() { alpha(); }\n").unwrap();
    std::fs::write(project.path().join("a.rs"), "fn alpha() {}\n").unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));

    let first = run(&tool, json!({"operation": "scan", "query": "alpha"})).await;
    let second = run(&tool, json!({"operation": "scan", "query": "alpha"})).await;
    assert_eq!(first["generation"], second["generation"]);
    assert_eq!(first["complete"], true);
    let files = first["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["path"], "a.rs");
    assert_eq!(files[1]["path"], "b.rs");
    assert_eq!(files[0]["matches"][0]["line"], 1);
    assert!(files[0]["source_hash"].as_str().unwrap().len() == 64);

    std::fs::write(project.path().join("a.rs"), "fn alpha_v2() {}\n").unwrap();
    let changed = run(&tool, json!({"operation": "scan", "query": "alpha"})).await;
    assert_ne!(first["generation"], changed["generation"]);
}

#[tokio::test]
async fn budgets_mark_results_partial_instead_of_claiming_complete() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    for index in 0..5 {
        std::fs::write(project.path().join(format!("{index}.rs")), "fn x() {}\n").unwrap();
    }
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let response = run(&tool, json!({"operation":"scan","max_files":2})).await;
    assert_eq!(response["complete"], false);
    assert_eq!(response["files"].as_array().unwrap().len(), 2);
    assert!(response["truncated_reason"].is_string());
}

#[tokio::test]
async fn scan_pagination_has_stable_order_and_explicit_continuation() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    for index in 0..5 {
        std::fs::write(
            project.path().join(format!("{index}.rs")),
            format!("fn f{index}() {{}}\n"),
        )
        .unwrap();
    }
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let first = run(&tool, json!({"operation":"scan","max_files":2})).await;
    assert_eq!(first["files"][0]["path"], "0.rs");
    assert_eq!(first["files"][1]["path"], "1.rs");
    assert_eq!(first["complete"], false);
    assert_eq!(first["next_after_path"], "1.rs");

    let second = run(
        &tool,
        json!({
            "operation":"scan","max_files":2,"after_path":first["next_after_path"]
        }),
    )
    .await;
    assert_eq!(second["files"][0]["path"], "2.rs");
    assert_eq!(second["files"][1]["path"], "3.rs");
    assert_eq!(second["next_after_path"], "3.rs");

    let third = run(
        &tool,
        json!({
            "operation":"scan","max_files":2,"after_path":second["next_after_path"]
        }),
    )
    .await;
    assert_eq!(third["files"].as_array().unwrap().len(), 1);
    assert_eq!(third["files"][0]["path"], "4.rs");
    assert_eq!(third["complete"], true);
    assert_eq!(third["next_after_path"], Value::Null);
}

#[tokio::test]
async fn non_utf8_source_is_not_silently_reported_as_complete() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("binary.rs"), [0xff, 0xfe, 0x00]).unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let result = run(&tool, json!({"operation":"scan"})).await;
    assert_eq!(result["complete"], false);
    assert_eq!(result["files"].as_array().unwrap().len(), 0);
    assert_eq!(result["truncated_reason"], "unreadable_or_unsupported_file");
}

#[tokio::test]
async fn oversized_source_is_reported_as_partial_without_allocating_file_contents() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("huge.rs"), vec![b'x'; 256 * 1024 + 1]).unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let result = run(&tool, json!({"operation":"scan"})).await;
    assert_eq!(result["complete"], false);
    assert_eq!(result["truncated_reason"], "oversize_file");
    assert_eq!(result["files"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn global_path_traversal_budget_bounds_sparse_directory_trees() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    for index in 0..12 {
        std::fs::create_dir(project.path().join(format!("sub{index}"))).unwrap();
    }
    std::fs::write(project.path().join("root.rs"), "fn root() {}").unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let result = run(&tool, json!({"operation":"scan","max_entries":3})).await;
    assert_eq!(result["complete"], false);
    assert_eq!(result["truncated_reason"], "max_entries");
    assert!(result["files"].as_array().unwrap().len() <= 2);
}

#[tokio::test]
async fn scan_serializes_large_escaped_matches_without_corrupting_json() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let line = format!("MATCH {}\n", "\u{0007}".repeat(155));
    for index in 0..20 {
        std::fs::write(
            project.path().join(format!("{index:02}.rs")),
            line.repeat(3),
        )
        .unwrap();
    }
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let output = tool
        .execute(json!({"operation":"scan","query":"MATCH"}))
        .await
        .unwrap();
    assert!(!output.is_error);
    let raw = output.output();
    assert!(raw.len() <= 26_000, "oversized JSON result: {}", raw.len());
    let result: Value = serde_json::from_str(&raw).expect("valid bounded JSON");
    assert_eq!(result["complete"], false);
    assert_eq!(result["truncated_reason"], "max_response_bytes");
}

#[tokio::test]
async fn byte_budget_partial_scan_supplies_resume_cursor() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let body = "a".repeat(255 * 1024);
    for i in 0..12 {
        std::fs::write(project.path().join(format!("{i:02}.rs")), &body).unwrap();
    }
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let first = run(&tool, json!({"operation":"scan"})).await;
    assert_eq!(first["complete"], false);
    assert_eq!(first["truncated_reason"], "max_total_bytes");
    let cursor = first["next_after_path"]
        .as_str()
        .expect("byte budget cursor");
    assert_eq!(cursor, "07.rs");
    let second = run(&tool, json!({"operation":"scan","after_path":cursor})).await;
    assert_eq!(second["files"][0]["path"], "08.rs");
}

#[tokio::test]
async fn source_pages_follow_global_lexical_order_even_with_directory_prefixes() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("a")).unwrap();
    std::fs::write(project.path().join("a/child.rs"), "fn nested() {}\n").unwrap();
    std::fs::write(project.path().join("a.rs"), "fn sibling() {}\n").unwrap();
    std::fs::write(project.path().join("b.rs"), "fn final_file() {}\n").unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let first = run(&tool, json!({"operation":"scan","max_files":1})).await;
    assert_eq!(first["files"][0]["path"], "a.rs");
    assert_eq!(first["next_after_path"], "a.rs");
    let second = run(
        &tool,
        json!({
            "operation":"scan","max_files":1,"after_path":"a.rs"
        }),
    )
    .await;
    assert_eq!(second["files"][0]["path"], "a/child.rs");
    let third = run(
        &tool,
        json!({
            "operation":"scan","max_files":1,"after_path":"a/child.rs"
        }),
    )
    .await;
    assert_eq!(third["files"][0]["path"], "b.rs");
    assert_eq!(third["complete"], true);
    assert_eq!(third["next_after_path"], Value::Null);
}

#[tokio::test]
async fn scanned_long_line_excerpt_contains_the_search_hit() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("long.rs"),
        format!("{}needle\n", "x".repeat(240)),
    )
    .unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let result = run(&tool, json!({"operation":"scan","query":"needle"})).await;
    let hit = &result["files"][0]["matches"][0];
    assert!(hit["text"].as_str().unwrap().contains("needle"));
    assert!(hit["column_start"].as_u64().unwrap() > 1);
    assert_eq!(hit["excerpt_truncated"], true);
}

#[tokio::test]
async fn scanned_unicode_line_preserves_utf8_boundaries_in_cropped_excerpt() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("unicode.rs"),
        format!("{}needle🧠\n", "🙂".repeat(200)),
    )
    .unwrap();
    let tool = SourceEvidenceTool::new(policy(home.path(), project.path()));
    let result = run(&tool, json!({"operation":"scan","query":"needle🧠"})).await;
    let hit = &result["files"][0]["matches"][0];
    assert!(hit["text"].as_str().unwrap().contains("needle🧠"));
    assert!(hit["text"].as_str().unwrap().chars().count() <= 160);
    assert!(hit["column_start"].as_u64().unwrap() > 1);
}

#[path = "source_evidence_boundary_tests.rs"]
mod boundary;

#[path = "source_evidence_graph_tests.rs"]
mod graph;

#[path = "source_evidence_verification_tests.rs"]
mod verification;

#[path = "source_evidence_arguments_tests.rs"]
mod arguments;
