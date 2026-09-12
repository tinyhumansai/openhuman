use super::*;
use serde_json::json;
use std::sync::Mutex;

/// Serialize env-var mutation across tests in this module so they
/// don't race each other under Rust's default parallel runner.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn lsp_name_and_schema() {
    let tool = LspTool::new();
    assert_eq!(tool.name(), "lsp");
    let schema = tool.parameters_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("kind")));
    assert!(required.contains(&json!("language")));
    assert!(required.contains(&json!("file")));
}

#[tokio::test]
async fn lsp_returns_not_implemented_error() {
    let tool = LspTool::new();
    let result = tool
        .execute(json!({
            "kind": "definition", "language": "rust", "file": "src/main.rs",
            "line": 0, "character": 0
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("not yet implemented"));
}

#[test]
fn lsp_capability_gate_off_by_default() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var(LSP_ENABLED_ENV).ok();
    std::env::remove_var(LSP_ENABLED_ENV);
    assert!(!lsp_capability_enabled());
    if let Some(v) = prev {
        std::env::set_var(LSP_ENABLED_ENV, v);
    }
}

#[test]
fn lsp_capability_gate_accepts_truthy_values() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var(LSP_ENABLED_ENV).ok();
    for v in ["1", "true", "TRUE", "yes", "on"] {
        std::env::set_var(LSP_ENABLED_ENV, v);
        assert!(lsp_capability_enabled(), "expected truthy for {v:?}");
    }
    for v in ["0", "false", "no", "off", ""] {
        std::env::set_var(LSP_ENABLED_ENV, v);
        assert!(!lsp_capability_enabled(), "expected falsy for {v:?}");
    }
    match prev {
        Some(v) => std::env::set_var(LSP_ENABLED_ENV, v),
        None => std::env::remove_var(LSP_ENABLED_ENV),
    }
}
