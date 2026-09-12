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
