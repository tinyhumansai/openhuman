use super::*;

#[tokio::test]
async fn rejects_unknown_mode_before_opening_external_search_resources() {
    let error = MemoryHybridSearchTool
        .execute(json!({
            "query": "release checklist",
            "namespace": "global",
            "mode": "mystery"
        }))
        .await
        .expect_err("an unknown mode must fail validation");

    let message = error.to_string();
    assert!(message.contains("unknown mode 'mystery'"), "{message}");
    // Validation runs before config, provider, and store setup. Reaching any
    // external search path would replace this precise validation error.
    assert!(!message.contains("load config failed"), "{message}");
}
