use super::*;
use crate::openhuman::memory::api::types::{MemoryEntry, MemoryTaint};
use crate::openhuman::memory::MemoryCategory;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};

fn test_security() -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy::default())
}

/// A namespace nothing else writes to.
///
/// The tool holds no handle — it forgets through the *bound* driver, which
/// under a real module is the process-global test workspace every other
/// test shares. The legacy fallback below turns on `forget("", "<ns>/<k>")`
/// finding nothing under the split shape first, so a fixed namespace would
/// let a row another test left at `("global", "temp")` decide which branch
/// runs.
fn unique_namespace() -> String {
    format!(
        "forget{}",
        &uuid::Uuid::new_v4().as_simple().to_string()[..12]
    )
}

/// Seed a **legacy packed-key** row — namespace `""`, key `"<ns>/<key>"` —
/// through the guard, the same door the tool reads and deletes through.
///
/// A fixture store built here would be a different store entirely, so the
/// tool would find nothing and `forget_existing` would pass for the wrong
/// reason, while the two "blocked" tests' survival assertions would hold
/// whether or not the refusal worked.
async fn seed_legacy(namespace: &str, key: &str) {
    active_memory_guard()
        .await
        .expect("a bound memory guard")
        .store(
            "",
            &format!("{namespace}/{key}"),
            "temporary",
            MemoryCategory::Conversation,
            None,
            MemoryTaint::default(),
        )
        .await
        .expect("seed through the guard");
}

/// Read the legacy packed-key row back through the guard.
async fn legacy_row(namespace: &str, key: &str) -> Option<MemoryEntry> {
    active_memory_guard()
        .await
        .expect("a bound memory guard")
        .get("", &format!("{namespace}/{key}"))
        .await
        .expect("read back through the guard")
}

#[test]
fn name_and_schema() {
    let tool = MemoryForgetTool::new(test_security());
    assert_eq!(tool.name(), "memory_forget");
    assert!(tool.parameters_schema()["properties"]["key"].is_object());
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn forget_existing() {
    let namespace = unique_namespace();
    seed_legacy(&namespace, "temp").await;

    let tool = MemoryForgetTool::new(test_security());
    let result = tool
        .execute(json!({"namespace": namespace, "key": "temp"}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.output().contains("Forgot"));

    assert!(legacy_row(&namespace, "temp").await.is_none());
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn forget_nonexistent() {
    let namespace = unique_namespace();
    let tool = MemoryForgetTool::new(test_security());
    let result = tool
        .execute(json!({"namespace": namespace, "key": "nope"}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.output().contains("No memory found"));
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn forget_missing_key() {
    let tool = MemoryForgetTool::new(test_security());
    let result = tool.execute(json!({})).await;
    assert!(result.is_err());
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn forget_blocked_in_readonly_mode() {
    let namespace = unique_namespace();
    seed_legacy(&namespace, "temp").await;
    let readonly = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    });
    let tool = MemoryForgetTool::new(readonly);
    let result = tool
        .execute(json!({"namespace": namespace, "key": "temp"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("read-only mode"));
    assert!(legacy_row(&namespace, "temp").await.is_some());
}

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
the tool resolves the bound driver rather than being handed a memory handle"]
async fn forget_blocked_when_rate_limited() {
    let namespace = unique_namespace();
    seed_legacy(&namespace, "temp").await;
    let limited = Arc::new(SecurityPolicy {
        max_actions_per_hour: 0,
        ..SecurityPolicy::default()
    });
    let tool = MemoryForgetTool::new(limited);
    let result = tool
        .execute(json!({"namespace": namespace, "key": "temp"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Rate limit exceeded"));
    assert!(legacy_row(&namespace, "temp").await.is_some());
}
