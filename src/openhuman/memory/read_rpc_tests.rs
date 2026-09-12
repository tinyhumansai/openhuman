use super::*;
use tempfile::TempDir;

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    // Point config_path inside the tempdir so any persistence during
    // tests stays inside disposable workspace state.
    cfg.config_path = tmp.path().join("config.toml");
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    // Default llm is Cloud — but the cloud provider needs a bearer
    // token to actually fire. Tests that exercise the LLM path
    // override either the backend or the extractor. The read RPCs
    // below don't touch the LLM, so this default is fine.
    (tmp, cfg)
}

// ── tree-mode graph export (summaries + leaf chunks) ────────────────────

#[path = "read_rpc_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "read_rpc_tests_part_02_tests.rs"]
mod part_02_tests;
