use super::*;
use tempfile::TempDir;

fn test_config() -> (TempDir, Arc<Config>) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    (tmp, Arc::new(cfg))
}

#[test]
fn name_and_schema() {
    let (_tmp, cfg) = test_config();
    let tool = MemoryDoctorTool::new(cfg);
    assert_eq!(tool.name(), "memory_doctor");
    // No required args.
    assert_eq!(tool.parameters_schema()["required"], json!([]));
}

#[tokio::test]
async fn execute_returns_a_report_for_a_misconfigured_workspace() {
    let _g = tinymemory_core::tree::health::test_guard();
    let (_tmp, cfg) = test_config();
    // No embeddings provider, local AI off → unhealthy with a typed cause.
    let tool = MemoryDoctorTool::new(cfg);
    let result = tool.execute(json!({})).await.unwrap();
    assert!(!result.is_error);
    let out = result.output();
    assert!(
        out.contains("\"healthy\""),
        "report should serialize: {out}"
    );
    assert!(
        out.contains("embeddings_unconfigured") || out.contains("\"healthy\": false"),
        "misconfigured workspace should surface a blocking cause: {out}"
    );
}
