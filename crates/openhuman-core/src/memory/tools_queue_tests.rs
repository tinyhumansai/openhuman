use super::*;
use crate::memory::test_fixtures::config_in;

#[tokio::test]
async fn learning_is_durably_queued_even_when_the_backend_refuses_writes() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    crate::memory::test_fixtures::RefusingEngine::out_of_credits().bind(&config);
    let tool = MemoryTool::new(Arc::new(config.clone()));
    let result = tool
        .execute(json!({"action":"learn","text":"Prefers tea"}))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.text());
    let value: Value = serde_json::from_str(&result.text()).unwrap();
    assert_eq!(value["status"], "queued; not yet saved to memory");
    assert_eq!(super::super::tool_writes::drain(&config).await, 0);
}
