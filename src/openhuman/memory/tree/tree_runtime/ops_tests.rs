use super::*;
use tempfile::TempDir;

// `TreeNode`, `level_from_node_id` and `derive_parent_id` used to arrive
// through `super::*` while `ops.rs` still globbed the engine crate's runtime
// module. `ops.rs` names the contract explicitly now (#5560) and imports only
// the two items it uses, so these are named here — the same items, from the
// same crate the sibling `tree_runtime/mod.rs` re-exports them from.
use crate::openhuman::memory::api::tree::{derive_parent_id, level_from_node_id, TreeNode};

// The handlers under test resolve a `MemoryProvider` now, so these tests bind
// one. See `tree_runtime::test_support` for what it is and why it is backed by
// the real engine store rather than a fake.

fn rfc3339_z(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn config_in_tempdir() -> (TempDir, Config) {
    let tmp = TempDir::new().expect("tempdir");
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    (tmp, cfg)
}

fn test_node(
    namespace: &str,
    node_id: &str,
    summary: &str,
    created_at: DateTime<Utc>,
    child_count: u32,
) -> TreeNode {
    TreeNode {
        node_id: node_id.to_string(),
        namespace: namespace.to_string(),
        level: level_from_node_id(node_id),
        parent_id: derive_parent_id(node_id),
        summary: summary.to_string(),
        token_count: estimate_tokens(summary),
        child_count,
        created_at,
        updated_at: created_at,
        metadata: None,
    }
}

#[test]
fn create_provider_uses_local_model_when_local_ai_enabled() {
    // #002 FR-007: local path returns the user's local chat model.
    let mut cfg = Config::default();
    cfg.local_ai.runtime_enabled = true;
    cfg.local_ai.chat_model_id = "qwen2.5:7b".to_string();
    let (_provider, model) = create_provider(&cfg).expect("local provider should build");
    assert_eq!(model, "qwen2.5:7b");
}

#[test]
fn create_provider_errors_without_cloud_opt_in() {
    // By default, cloud summarization is off — memory summaries are
    // sensitive, so an explicit opt-in is required before routing them to
    // an external provider.
    let mut cfg = Config::default();
    cfg.local_ai.runtime_enabled = false;
    // cloud_summarization_opt_in defaults to false
    match create_provider(&cfg) {
        Err(e) => assert!(
            e.contains("no summarization provider"),
            "unexpected error: {e}"
        ),
        Ok(_) => panic!("expected error without cloud opt-in"),
    }
}

#[test]
fn create_provider_uses_cloud_when_opted_in_and_local_ai_off() {
    // #002 FR-007: with explicit opt-in Build Summary Trees uses the
    // configured cloud provider when local AI is disabled.
    let mut cfg = Config::default();
    cfg.local_ai.runtime_enabled = false;
    cfg.memory_tree.cloud_summarization_opt_in = true;
    let (_provider, model) =
        create_provider(&cfg).expect("cloud fallback should build when opted in");
    assert!(
        !model.trim().is_empty(),
        "cloud fallback must resolve a model"
    );
}
