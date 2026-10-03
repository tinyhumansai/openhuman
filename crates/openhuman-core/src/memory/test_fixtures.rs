//! Shared fixtures for the memory unit tests: a config rooted in a temp dir,
//! and TinyMemory's in-memory reference engine bound to it.

use std::sync::Arc;

use tinymemory::conformance::ReferenceEngine;
use tinymemory::{ListRequest, MemoryEngine, MetaFilter};

use crate::config::Config;

/// A config whose workspace, action dir and credential store live in `tmp`.
/// The engine stays `tinyhumans` with no credential, so memory is off until a
/// test binds an engine.
pub(crate) fn config_in(tmp: &tempfile::TempDir) -> Config {
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

/// Binds a fresh reference engine to `config`'s workspace and returns it.
pub(crate) fn bind_reference(config: &Config) -> Arc<ReferenceEngine> {
    let engine = Arc::new(ReferenceEngine::new());
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());
    engine
}

/// Every item `engine` holds that matches `filter`.
pub(crate) async fn stored(engine: &ReferenceEngine, filter: MetaFilter) -> Vec<tinymemory::Hit> {
    engine
        .list(ListRequest {
            filter,
            limit: 100,
            cursor: None,
        })
        .await
        .expect("list")
        .items
}
