use super::MemoryTool;
use crate::memory::test_fixtures::{bind_reference, config_in, RefusingEngine};
use tinytools::Tool;

#[tokio::test]
async fn write_queue_filesystem_failure_is_model_visible_through_execute() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);
    std::fs::write(config.workspace_dir.join("memory"), b"not a directory").unwrap();

    let result = MemoryTool::new(std::sync::Arc::new(config))
        .execute(serde_json::json!({
            "action": "learn",
            "text": "a durable memory fact"
        }))
        .await
        .unwrap();

    assert!(result.is_error, "{}", result.text());
    assert!(
        result.text().contains("memory write queue unavailable"),
        "{}",
        result.text()
    );
    assert!(result.text().contains("UNAVAILABLE"), "{}", result.text());
}

#[tokio::test]
async fn learning_is_durably_queued_even_when_the_backend_refuses_writes() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    RefusingEngine::out_of_credits().bind(&config);
    let tool = MemoryTool::new(std::sync::Arc::new(config.clone()));
    let result = tool
        .execute(serde_json::json!({"action":"learn","text":"Prefers tea"}))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.text());
    let value: serde_json::Value = serde_json::from_str(&result.text()).unwrap();
    assert_eq!(value["status"], "queued; not yet saved to memory");
    assert_eq!(crate::memory::tool_writes::drain(&config).await, 0);
}
