use super::*;
use serde_json::json;

#[tokio::test]
async fn missing_query_errors() {
    let err = run_fast_walk(json!({})).await.unwrap_err();
    assert!(err.to_string().contains("`query` is required"));
}

#[tokio::test]
async fn blank_query_errors() {
    let err = run_fast_walk(json!({"query": "   "})).await.unwrap_err();
    assert!(err.to_string().contains("`query` is required"));
}
